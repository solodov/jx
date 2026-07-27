use super::*;
use crate::repository::StackMetadataWorkItemHandlerRun;

const PULL_REQUEST_HANDLER_LOG_FILE: &str = "jx-pull-request-handlers.log";
const PULL_REQUEST_HANDLER_ACTION: &str = "pull_request_handler";
const WORK_ITEM_HANDLER_LOG_FILE: &str = "jx-work-item-handlers.log";

/// Command-side integration boundary for pull-request stack state.
pub(super) struct PullRequestStackManager<'a> {
    context: &'a RepositoryContext,
    services: &'a dyn CommandServices,
    perf: PerfLog,
    status_metadata_maintainer: StackStatusMetadataMaintainer<'a>,
}

/// PRs whose generated stack context was refreshed after stack synchronization.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PullRequestStackPublishUpdate {
    pub(super) pull_requests: Vec<PullRequestRecord>,
}

impl PullRequestStackPublishUpdate {
    pub(super) fn is_empty(&self) -> bool {
        self.pull_requests.is_empty()
    }
}

/// Local stack branches selected for sync with the maintained metadata used to render them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PullRequestStackSyncSelection {
    pub(super) branches: Vec<String>,
    pub(super) metadata: StackMetadata,
}

impl<'a> PullRequestStackManager<'a> {
    pub(super) fn new(
        context: &'a RepositoryContext,
        services: &'a dyn CommandServices,
        perf: PerfLog,
        environment: &'a RuntimeEnvironment,
    ) -> Self {
        Self {
            context,
            services,
            perf,
            status_metadata_maintainer: StackStatusMetadataMaintainer::new(context, environment),
        }
    }

    /// Loads the locally stored stack snapshot without requiring GitHub access.
    pub(super) fn stored_snapshot(
        &self,
        selection: PullRequestStackSelection,
    ) -> Result<PullRequestStackSnapshot, CommandError> {
        self.snapshot_from_live_pull_requests(Vec::new(), selection)
    }

    /// Applies GitHub status facts to durable metadata and prunes completed cached trees.
    pub(super) fn maintain_status_metadata(
        &self,
        statuses: &[PullRequestStatusRecord],
    ) -> Result<PullRequestStackSnapshot, CommandError> {
        let metadata = self.read_metadata()?;
        let maintenance = self
            .status_metadata_maintainer
            .maintain(&metadata, statuses)?;
        let maintained = maintenance.metadata;
        let local_branches = self.local_pull_request_branches()?;
        Ok(PullRequestStackSnapshot::from_metadata(
            &maintained,
            &local_branches,
            &[],
            PullRequestStackSelection::default(),
        ))
    }

    /// Refreshes stack state and updates GitHub PR descriptions/bases from that state.
    pub(super) fn refresh_and_sync_authored_open_pull_requests(
        &self,
    ) -> Result<PullRequestStackSnapshot, CommandError> {
        let refresh = self.refresh_authored_open_stack_metadata()?;
        let snapshot = PullRequestStackSnapshot::from_metadata(
            &refresh.metadata,
            &refresh.local_branches,
            &refresh.live_pull_requests,
            PullRequestStackSelection::default(),
        );
        let _ = self.sync_pull_requests_for_snapshot(
            &snapshot,
            &refresh.live_pull_requests,
            &refresh.metadata,
        )?;
        Ok(snapshot)
    }

    /// Builds the full cached stack for interactive opening without refreshing GitHub state.
    pub(super) fn cached_open_snapshot(&self) -> Result<PullRequestStackSnapshot, CommandError> {
        let metadata = self.read_metadata()?;
        if metadata.nodes.is_empty() {
            return Err(missing_local_bookmark_pull_requests(self.context).into());
        }

        let selection = self
            .services
            .pull_request_candidate_bookmarks(self.context, None)?
            .first()
            .map(|branch| PullRequestStackSelection::branch(branch.clone()))
            .unwrap_or_default();
        let snapshot = PullRequestStackSnapshot::from_metadata(&metadata, &[], &[], selection);
        if stack_snapshot_has_openable_pull_request(&snapshot) {
            return Ok(snapshot);
        }

        Err(missing_local_bookmark_pull_requests(self.context).into())
    }

    /// Refreshes durable stack metadata from local jj branch ancestry.
    pub(super) fn refresh_local_stack_metadata(&self) -> Result<StackMetadata, CommandError> {
        let mut span = self.perf.start(
            "stack.refresh_local_metadata",
            [perf_attr("repo", self.context.origin.github.slug())],
        );
        let result = self.refresh_local_stack_metadata_traced(&mut span);
        if let Ok(metadata) = &result {
            span.set([perf_attr("metadata_node_count", metadata.nodes.len())]);
        }
        if let Err(error) = &result {
            span.record_error(error);
        }
        span.end();
        result
    }

    fn refresh_local_stack_metadata_traced(
        &self,
        span: &mut PerfSpan,
    ) -> Result<StackMetadata, CommandError> {
        let metadata = span.measure_with_result_attrs(
            "read_metadata",
            Vec::new(),
            || self.read_metadata(),
            metadata_result_attrs,
        )?;
        let metadata = self.apply_local_stack_metadata_traced(
            metadata,
            span,
            "refresh_local_stack_metadata.apply_local_stack_metadata",
        )?;
        span.measure(
            "write_metadata",
            [perf_attr("metadata_node_count", metadata.nodes.len())],
            || self.write_metadata(&metadata),
        )?;
        Ok(metadata)
    }

    /// Selects local stack component branches for sync after applying stack maintenance.
    pub(super) fn sync_selection_for_selector(
        &self,
        selector: Option<&str>,
    ) -> Result<PullRequestStackSyncSelection, CommandError> {
        let selected_branches = self
            .services
            .pull_request_candidate_bookmarks(self.context, selector)?;
        self.sync_selection_for_branches(&selected_branches)
    }

    /// Selects local stack component branches from already-resolved PR branches.
    pub(super) fn sync_selection_for_branches(
        &self,
        selected_branches: &[String],
    ) -> Result<PullRequestStackSyncSelection, CommandError> {
        let metadata = self.sync_metadata()?;
        let local_branches = self.local_pull_request_branches()?;
        let syncable_metadata = prune_merged_stack_metadata_trees(&metadata);
        let branches = PullRequestStackSnapshot::from_metadata(
            &syncable_metadata,
            &local_branches,
            &[],
            PullRequestStackSelection::default(),
        )
        .local_component_branches_for(selected_branches);

        Ok(PullRequestStackSyncSelection { branches, metadata })
    }

