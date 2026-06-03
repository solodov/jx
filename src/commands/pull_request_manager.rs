use super::*;

/// Command-side integration boundary for pull-request stack state.
pub(super) struct PullRequestStackManager<'a> {
    context: &'a RepositoryContext,
    services: &'a dyn CommandServices,
    perf: PerfLog,
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
    ) -> Self {
        Self {
            context,
            services,
            perf,
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
        let maintained = domain::maintain_stack_metadata_pull_request_statuses(statuses, &metadata);
        if maintained != metadata {
            self.write_metadata(&maintained)?;
        }
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
        let metadata = self.sync_metadata()?;
        let local_branches = self.local_pull_request_branches()?;
        let branches = PullRequestStackSnapshot::from_metadata(
            &metadata,
            &local_branches,
            &[],
            PullRequestStackSelection::default(),
        )
        .local_component_branches_for(&selected_branches);

        Ok(PullRequestStackSyncSelection { branches, metadata })
    }

    /// Upserts newly published stack PRs and syncs stack context once for their component.
    pub(super) fn update_after_stack_publish(
        &self,
        reports: &[PullRequestReport],
    ) -> Result<PullRequestStackPublishUpdate, CommandError> {
        let mut span = self.perf.start(
            "stack.update_after_publish",
            [
                perf_attr("repo", self.context.origin.github.slug()),
                perf_attr("report_count", reports.len()),
            ],
        );
        let result = self.update_after_stack_publish_traced(reports, &mut span);
        if let Ok(update) = &result {
            span.set([perf_attr("updated_pr_count", update.pull_requests.len())]);
        }
        if let Err(error) = &result {
            span.record_error(error);
        }
        span.end();
        result
    }

    fn update_after_stack_publish_traced(
        &self,
        reports: &[PullRequestReport],
        span: &mut PerfSpan,
    ) -> Result<PullRequestStackPublishUpdate, CommandError> {
        let Some(selection) = reports
            .first()
            .map(|report| PullRequestStackSelection::pull_request(report.pull_request.number))
        else {
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
        span.set([perf_attr("seed_pr_count", seed_pull_requests.len())]);

        self.sync_stack_component(selection, &seed_pull_requests, span)
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
            [perf_attr(
                "component_pr_count",
                component_pull_requests.len(),
            )],
            || {
                self.sync_available_pull_requests_for_snapshot(
                    &refreshed_component,
                    &component_pull_requests,
                    &metadata,
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
        if local_branches.is_empty() {
            let metadata = StackMetadata::default();
            self.write_metadata(&metadata)?;
            return Ok(AuthoredStackMetadataRefresh {
                metadata,
                local_branches,
                live_pull_requests: Vec::new(),
            });
        }

        let metadata = self.refresh_metadata_by_number(self.read_metadata()?)?;
        let live_pull_requests = self.authored_open_pull_requests_for_branches(&local_branches)?;
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
        let mut pull_requests = Vec::new();
        let mut seen_numbers = BTreeSet::new();
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
        self.sync_available_pull_requests_for_snapshot(snapshot, &pull_requests, metadata)
    }

    fn sync_available_pull_requests_for_snapshot(
        &self,
        snapshot: &PullRequestStackSnapshot,
        pull_requests: &[PullRequestRecord],
        metadata: &StackMetadata,
    ) -> Result<PullRequestStackPublishUpdate, CommandError> {
        if pull_requests.is_empty() {
            return Ok(PullRequestStackPublishUpdate::default());
        }

        let push = stack_context_sync_push(snapshot, pull_requests);
        let pull_requests = self.sync_pull_requests_with_metadata(&push, metadata)?;
        Ok(PullRequestStackPublishUpdate { pull_requests })
    }

    fn sync_metadata(&self) -> Result<StackMetadata, CommandError> {
        let metadata = self.refresh_metadata_by_number(self.read_metadata()?)?;
        let pruned = prune_merged_stack_metadata_trees(&metadata);
        if pruned != metadata {
            self.write_metadata(&pruned)?;
        }
        Ok(pruned)
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

fn stack_context_sync_push(
    component: &PullRequestStackSnapshot,
    pull_requests: &[PullRequestRecord],
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
        .filter(|node| {
            node.pull_request_number()
                .and_then(|number| pull_requests_by_number.get(&number).copied())
                .or_else(|| pull_requests_by_branch.get(node.branch.as_str()).copied())
                .is_some()
        })
        .map(|node| {
            let pull_request = node
                .pull_request_number()
                .and_then(|number| pull_requests_by_number.get(&number).copied())
                .or_else(|| pull_requests_by_branch.get(node.branch.as_str()).copied())
                .expect("filter ensured pull request exists");
            PushedBookmarkSummary {
                branch: pull_request.head_branch.clone(),
                old_short_commit_id: None,
                new_short_commit_id: None,
                old_description: None,
                new_description: Some(pull_request.title.clone()),
                pull_request_description: Some(pull_request_description(pull_request)),
                pull_request_base: Some(node.base_branch.clone()),
                new_workspace_visibility: WorkspaceVisibility::default(),
            }
        })
        .collect::<Vec<_>>();

    TrackedPushOutcome {
        pushed_refs: 0,
        bookmarks,
        pushed_commits: Vec::new(),
    }
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