    /// Upserts stack PRs and refreshes generated context for PRs whose full update was skipped.
    pub(super) fn update_after_stack_publish_with_context_only(
        &self,
        reports: &[PullRequestReport],
        context_only_pull_requests: &[PullRequestRecord],
    ) -> Result<PullRequestStackPublishUpdate, CommandError> {
        let mut span = self.perf.start(
            "stack.update_after_publish",
            [
                perf_attr("repo", self.context.origin.github.slug()),
                perf_attr("report_count", reports.len()),
                perf_attr("context_only_pr_count", context_only_pull_requests.len()),
            ],
        );
        let result =
            self.update_after_stack_publish_traced(reports, context_only_pull_requests, &mut span);
        if let Ok(update) = &result {
            span.set([perf_attr("updated_pr_count", update.pull_requests.len())]);
        }
        if let Err(error) = &result {
            span.record_error(error);
        }
        span.end();
        result
    }

    pub(super) fn record_published_work_items(
        &self,
        reports: &[PullRequestReport],
        fixes: &[String],
        fixes_attached: bool,
        intent_branches: &BTreeSet<String>,
    ) -> Result<(), CommandError> {
        if reports.is_empty() {
            return Ok(());
        }

        let mut seed_pull_requests = Vec::new();
        let mut seen = BTreeSet::new();
        for report in reports {
            if let Some(base_pull_request) = &report.base_pull_request {
                if seen.insert(base_pull_request.number) {
                    seed_pull_requests.push(base_pull_request.clone());
                }
            }
            if seen.insert(report.pull_request.number) {
                seed_pull_requests.push(report.pull_request.clone());
            }
        }

        let original = self.read_metadata()?;
        let mut metadata = upsert_stack_metadata_pull_requests(&seed_pull_requests, &original);
        for report in reports {
            let Some(node) = metadata.nodes.iter_mut().find(|node| {
                node.pull_request == Some(report.pull_request.number)
                    || node.branch == report.pull_request.head_branch
            }) else {
                continue;
            };
            let is_intent = intent_branches.contains(&report.pull_request.head_branch);
            let report_fixes: &[String] = if is_intent { fixes } else { &[] };
            let mut work_ids = domain::pull_request_work_ids(
                &report.pull_request.title,
                report.task_id.as_deref(),
                report_fixes,
            );
            let mut fixed_work_ids = report_fixes.to_vec();
            if is_intent && fixes_attached {
                merge_work_ids(&mut fixed_work_ids, &work_ids);
            }
            merge_work_ids(&mut work_ids, &fixed_work_ids);
            merge_work_ids(&mut work_ids, &node.fixes_work_ids);
            if !work_ids.is_empty() {
                node.work_ids = work_ids;
            }
            if !fixed_work_ids.is_empty() {
                merge_work_ids(&mut node.fixes_work_ids, &fixed_work_ids);
            }
        }

        if metadata != original {
            self.write_metadata(&metadata)?;
        }
        Ok(())
    }

    fn update_after_stack_publish_traced(
        &self,
        reports: &[PullRequestReport],
        context_only_pull_requests: &[PullRequestRecord],
        span: &mut PerfSpan,
    ) -> Result<PullRequestStackPublishUpdate, CommandError> {
        let selection = reports
            .first()
            .map(|report| PullRequestStackSelection::pull_request(report.pull_request.number))
            .or_else(|| {
                context_only_pull_requests.first().map(|pull_request| {
                    PullRequestStackSelection::pull_request(pull_request.number)
                })
            });
        let Some(selection) = selection else {
            return Ok(PullRequestStackPublishUpdate::default());
        };

        let mut seed_pull_requests = Vec::new();
        let mut seen = BTreeSet::new();
        for report in reports {
            if let Some(base_pull_request) = &report.base_pull_request {
                if seen.insert(base_pull_request.number) {
                    seed_pull_requests.push(base_pull_request.clone());
                }
            }
            if seen.insert(report.pull_request.number) {
                seed_pull_requests.push(report.pull_request.clone());
            }
        }
        for pull_request in context_only_pull_requests {
            if seen.insert(pull_request.number) {
                seed_pull_requests.push(pull_request.clone());
            }
        }
        let context_only_branches = context_only_pull_requests
            .iter()
            .map(|pull_request| pull_request.head_branch.clone())
            .collect::<BTreeSet<_>>();
        span.set([
            perf_attr("seed_pr_count", seed_pull_requests.len()),
            perf_attr("context_only_branch_count", context_only_branches.len()),
        ]);

        self.sync_stack_component(
            selection,
            &seed_pull_requests,
            !reports.is_empty(),
            &context_only_branches,
            span,
        )
    }

    /// Syncs PR descriptions using the currently stored stack metadata.
    pub(super) fn sync_pull_requests(
        &self,
        push: &TrackedPushOutcome,
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        let metadata = self.sync_metadata()?;
        if push.bookmarks.is_empty() {
            return Ok(Vec::new());
        }
        self.sync_pull_requests_with_metadata(push, &metadata)
    }

    fn sync_stack_component(
        &self,
        selection: PullRequestStackSelection,
        seed_pull_requests: &[PullRequestRecord],
        full_component_sync: bool,
        context_only_branches: &BTreeSet<String>,
        span: &mut PerfSpan,
    ) -> Result<PullRequestStackPublishUpdate, CommandError> {
        let local_branch_facts = span.measure_with_result_attrs(
            "load_local_stack_branches",
            [perf_attr("seed_pr_count", seed_pull_requests.len())],
            || self.services.local_stack_branch_facts(self.context),
            local_stack_branch_facts_result_attrs,
        )?;
        record_local_stack_branch_metrics(span, &local_branch_facts.metrics);
        let local_branches = local_branch_facts.branches;
        span.set([perf_attr("local_branch_count", local_branches.len())]);

        let stack_metadata_step = span.start_step(
            "stack_metadata_with_local_and_seed_prs",
            [perf_attr("seed_pr_count", seed_pull_requests.len())],
        );
        let metadata_result = self.stack_metadata_with_local_and_seed_pull_requests(
            seed_pull_requests,
            &local_branches,
            span,
        );
        span.finish_step(
            stack_metadata_step,
            metadata_result_attrs(&metadata_result),
            metadata_result.as_ref().err(),
        );
        let metadata = metadata_result?;
        let snapshot = PullRequestStackSnapshot::from_metadata(
            &metadata,
            &[],
            seed_pull_requests,
            selection.clone(),
        );
        let component = snapshot.component_for_selection(selection.clone());
        span.set([perf_attr("component_node_count", component.nodes.len())]);
        let component_pull_requests = span.measure(
            "pull_requests_for_component",
            [
                perf_attr("component_node_count", component.nodes.len()),
                perf_attr("seed_pr_count", seed_pull_requests.len()),
            ],
            || self.pull_requests_for_component(&component, seed_pull_requests),
        )?;
        span.set([perf_attr(
            "component_pr_count",
            component_pull_requests.len(),
        )]);
        if component_pull_requests.is_empty() {
            return Ok(PullRequestStackPublishUpdate::default());
        }

        let metadata = refresh_stack_metadata_pull_requests(&component_pull_requests, &metadata);
        let metadata = self.apply_local_stack_metadata_snapshot_traced(
            metadata,
            &local_branches,
            span,
            "component_metadata.apply_local_stack_metadata",
        )?;
        span.measure(
            "write_metadata",
            [perf_attr("metadata_node_count", metadata.nodes.len())],
            || self.write_metadata(&metadata),
        )?;

        let refreshed_snapshot = PullRequestStackSnapshot::from_metadata(
            &metadata,
            &[],
            &component_pull_requests,
            selection.clone(),
        );
        let refreshed_component = refreshed_snapshot.component_for_selection(selection);
        span.measure(
            "sync_available_pull_requests",
            [
                perf_attr("component_pr_count", component_pull_requests.len()),
                perf_attr("full_component_sync", full_component_sync),
                perf_attr("context_only_branch_count", context_only_branches.len()),
            ],
            || {
                self.sync_available_pull_requests_for_snapshot(
                    &refreshed_component,
                    &component_pull_requests,
                    &metadata,
                    full_component_sync,
                    context_only_branches,
                )
            },
        )
    }

    fn stack_metadata_with_local_and_seed_pull_requests(
        &self,
        seed_pull_requests: &[PullRequestRecord],
        local_branches: &[LocalStackBranch],
        span: &mut PerfSpan,
    ) -> Result<StackMetadata, CommandError> {
        let metadata = span.measure_with_result_attrs(
            "stack_metadata.read_metadata",
            Vec::new(),
            || self.read_metadata(),
            metadata_result_attrs,
        )?;
        let metadata = self.apply_local_stack_metadata_snapshot_traced(
            metadata,
            local_branches,
            span,
            "stack_metadata.apply_existing_local",
        )?;
        let metadata = span.measure_with_result_attrs(
            "stack_metadata.upsert_seed_prs",
            [perf_attr("seed_pr_count", seed_pull_requests.len())],
            || {
                Ok::<_, CommandError>(upsert_stack_metadata_pull_requests(
                    seed_pull_requests,
                    &metadata,
                ))
            },
            metadata_result_attrs,
        )?;
        let metadata = self.apply_local_stack_metadata_snapshot_traced(
            metadata,
            local_branches,
            span,
            "stack_metadata.apply_seeded_local",
        )?;
        span.measure(
            "stack_metadata.write_metadata",
            [perf_attr("metadata_node_count", metadata.nodes.len())],
            || self.write_metadata(&metadata),
        )?;
        Ok(metadata)
    }

    fn snapshot_from_live_pull_requests(
        &self,
        live_pull_requests: Vec<PullRequestRecord>,
        selection: PullRequestStackSelection,
    ) -> Result<PullRequestStackSnapshot, CommandError> {
        let metadata = self.read_metadata()?;
        let local_branches = self.local_pull_request_branches()?;
        Ok(PullRequestStackSnapshot::from_metadata(
            &metadata,
            &local_branches,
            &live_pull_requests,
            selection,
        ))
    }

    fn refresh_authored_open_stack_metadata(
        &self,
    ) -> Result<AuthoredStackMetadataRefresh, CommandError> {
        let local_branches = self.local_pull_request_branches()?;
        let metadata = self.refresh_metadata_by_number(self.read_metadata()?)?;
        let live_pull_requests = self.authored_open_pull_requests_for_branches(&local_branches)?;
        if local_branches.is_empty() && live_pull_requests.is_empty() {
            let metadata = StackMetadata::default();
            self.write_metadata(&metadata)?;
            return Ok(AuthoredStackMetadataRefresh {
                metadata,
                local_branches,
                live_pull_requests,
            });
        }

        let metadata = stack_metadata_from_pull_requests(&live_pull_requests, &metadata);
        let metadata = self.apply_local_stack_metadata(metadata)?;
        self.write_metadata(&metadata)?;

        Ok(AuthoredStackMetadataRefresh {
            metadata,
            local_branches,
            live_pull_requests,
        })
    }

    fn authored_open_pull_requests_for_branches(
        &self,
        branches: &[String],
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        let author = self
            .services
            .authenticated_login(&self.context.token_source)?;
        let mut pull_requests = self
            .services
            .authored_open_pull_requests(self.context, &author)?;
        let mut seen_numbers = pull_requests
            .iter()
            .map(|pull_request| pull_request.number)
            .collect::<BTreeSet<_>>();
        for branch in branches {
            let Some(pull_request) = self.services.find_authored_open_pull_request_for_head(
                self.context,
                branch,
                &author,
            )?
            else {
                continue;
            };
            if seen_numbers.insert(pull_request.number) {
                pull_requests.push(pull_request);
            }
        }
        Ok(pull_requests)
    }

    fn local_pull_request_branches(&self) -> Result<Vec<String>, CommandError> {
        self.services
            .pull_request_bookmarks(self.context)
            .map_err(Into::into)
    }

    fn pull_requests_for_component(
        &self,
        component: &PullRequestStackSnapshot,
        seed_pull_requests: &[PullRequestRecord],
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        let mut pull_requests_by_branch = seed_pull_requests
            .iter()
            .map(|pull_request| (pull_request.head_branch.clone(), pull_request.clone()))
            .collect::<BTreeMap<_, _>>();

        for node in &component.nodes {
            if pull_requests_by_branch.contains_key(&node.branch) {
                continue;
            }
            let pull_request = match self
                .services
                .find_pull_request_for_head(self.context, &node.branch)?
            {
                Some(pull_request) => Some(pull_request),
                None => node
                    .pull_request_number()
                    .map(|number| {
                        self.services
                            .find_pull_request_by_number(self.context, number)
                    })
                    .transpose()?
                    .flatten(),
            };
            let Some(pull_request) = pull_request else {
                continue;
            };
            pull_requests_by_branch.insert(node.branch.clone(), pull_request);
        }

        Ok(component
            .nodes
            .iter()
            .filter_map(|node| pull_requests_by_branch.get(&node.branch).cloned())
            .collect())
    }

    pub(super) fn sync_pull_requests_with_metadata(
        &self,
        push: &TrackedPushOutcome,
        metadata: &StackMetadata,
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        let mut span = self.perf.start(
            "stack.sync_pull_requests",
            [
                perf_attr("repo", self.context.origin.github.slug()),
                perf_attr("bookmark_count", push.bookmarks.len()),
                perf_attr("metadata_node_count", metadata.nodes.len()),
            ],
        );
        let result = span
            .measure("sync_pull_requests", Vec::new(), || {
                self.services
                    .sync_pull_requests(self.context, push, metadata)
            })
            .map_err(CommandError::from);
        if let Ok(pull_requests) = &result {
            span.set([perf_attr("synced_pr_count", pull_requests.len())]);
        }
        if let Err(error) = &result {
            span.record_error(error);
        }
        span.end();
        result
    }

    fn sync_pull_requests_for_snapshot(
        &self,
        snapshot: &PullRequestStackSnapshot,
        seed_pull_requests: &[PullRequestRecord],
        metadata: &StackMetadata,
    ) -> Result<PullRequestStackPublishUpdate, CommandError> {
        let pull_requests = self.pull_requests_for_component(snapshot, seed_pull_requests)?;
        self.sync_available_pull_requests_for_snapshot(
            snapshot,
            &pull_requests,
            metadata,
            true,
            &BTreeSet::new(),
        )
    }

    fn sync_available_pull_requests_for_snapshot(
        &self,
        snapshot: &PullRequestStackSnapshot,
        pull_requests: &[PullRequestRecord],
        metadata: &StackMetadata,
        full_component_sync: bool,
        context_only_branches: &BTreeSet<String>,
    ) -> Result<PullRequestStackPublishUpdate, CommandError> {
        if pull_requests.is_empty() {
            return Ok(PullRequestStackPublishUpdate::default());
        }

        let mut synced_pull_requests = Vec::new();
        if full_component_sync {
            let full_branches = pull_requests
                .iter()
                .map(|pull_request| pull_request.head_branch.clone())
                .filter(|branch| !context_only_branches.contains(branch))
                .collect::<BTreeSet<_>>();
            if !full_branches.is_empty() {
                let push = stack_context_sync_push(
                    snapshot,
                    pull_requests,
                    &full_branches,
                    StackContextSyncMode::Full,
                );
                synced_pull_requests
                    .extend(self.sync_pull_requests_with_metadata(&push, metadata)?);
            }
        }

        if !context_only_branches.is_empty() {
            let push = stack_context_sync_push(
                snapshot,
                pull_requests,
                context_only_branches,
                StackContextSyncMode::ContextOnly,
            );
            if !push.bookmarks.is_empty() {
                synced_pull_requests
                    .extend(self.sync_pull_requests_with_metadata(&push, metadata)?);
            }
        }

        deduplicate_pull_requests_by_number(&mut synced_pull_requests);
        Ok(PullRequestStackPublishUpdate {
            pull_requests: synced_pull_requests,
        })
    }

    fn sync_metadata(&self) -> Result<StackMetadata, CommandError> {
        self.refresh_metadata_by_number(self.read_metadata()?)
    }

    fn apply_local_stack_metadata(
        &self,
        metadata: StackMetadata,
    ) -> Result<StackMetadata, CommandError> {
        let local_branches = self.services.local_stack_branches(self.context)?;
        Ok(apply_local_stack_branches(&local_branches, &metadata))
    }

    fn apply_local_stack_metadata_traced(
        &self,
        metadata: StackMetadata,
        span: &mut PerfSpan,
        step_name: &'static str,
    ) -> Result<StackMetadata, CommandError> {
        let metadata_node_count = metadata.nodes.len();
        let step = span.start_step(
            step_name,
            [perf_attr("metadata_node_count", metadata_node_count)],
        );
        let result = self.apply_local_stack_metadata_traced_inner(metadata, span, step_name);
        span.finish_step(step, metadata_result_attrs(&result), result.as_ref().err());
        result
    }

    fn apply_local_stack_metadata_traced_inner(
        &self,
        metadata: StackMetadata,
        span: &mut PerfSpan,
        step_name: &'static str,
    ) -> Result<StackMetadata, CommandError> {
        let metadata_node_count = metadata.nodes.len();
        let local_branches = span.measure_with_result_attrs(
            format!("{step_name}.local_stack_branches"),
            [perf_attr("metadata_node_count", metadata_node_count)],
            || self.services.local_stack_branches(self.context),
            local_stack_branch_result_attrs,
        )?;
        self.apply_local_stack_branches_snapshot(metadata, &local_branches, span, step_name)
    }

    fn apply_local_stack_metadata_snapshot_traced(
        &self,
        metadata: StackMetadata,
        local_branches: &[LocalStackBranch],
        span: &mut PerfSpan,
        step_name: &'static str,
    ) -> Result<StackMetadata, CommandError> {
        let step = span.start_step(
            step_name,
            [
                perf_attr("metadata_node_count", metadata.nodes.len()),
                perf_attr("local_branch_count", local_branches.len()),
                perf_attr("reused_local_branches", true),
            ],
        );
        let result =
            self.apply_local_stack_branches_snapshot(metadata, local_branches, span, step_name);
        span.finish_step(step, metadata_result_attrs(&result), result.as_ref().err());
        result
    }

    fn apply_local_stack_branches_snapshot(
        &self,
        metadata: StackMetadata,
        local_branches: &[LocalStackBranch],
        span: &mut PerfSpan,
        step_name: &'static str,
    ) -> Result<StackMetadata, CommandError> {
        let metadata_node_count = metadata.nodes.len();
        span.measure_with_result_attrs(
            format!("{step_name}.apply_local_stack_branches"),
            [
                perf_attr("local_branch_count", local_branches.len()),
                perf_attr("metadata_node_count", metadata_node_count),
            ],
            || Ok::<_, CommandError>(apply_local_stack_branches(local_branches, &metadata)),
            metadata_result_attrs,
        )
    }

    fn refresh_metadata_by_number(
        &self,
        metadata: StackMetadata,
    ) -> Result<StackMetadata, CommandError> {
        let pull_requests = self.pull_requests_for_metadata_numbers(&metadata)?;
        if pull_requests.is_empty() {
            return Ok(metadata);
        }

        let refreshed = refresh_stack_metadata_pull_requests(&pull_requests, &metadata);
        if refreshed != metadata {
            self.write_metadata(&refreshed)?;
        }
        Ok(refreshed)
    }

    fn pull_requests_for_metadata_numbers(
        &self,
        metadata: &StackMetadata,
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        let numbers = metadata
            .nodes
            .iter()
            .filter_map(|node| node.pull_request)
            .collect::<Vec<_>>();
        self.services
            .find_pull_requests_by_numbers(self.context, &numbers)
            .map_err(Into::into)
    }

    fn read_metadata(&self) -> Result<StackMetadata, CommandError> {
        read_stack_metadata(&self.context.repository_root).map_err(Into::into)
    }

    fn write_metadata(&self, metadata: &StackMetadata) -> Result<(), CommandError> {
        write_stack_metadata(&self.context.repository_root, metadata).map_err(Into::into)
    }
}

fn merge_work_ids(target: &mut Vec<String>, work_ids: &[String]) {
    for work_id in work_ids.iter().map(|work_id| work_id.trim()) {
        if !work_id.is_empty() && !target.iter().any(|existing| existing == work_id) {
            target.push(work_id.to_owned());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PullRequestMergedEffect {
    repository: GitHubRepository,
    pull_request: u64,
    pull_request_url: Option<String>,
    title: String,
    branch: String,
    base_branch: String,
    merged_at: Option<String>,
}

fn pull_request_merged_effects(
    context: &RepositoryContext,
    metadata: &StackMetadata,
    statuses: &[PullRequestStatusRecord],
) -> Vec<PullRequestMergedEffect> {
    let statuses_by_number = statuses
        .iter()
        .map(|status| (status.number, status))
        .collect::<BTreeMap<_, _>>();
    let mut effects = Vec::new();
    let mut seen = BTreeSet::new();
    for node in &metadata.nodes {
        if !node.merged {
            continue;
        }
        let Some(pull_request) = node.pull_request else {
            continue;
        };
        if !seen.insert(pull_request) {
            continue;
        }
        let status = statuses_by_number.get(&pull_request).copied();
        effects.push(PullRequestMergedEffect {
            repository: context.origin.github.clone(),
            pull_request,
            pull_request_url: status
                .and_then(|status| status.url.clone())
                .or(node.url.clone()),
            title: status
                .map(|status| status.title.clone())
                .unwrap_or_else(|| node.title.clone()),
            branch: status
                .map(|status| status.head_branch.clone())
                .unwrap_or_else(|| node.branch.clone()),
            base_branch: status
                .map(|status| status.base_branch.clone())
                .unwrap_or_else(|| node.base_branch.clone()),
            merged_at: status.and_then(|status| status.merged_at.clone()),
        });
    }
    effects
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkItemFixedEffect {
    work_id: String,
    repository: GitHubRepository,
    pull_request: u64,
    pull_request_url: Option<String>,
    title: String,
    branch: String,
}

fn fixed_work_item_effects(
    context: &RepositoryContext,
    metadata: &StackMetadata,
) -> Vec<WorkItemFixedEffect> {
    let mut effects = Vec::new();
    let mut seen = BTreeSet::new();
    for node in &metadata.nodes {
        if !node.merged || node.fixes_work_ids.is_empty() {
            continue;
        }
        let Some(pull_request) = node.pull_request else {
            continue;
        };
        for work_id in &node.fixes_work_ids {
            if seen.insert((work_id.clone(), pull_request)) {
                effects.push(WorkItemFixedEffect {
                    work_id: work_id.clone(),
                    repository: context.origin.github.clone(),
                    pull_request,
                    pull_request_url: node.url.clone(),
                    title: node.title.clone(),
                    branch: node.branch.clone(),
                });
            }
        }
    }
    effects
}

fn work_item_handler_run(
    handler: &RepoWorkItemHandler,
    effect: &WorkItemFixedEffect,
) -> StackMetadataWorkItemHandlerRun {
    StackMetadataWorkItemHandlerRun {
        handler: work_item_handler_label(handler),
        work_id: effect.work_id.clone(),
        pull_request: effect.pull_request,
    }
}

/// Maintains durable stack-status metadata through the single path shared by local and global status views.
pub(super) struct StackStatusMetadataMaintainer<'a> {
    context: &'a RepositoryContext,
    environment: &'a RuntimeEnvironment,
    pull_request_handler_log: PullRequestHandlerLog,
    work_item_handler_log: WorkItemHandlerLog,
}

impl<'a> StackStatusMetadataMaintainer<'a> {
    pub(super) fn new(context: &'a RepositoryContext, environment: &'a RuntimeEnvironment) -> Self {
        Self {
            context,
            environment,
            pull_request_handler_log: PullRequestHandlerLog::from_environment(environment),
            work_item_handler_log: WorkItemHandlerLog::from_environment(environment),
        }
    }

    /// Applies status facts, reconciles configured work-item side effects, and writes changed metadata.
    pub(super) fn maintain(
        &self,
        metadata: &StackMetadata,
        statuses: &[PullRequestStatusRecord],
    ) -> Result<StackStatusMetadataMaintenance, CommandError> {
        let mut refreshed =
            domain::refresh_stack_metadata_pull_request_statuses(statuses, metadata);
        // Reconcile side effects before completed stacks age out so missing ledgers do not skip configured cleanup.
        apply_pull_request_effects(
            self.context,
            self.environment,
            &self.context.repository_root,
            &refreshed,
            statuses,
            &self.pull_request_handler_log,
        )?;
        apply_work_item_effects(
            self.context,
            &self.context.repository_root,
            &mut refreshed,
            &self.work_item_handler_log,
        )?;
        let maintained =
            domain::maintain_stack_metadata_pull_request_statuses(statuses, &refreshed);
        if &maintained != metadata {
            write_stack_metadata(&self.context.repository_root, &maintained)?;
        }
        Ok(StackStatusMetadataMaintenance {
            metadata: maintained,
        })
    }
}

/// Updated stack metadata after status maintenance has run to completion.
pub(super) struct StackStatusMetadataMaintenance {
    pub(super) metadata: StackMetadata,
}

fn apply_pull_request_effects(
    context: &RepositoryContext,
    environment: &RuntimeEnvironment,
    repository_root: &Path,
    metadata: &StackMetadata,
    statuses: &[PullRequestStatusRecord],
    log: &PullRequestHandlerLog,
) -> Result<(), CommandError> {
    let handlers = context
        .config
        .repo
        .pull_request_handlers_for(&context.origin.github)
        .into_iter()
        .filter(|handler| handler.on == RepoPullRequestEvent::Merged)
        .collect::<Vec<_>>();
    if handlers.is_empty() {
        return Ok(());
    }

    let effects = pull_request_merged_effects(context, metadata, statuses);
    if effects.is_empty() {
        return Ok(());
    }

    let store = PullRequestStore::open(environment)?;
    let mut applied_runs = BTreeSet::new();
    for effect in &effects {
        for handler in &handlers {
            let handler_label = pull_request_handler_label(handler);
            let run = (handler.on, handler_label.clone(), effect.pull_request);
            if applied_runs.contains(&run)
                || store.has_pull_request_action(
                    &effect.repository,
                    effect.pull_request,
                    PULL_REQUEST_HANDLER_ACTION,
                    handler.on.label(),
                    Some(handler_label.as_str()),
                )?
            {
                continue;
            }
            let command = render_pull_request_handler_command(handler, effect);
            run_pull_request_handler(handler, effect, &command, repository_root, log)?;
            store.record_pull_request_action(
                &effect.repository,
                effect.pull_request,
                PULL_REQUEST_HANDLER_ACTION,
                handler.on.label(),
                Some(handler_label.as_str()),
                pull_request_handler_action_details(handler, effect, &command),
            )?;
            applied_runs.insert(run);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PullRequestHandlerLog {
    path: Option<PathBuf>,
}

impl PullRequestHandlerLog {
    fn from_environment(environment: &RuntimeEnvironment) -> Self {
        Self {
            path: pull_request_handler_log_path(environment),
        }
    }

    fn append(
        &self,
        repository_root: &Path,
        handler: &RepoPullRequestHandler,
        effect: &PullRequestMergedEffect,
        command: &[String],
        status: &str,
        message: Option<&str>,
    ) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        else {
            return;
        };
        let mut record = serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "status": status,
            "handler": pull_request_handler_label(handler),
            "event": handler.on.label(),
            "repo": effect.repository.slug(),
            "prNumber": effect.pull_request,
            "prUrl": effect.pull_request_url.as_deref(),
            "title": effect.title.as_str(),
            "branch": effect.branch.as_str(),
            "baseBranch": effect.base_branch.as_str(),
            "mergedAt": effect.merged_at.as_deref(),
            "cwd": repository_root.display().to_string(),
            "command": command,
        });
        if let Some(message) = message {
            record["message"] = serde_json::Value::String(message.to_owned());
        }
        if serde_json::to_writer(&mut file, &record).is_ok() {
            let _ = writeln!(file);
        }
    }
}

fn pull_request_handler_log_path(environment: &RuntimeEnvironment) -> Option<PathBuf> {
    if let Some(path) = environment
        .variable("JX_PULL_REQUEST_HANDLER_LOG")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        if matches!(path, "off" | "false" | "0") {
            return None;
        }
        return Some(PathBuf::from(path));
    }

    environment
        .variable("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .home_dir()
                .map(|home| home.join(".local").join("state"))
        })
        .map(|root| root.join("jx").join(PULL_REQUEST_HANDLER_LOG_FILE))
}

fn run_pull_request_handler(
    handler: &RepoPullRequestHandler,
    effect: &PullRequestMergedEffect,
    command: &[String],
    repository_root: &Path,
    log: &PullRequestHandlerLog,
) -> Result<(), CommandError> {
    let handler_label = pull_request_handler_label(handler);
    let Some(program) = command.first() else {
        log.append(
            repository_root,
            handler,
            effect,
            command,
            "error",
            Some("command is empty"),
        );
        return Err(CommandError::PullRequestHandler {
            handler: handler_label,
            pull_request: effect.pull_request,
            message: "command is empty".to_owned(),
        });
    };
    log.append(repository_root, handler, effect, command, "start", None);
    let status = ProcessCommand::new(program)
        .args(command.iter().skip(1))
        .current_dir(repository_root)
        .status()
        .map_err(|source| {
            let message = source.to_string();
            log.append(
                repository_root,
                handler,
                effect,
                command,
                "error",
                Some(message.as_str()),
            );
            CommandError::PullRequestHandler {
                handler: handler_label.clone(),
                pull_request: effect.pull_request,
                message,
            }
        })?;
    if !status.success() {
        let message = status.to_string();
        log.append(
            repository_root,
            handler,
            effect,
            command,
            "error",
            Some(message.as_str()),
        );
        return Err(CommandError::PullRequestHandler {
            handler: handler_label,
            pull_request: effect.pull_request,
            message,
        });
    }
    log.append(repository_root, handler, effect, command, "success", None);
    Ok(())
}

fn render_pull_request_handler_command(
    handler: &RepoPullRequestHandler,
    effect: &PullRequestMergedEffect,
) -> Vec<String> {
    handler
        .command
        .iter()
        .map(|arg| render_pull_request_handler_arg(arg, effect))
        .collect()
}

fn render_pull_request_handler_arg(arg: &str, effect: &PullRequestMergedEffect) -> String {
    arg.replace("{repo}", &effect.repository.slug())
        .replace("{pr_number}", &effect.pull_request.to_string())
        .replace(
            "{pr_url}",
            effect.pull_request_url.as_deref().unwrap_or_default(),
        )
        .replace("{title}", &effect.title)
        .replace("{branch}", &effect.branch)
        .replace("{base_branch}", &effect.base_branch)
        .replace(
            "{merged_at}",
            effect.merged_at.as_deref().unwrap_or_default(),
        )
}

fn pull_request_handler_action_details(
    handler: &RepoPullRequestHandler,
    effect: &PullRequestMergedEffect,
    command: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "handler": pull_request_handler_label(handler),
        "event": handler.on.label(),
        "repo": effect.repository.slug(),
        "prNumber": effect.pull_request,
        "prUrl": effect.pull_request_url.as_deref(),
        "title": effect.title.as_str(),
        "branch": effect.branch.as_str(),
        "baseBranch": effect.base_branch.as_str(),
        "mergedAt": effect.merged_at.as_deref(),
        "command": command,
    })
}

fn pull_request_handler_label(handler: &RepoPullRequestHandler) -> String {
    handler
        .id
        .clone()
        .unwrap_or_else(|| handler.on.label().to_owned())
}

fn apply_work_item_effects(
    context: &RepositoryContext,
    repository_root: &Path,
    metadata: &mut StackMetadata,
    log: &WorkItemHandlerLog,
) -> Result<(), CommandError> {
    if !context
        .config
        .repo
        .work_items_for(&context.origin.github)
        .apply_on_stack_status()
    {
        return Ok(());
    }

    let handlers = context
        .config
        .repo
        .work_item_handlers_for(&context.origin.github)
        .into_iter()
        .filter(|handler| handler.on == RepoWorkItemEvent::Fixed)
        .collect::<Vec<_>>();
    if handlers.is_empty() {
        return Ok(());
    }

    let effects = fixed_work_item_effects(context, metadata);
    if effects.is_empty() {
        return Ok(());
    }

    let mut recorded_runs = metadata
        .work_item_handler_runs
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut applied_runs = Vec::new();
    for effect in &effects {
        for handler in &handlers {
            let run = work_item_handler_run(handler, effect);
            if recorded_runs.contains(&run) {
                continue;
            }
            run_work_item_handler(handler, effect, repository_root, log)?;
            if recorded_runs.insert(run.clone()) {
                applied_runs.push(run);
            }
        }
    }
    if !applied_runs.is_empty() {
        metadata.work_item_handler_runs.extend(applied_runs);
        metadata.work_item_handler_runs.sort();
        metadata.work_item_handler_runs.dedup();
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkItemHandlerLog {
    path: Option<PathBuf>,
}

impl WorkItemHandlerLog {
    fn from_environment(environment: &RuntimeEnvironment) -> Self {
        Self {
            path: work_item_handler_log_path(environment),
        }
    }

    fn append(
        &self,
        repository_root: &Path,
        handler: &RepoWorkItemHandler,
        effect: &WorkItemFixedEffect,
        command: &[String],
        status: &str,
        message: Option<&str>,
    ) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        else {
            return;
        };
        let mut record = serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "status": status,
            "handler": work_item_handler_label(handler),
            "event": handler.on.label(),
            "workId": effect.work_id.as_str(),
            "repo": effect.repository.slug(),
            "prNumber": effect.pull_request,
            "prUrl": effect.pull_request_url.as_deref(),
            "title": effect.title.as_str(),
            "branch": effect.branch.as_str(),
            "cwd": repository_root.display().to_string(),
            "command": command,
        });
        if let Some(message) = message {
            record["message"] = serde_json::Value::String(message.to_owned());
        }
        if serde_json::to_writer(&mut file, &record).is_ok() {
            let _ = writeln!(file);
        }
    }
}

fn work_item_handler_log_path(environment: &RuntimeEnvironment) -> Option<PathBuf> {
    if let Some(path) = environment
        .variable("JX_WORK_ITEM_HANDLER_LOG")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        if matches!(path, "off" | "false" | "0") {
            return None;
        }
        return Some(PathBuf::from(path));
    }

    environment
        .variable("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .home_dir()
                .map(|home| home.join(".local").join("state"))
        })
        .map(|root| root.join("jx").join(WORK_ITEM_HANDLER_LOG_FILE))
}

fn run_work_item_handler(
    handler: &RepoWorkItemHandler,
    effect: &WorkItemFixedEffect,
    repository_root: &Path,
    log: &WorkItemHandlerLog,
) -> Result<(), CommandError> {
    let command = render_work_item_handler_command(handler, effect);
    let handler_label = work_item_handler_label(handler);
    let Some(program) = command.first() else {
        log.append(
            repository_root,
            handler,
            effect,
            &command,
            "error",
            Some("command is empty"),
        );
        return Err(CommandError::WorkItemHandler {
            handler: handler_label,
            work_id: effect.work_id.clone(),
            message: "command is empty".to_owned(),
        });
    };
    log.append(repository_root, handler, effect, &command, "start", None);
    let status = ProcessCommand::new(program)
        .args(command.iter().skip(1))
        .current_dir(repository_root)
        .status()
        .map_err(|source| {
            let message = source.to_string();
            log.append(
                repository_root,
                handler,
                effect,
                &command,
                "error",
                Some(message.as_str()),
            );
            CommandError::WorkItemHandler {
                handler: handler_label.clone(),
                work_id: effect.work_id.clone(),
                message,
            }
        })?;
    if !status.success() {
        let message = status.to_string();
        log.append(
            repository_root,
            handler,
            effect,
            &command,
            "error",
            Some(message.as_str()),
        );
        return Err(CommandError::WorkItemHandler {
            handler: handler_label,
            work_id: effect.work_id.clone(),
            message,
        });
    }
    log.append(repository_root, handler, effect, &command, "success", None);
    Ok(())
}

fn render_work_item_handler_command(
    handler: &RepoWorkItemHandler,
    effect: &WorkItemFixedEffect,
) -> Vec<String> {
    handler
        .command
        .iter()
        .map(|arg| render_work_item_handler_arg(arg, effect))
        .collect()
}

fn render_work_item_handler_arg(arg: &str, effect: &WorkItemFixedEffect) -> String {
    arg.replace("{work_id}", &effect.work_id)
        .replace("{repo}", &effect.repository.slug())
        .replace("{pr_number}", &effect.pull_request.to_string())
        .replace(
            "{pr_url}",
            effect.pull_request_url.as_deref().unwrap_or_default(),
        )
        .replace("{title}", &effect.title)
        .replace("{branch}", &effect.branch)
}

fn work_item_handler_label(handler: &RepoWorkItemHandler) -> String {
    handler
        .id
        .clone()
        .unwrap_or_else(|| handler.on.label().to_owned())
}

fn metadata_result_attrs(result: &Result<StackMetadata, CommandError>) -> Vec<PerfAttr> {
    result
        .as_ref()
        .map(|metadata| vec![perf_attr("metadata_node_count", metadata.nodes.len())])
        .unwrap_or_default()
}

fn local_stack_branch_result_attrs(
    result: &Result<Vec<LocalStackBranch>, JjError>,
) -> Vec<PerfAttr> {
    result
        .as_ref()
        .map(|branches| vec![perf_attr("local_branch_count", branches.len())])
        .unwrap_or_default()
}

fn local_stack_branch_facts_result_attrs(
    result: &Result<LocalStackBranchFacts, JjError>,
) -> Vec<PerfAttr> {
    result
        .as_ref()
        .map(|facts| local_stack_branch_metric_attrs(&facts.metrics))
        .unwrap_or_default()
}

fn local_stack_branch_metric_attrs(metrics: &LocalStackBranchMetrics) -> Vec<PerfAttr> {
    vec![
        perf_attr("local_branch_count", metrics.branch_count),
        perf_attr("local_bookmark_count", metrics.local_bookmark_count),
        perf_attr("normal_bookmark_count", metrics.normal_bookmark_count),
        perf_attr(
            "skipped_non_normal_bookmark_count",
            metrics.skipped_non_normal_bookmark_count,
        ),
        perf_attr("loaded_commit_count", metrics.loaded_commit_count),
        perf_attr("resolved_trunk_count", metrics.resolved_trunk_count),
        perf_attr(
            "skipped_missing_trunk_count",
            metrics.skipped_missing_trunk_count,
        ),
        perf_attr("stack_path_count", metrics.stack_path_count),
        perf_attr("skipped_non_linear_count", metrics.skipped_non_linear_count),
        perf_attr("skipped_trunk_count", metrics.skipped_trunk_count),
        perf_attr("jj_total_us", metrics.total_us),
    ]
}

fn record_local_stack_branch_metrics(span: &mut PerfSpan, metrics: &LocalStackBranchMetrics) {
    let attrs = local_stack_branch_metric_attrs(metrics);
    record_local_stack_branch_metric_step(
        span,
        "enumerate_bookmarks",
        metrics.enumerate_bookmarks_us,
        attrs.clone(),
    );
    record_local_stack_branch_metric_step(
        span,
        "load_commit",
        metrics.load_commit_us,
        attrs.clone(),
    );
    record_local_stack_branch_metric_step(
        span,
        "resolve_trunk",
        metrics.resolve_trunk_us,
        attrs.clone(),
    );
    record_local_stack_branch_metric_step(
        span,
        "linear_stack_path",
        metrics.linear_stack_path_us,
        attrs.clone(),
    );
    record_local_stack_branch_metric_step(
        span,
        "nearest_ancestor_bookmark",
        metrics.nearest_ancestor_bookmark_us,
        attrs.clone(),
    );
    record_local_stack_branch_metric_step(span, "sort_dedup", metrics.sort_dedup_us, attrs.clone());
    record_local_stack_branch_metric_step(span, "total", metrics.total_us, attrs);
}

fn record_local_stack_branch_metric_step(
    span: &mut PerfSpan,
    phase: &str,
    duration_us: u64,
    attrs: Vec<PerfAttr>,
) {
    span.record_step_us(
        format!("local_stack_branches.{phase}"),
        duration_us,
        attrs,
        None::<&CommandError>,
    );
}

struct AuthoredStackMetadataRefresh {
    metadata: StackMetadata,
    local_branches: Vec<String>,
    live_pull_requests: Vec<PullRequestRecord>,
}

fn stack_snapshot_has_openable_pull_request(snapshot: &PullRequestStackSnapshot) -> bool {
    snapshot
        .nodes
        .iter()
        .any(|node| node.pull_request_number().is_some())
}

fn missing_local_bookmark_pull_requests(context: &RepositoryContext) -> WorkflowError {
    WorkflowError::MissingLocalBookmarkPullRequests {
        repository: context.origin.github.slug(),
    }
}

#[derive(Clone, Copy)]
enum StackContextSyncMode {
    Full,
    ContextOnly,
}

fn stack_context_sync_push(
    component: &PullRequestStackSnapshot,
    pull_requests: &[PullRequestRecord],
    branches: &BTreeSet<String>,
    mode: StackContextSyncMode,
) -> TrackedPushOutcome {
    let pull_requests_by_branch = pull_requests
        .iter()
        .map(|pull_request| (pull_request.head_branch.as_str(), pull_request))
        .collect::<BTreeMap<_, _>>();
    let pull_requests_by_number = pull_requests
        .iter()
        .map(|pull_request| (pull_request.number, pull_request))
        .collect::<BTreeMap<_, _>>();
    let bookmarks = component
        .nodes
        .iter()
        .filter_map(|node| {
            let pull_request = node
                .pull_request_number()
                .and_then(|number| pull_requests_by_number.get(&number).copied())
                .or_else(|| pull_requests_by_branch.get(node.branch.as_str()).copied())?;
            branches
                .contains(&pull_request.head_branch)
                .then_some((node, pull_request))
        })
        .map(|(node, pull_request)| PushedBookmarkSummary {
            branch: pull_request.head_branch.clone(),
            old_short_commit_id: None,
            new_short_commit_id: None,
            old_short_change_id: None,
            new_short_change_id: None,
            old_description: None,
            new_description: Some(pull_request.title.clone()),
            pull_request_description: Some(pull_request_description(pull_request)),
            pull_request_base: match mode {
                StackContextSyncMode::Full => Some(node.base_branch.clone()),
                StackContextSyncMode::ContextOnly => None,
            },
            new_workspace_visibility: WorkspaceVisibility::default(),
        })
        .collect::<Vec<_>>();

    TrackedPushOutcome {
        pushed_refs: 0,
        bookmarks,
        pushed_commits: Vec::new(),
    }
}

fn deduplicate_pull_requests_by_number(pull_requests: &mut Vec<PullRequestRecord>) {
    let mut seen = BTreeSet::new();
    pull_requests.retain(|pull_request| seen.insert(pull_request.number));
}

fn pull_request_description(pull_request: &PullRequestRecord) -> String {
    match pull_request
        .body
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())
    {
        Some(body) => format!("{}\n\n{body}", pull_request.title),
        None => pull_request.title.clone(),
    }
}
