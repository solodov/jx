use super::*;
use crate::jj::{StackPublishMetrics, StackPublishNodeFacts};

#[cfg(test)]
use crate::repository::StackMetadataNode;

#[derive(Default)]
struct StackPublishPlanSelection {
    plans: Vec<PullRequestPlan>,
    context_only_pull_requests: Vec<PullRequestRecord>,
    skipped_count: usize,
}

/// Handles repo-local pull-request stack state that survives bookmark movement.
pub(super) fn handle_stack(
    request: StackRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let perf = PerfLog::from_environment(environment);
    if let StackRequest::Status(request) = request {
        if request.interactive {
            return run_stack_status_dashboard(request, environment);
        }
        if request.all {
            return handle_global_stack_status(
                request,
                environment,
                services,
                progress,
                output,
                &perf,
            );
        }
        let context = RepositoryContext::discover(environment)?;
        let manager = PullRequestStackManager::new(&context, services, perf.clone(), environment);
        return StackStatusExecution {
            services,
            progress,
            context: &context,
            manager: &manager,
            output,
            perf: &perf,
        }
        .run(request);
    }

    let context = RepositoryContext::discover(environment)?;
    let manager = PullRequestStackManager::new(&context, services, perf.clone(), environment);
    match request {
        StackRequest::Show => {
            progress.status("Loading pull request stack…");
            let snapshot = manager.stored_snapshot(PullRequestStackSelection::default());
            progress.finish();
            Ok(CommandResult::success(render_stack_snapshot(
                &snapshot?,
                output.color,
            )?))
        }
        StackRequest::Open { print } => {
            progress.status("Loading pull request stack…");
            let snapshot = manager.cached_open_snapshot();
            progress.finish();
            Ok(CommandResult::success(open_stack_pull_request(
                &context,
                services,
                prompts.pull_request_selector,
                &snapshot?,
                print,
            )?))
        }
        StackRequest::Refresh => {
            progress.status("Refreshing pull request stack…");
            let snapshot = manager.refresh_and_sync_authored_open_pull_requests();
            progress.finish();
            Ok(CommandResult::success(render_stack_snapshot(
                &snapshot?,
                output.color,
            )?))
        }
        StackRequest::Move(request) => StackMoveExecution {
            environment,
            services,
            progress,
            context: &context,
            manager: &manager,
            output,
        }
        .run(request),
        StackRequest::Plan(request) => {
            progress.status("Planning pull request stack…");
            let selection = stack_plan_selection(&request.revisions);
            let facts = services.stack_plan_facts(&context, &selection);
            progress.finish();
            Ok(CommandResult::success(render_stack_plan(&facts?)?))
        }
        StackRequest::Publish(request) => StackPublishExecution {
            environment,
            services,
            progress,
            prompts,
            context: &context,
            manager: &manager,
            output,
            perf: &perf,
        }
        .run(request),
        StackRequest::CompleteReviewers(request) => Ok(CommandResult::success(
            render_stack_reviewer_completion(&context, &request.prefix),
        )),
        StackRequest::Status(_) => {
            unreachable!("stack status is handled before current-repo dispatch")
        }
    }
}

/// Renders configured reviewer names for shell completion without constraining CLI input.
fn render_stack_reviewer_completion(context: &RepositoryContext, prefix: &str) -> String {
    context
        .config
        .repo
        .reviewer_completion_for(&context.origin.github)
        .into_iter()
        .map(|reviewer| reviewer.display_name().to_owned())
        .filter(|reviewer| reviewer.starts_with(prefix))
        .map(|reviewer| format!("{reviewer}\n"))
        .collect()
}

fn run_stack_status_dashboard(
    request: StackStatusRequest,
    environment: &RuntimeEnvironment,
) -> Result<CommandResult, CommandError> {
    let environment = environment.clone();
    let loader_request = request.clone();
    let loader: DashboardFrameLoader = std::sync::Arc::new(move || {
        render_stack_status_dashboard_frame(loader_request.clone(), &environment)
            .map_err(|error| error.to_string())
    });
    run_interactive_dashboard("jx stack status", request.refresh_seconds, loader)
}

fn render_stack_status_dashboard_frame(
    request: StackStatusRequest,
    environment: &RuntimeEnvironment,
) -> Result<String, CommandError> {
    let services = ProductionServices::new(environment)?;
    let progress = SilentProgress;
    let output = OutputMode::from_process();
    let perf = PerfLog::from_environment(environment);
    let mut span = perf.start(
        "stack.status.dashboard_frame",
        [
            perf_attr("all", request.all),
            perf_attr("repo_filter_count", request.repo_filters.len()),
            perf_attr("parallelism", request.parallelism),
            perf_attr("format", stack_status_format_label(request.format)),
        ],
    );
    let result = if request.all {
        render_global_stack_status_dashboard_frame(
            &request,
            environment,
            &services,
            &progress,
            output,
            &mut span,
        )
    } else {
        render_current_stack_status_dashboard_frame(
            request,
            environment,
            &services,
            &progress,
            output,
            &perf,
            &mut span,
        )
    };
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn render_global_stack_status_dashboard_frame(
    request: &StackStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
    span: &mut PerfSpan,
) -> Result<String, CommandError> {
    let loaded = load_global_stack_status_view(request, environment, services, progress, span)?;
    span.measure(
        "render",
        [perf_attr(
            "format",
            stack_status_format_label(request.format),
        )],
        || {
            render_global_stack_status_output(
                &loaded.entries,
                loaded.total_repositories,
                environment.current_dir(),
                output.color,
                output.terminal_width,
                request.format,
                &loaded.display_names,
            )
        },
    )
}

fn render_current_stack_status_dashboard_frame(
    request: StackStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
    perf: &PerfLog,
    span: &mut PerfSpan,
) -> Result<String, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    let manager = PullRequestStackManager::new(&context, services, perf.clone(), environment);
    let execution = StackStatusExecution {
        services,
        progress,
        context: &context,
        manager: &manager,
        output,
        perf,
    };
    let loaded = execution.load_status_view(&request, span)?;
    span.measure(
        "render",
        [perf_attr(
            "format",
            stack_status_format_label(request.format),
        )],
        || {
            render_stack_status_output(
                &loaded.report,
                &context.repository_root,
                output.color,
                output.terminal_width,
                request.format,
                &loaded.display_names,
            )
        },
    )
}

fn handle_global_stack_status(
    request: StackStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
    perf: &PerfLog,
) -> Result<CommandResult, CommandError> {
    let mut span = perf.start(
        "stack.status",
        [
            perf_attr("all", true),
            perf_attr("repo_filter_count", request.repo_filters.len()),
            perf_attr("parallelism", request.parallelism),
            perf_attr("format", stack_status_format_label(request.format)),
        ],
    );
    let result = handle_global_stack_status_traced(
        request,
        environment,
        services,
        progress,
        output,
        &mut span,
    );
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn handle_global_stack_status_traced(
    request: StackStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
    span: &mut PerfSpan,
) -> Result<CommandResult, CommandError> {
    let loaded = load_global_stack_status_view(&request, environment, services, progress, span)?;
    progress.finish();

    let stdout = span.measure(
        "render",
        [perf_attr(
            "format",
            stack_status_format_label(request.format),
        )],
        || {
            render_global_stack_status_output(
                &loaded.entries,
                loaded.total_repositories,
                environment.current_dir(),
                output.color,
                output.terminal_width,
                request.format,
                &loaded.display_names,
            )
        },
    )?;
    Ok(CommandResult::success(stdout))
}

struct LoadedGlobalStackStatusView {
    entries: Vec<GlobalStackStatusEntry>,
    total_repositories: usize,
    display_names: BTreeMap<String, String>,
}

fn load_global_stack_status_view(
    request: &StackStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    span: &mut PerfSpan,
) -> Result<LoadedGlobalStackStatusView, CommandError> {
    progress.status("Discovering repositories…");
    let config = span.measure("discover_config", Vec::new(), || {
        WorkflowConfig::discover_global(environment).map_err(CommandError::from)
    })?;
    let token_source = TokenSource::discover(environment, &config);
    let repositories = span.measure("discover_repositories", Vec::new(), || {
        global_work_repositories(&config, environment).map_err(CommandError::from)
    })?;
    span.set([perf_attr("discovered_repo_count", repositories.len())]);
    let repositories = span.measure(
        "filter_repositories",
        [perf_attr("repo_filter_count", request.repo_filters.len())],
        || {
            filter_work_repositories(&repositories, &request.repo_filters)
                .map_err(CommandError::from)
        },
    )?;
    span.set([perf_attr("selected_repo_count", repositories.len())]);

    if !repositories.is_empty() {
        progress.percentage("Checking stack status", 0, repositories.len());
    }
    let entries = span.measure_with_result_attrs(
        "fetch_global_stack_status",
        [
            perf_attr("selected_repo_count", repositories.len()),
            perf_attr("parallelism", request.parallelism),
        ],
        || {
            Ok::<_, CommandError>(services.global_stack_status_entries(
                &repositories,
                request,
                environment,
                progress,
            ))
        },
        |result| {
            result
                .as_ref()
                .map(|entries| vec![perf_attr("entry_count", entries.len())])
                .unwrap_or_default()
        },
    )?;
    let display_name_logins = stack_status_entry_user_logins(&entries);
    let display_names = span.measure_with_result_attrs(
        "load_display_names",
        [
            perf_attr("format", stack_status_format_label(request.format)),
            perf_attr("login_count", display_name_logins.len()),
        ],
        || {
            Ok::<_, CommandError>(stack_status_display_names(
                services,
                &token_source,
                display_name_logins,
                request.format,
                progress,
            ))
        },
        |result| {
            result
                .as_ref()
                .map(|display_names| vec![perf_attr("display_name_count", display_names.len())])
                .unwrap_or_default()
        },
    )?;

    Ok(LoadedGlobalStackStatusView {
        total_repositories: repositories.len(),
        entries,
        display_names,
    })
}

struct StackStatusExecution<'a> {
    services: &'a dyn CommandServices,
    progress: &'a dyn ProgressSink,
    context: &'a RepositoryContext,
    manager: &'a PullRequestStackManager<'a>,
    output: OutputMode,
    perf: &'a PerfLog,
}

impl StackStatusExecution<'_> {
    fn run(&self, request: StackStatusRequest) -> Result<CommandResult, CommandError> {
        let mut span = self.perf.start(
            "stack.status",
            [
                perf_attr("repo", self.context.origin.github.slug()),
                perf_attr("all", false),
                perf_attr("repo_filter_count", request.repo_filters.len()),
                perf_attr("parallelism", request.parallelism),
                perf_attr("format", stack_status_format_label(request.format)),
            ],
        );
        let result = self.run_traced(request, &mut span);
        if let Err(error) = &result {
            span.record_error(error);
        }
        span.end();
        result
    }

    fn run_traced(
        &self,
        request: StackStatusRequest,
        span: &mut PerfSpan,
    ) -> Result<CommandResult, CommandError> {
        let loaded = self.load_status_view(&request, span)?;
        self.progress.finish();

        let stdout = span.measure(
            "render",
            [perf_attr(
                "format",
                stack_status_format_label(request.format),
            )],
            || {
                render_stack_status_output(
                    &loaded.report,
                    &self.context.repository_root,
                    self.output.color,
                    self.output.terminal_width,
                    request.format,
                    &loaded.display_names,
                )
            },
        )?;
        Ok(CommandResult::success(stdout))
    }

    fn load_status_view(
        &self,
        request: &StackStatusRequest,
        span: &mut PerfSpan,
    ) -> Result<LoadedStackStatusView, CommandError> {
        self.progress.status("Loading pull request stack…");
        let snapshot = span.measure_with_result_attrs(
            "load_stack_snapshot",
            Vec::new(),
            || {
                self.manager
                    .stored_snapshot(PullRequestStackSelection::default())
            },
            stack_snapshot_result_attrs,
        )?;
        let discovered_pull_requests = span.measure_with_result_attrs(
            "discover_missing_pull_requests",
            Vec::new(),
            || stack_status_missing_pull_requests(self.services, self.context, &snapshot),
            pull_request_discovery_result_attrs,
        )?;
        let numbers = stack_status_pull_request_numbers(&snapshot, &discovered_pull_requests);
        span.set([
            perf_attr("stack_node_count", snapshot.nodes.len()),
            perf_attr("pr_count", numbers.len()),
        ]);

        self.progress.status("Loading GitHub stack status…");
        let fetches = self.services.stack_status_fetches(
            self.context,
            &numbers,
            !snapshot.nodes.is_empty(),
        )?;
        if !numbers.is_empty() {
            span.record_step_us(
                "fetch_github_status",
                fetches.fetch_github_status_us,
                [
                    perf_attr("pr_count", numbers.len()),
                    perf_attr("status_count", fetches.statuses.len()),
                ],
                Option::<&CommandError>::None,
            );
        }
        if let Some(duration_us) = fetches.fetch_trunk_status_us {
            span.record_step_us(
                "fetch_trunk_status",
                duration_us,
                stack_status_trunk_attrs(fetches.trunk.as_ref()),
                Option::<&CommandError>::None,
            );
        }

        let statuses = fetches.statuses;
        self.progress.status("Maintaining stack cache…");
        let snapshot = span.measure_with_result_attrs(
            "maintain_stack_metadata",
            [perf_attr("status_count", statuses.len())],
            || self.manager.maintain_status_metadata(&statuses),
            stack_snapshot_result_attrs,
        )?;
        let trunk = (!snapshot.nodes.is_empty())
            .then_some(fetches.trunk)
            .flatten();
        let report =
            domain::pull_request_stack_status_report(self.context, snapshot, statuses, trunk);
        let display_name_logins = pull_request_status_user_logins(report.statuses.values());
        let display_names = span.measure_with_result_attrs(
            "load_display_names",
            [
                perf_attr("format", stack_status_format_label(request.format)),
                perf_attr("login_count", display_name_logins.len()),
            ],
            || {
                Ok::<_, CommandError>(stack_status_display_names(
                    self.services,
                    &self.context.token_source,
                    display_name_logins,
                    request.format,
                    self.progress,
                ))
            },
            |result| {
                result
                    .as_ref()
                    .map(|display_names| vec![perf_attr("display_name_count", display_names.len())])
                    .unwrap_or_default()
            },
        )?;

        Ok(LoadedStackStatusView {
            report,
            display_names,
        })
    }
}

struct LoadedStackStatusView {
    report: PullRequestStackStatusReport,
    display_names: BTreeMap<String, String>,
}

fn stack_status_trunk_attrs(trunk: Option<&RemoteStatusReport>) -> Vec<PerfAttr> {
    let mut attrs = vec![perf_attr("mode", "branch_head")];
    if let Some(trunk) = trunk {
        attrs.extend([
            perf_attr("state", trunk.comparison.label()),
            perf_attr("counts_exact", trunk.comparison.counts_exact),
            perf_attr("local_ahead_by", trunk.local_ahead_by),
            perf_attr("github_ahead_by", trunk.comparison.github_ahead_by),
            perf_attr("github_behind_by", trunk.comparison.github_behind_by),
        ]);
    }
    attrs
}

struct StackMoveExecution<'a> {
    environment: &'a RuntimeEnvironment,
    services: &'a dyn CommandServices,
    progress: &'a dyn ProgressSink,
    context: &'a RepositoryContext,
    manager: &'a PullRequestStackManager<'a>,
    output: OutputMode,
}

impl StackMoveExecution<'_> {
    fn run(&self, request: StackMoveRequest) -> Result<CommandResult, CommandError> {
        let old_selection = if request.no_sync {
            None
        } else {
            self.progress
                .status(stack_move_load_status(&request.revisions));
            self.manager.refresh_local_stack_metadata()?;
            Some(self.stack_move_sync_selection(&request.revisions)?)
        };

        self.progress.status(stack_move_status(&request.revisions));
        let outcome =
            self.services
                .move_stack(self.context, &request.revisions, &request.target)?;

        if request.no_sync {
            self.progress.status("Refreshing local stack metadata…");
            self.manager.refresh_local_stack_metadata()?;
            self.progress.finish();
            return Ok(CommandResult::success(render_stack_move(
                &outcome,
                &request.target,
                &request.revisions,
            )));
        }

        self.progress.status("Fetching origin…");
        let fetch = self.services.fetch_origin(self.context)?;
        self.progress.status("Refreshing local stack metadata…");
        self.manager.refresh_local_stack_metadata()?;
        self.progress.status("Selecting affected stack bookmarks…");
        let new_selection = if request.revisions.is_empty() {
            self.manager.sync_selection_for_selector(None)?
        } else {
            self.manager.sync_selection_for_branches(
                old_selection
                    .as_ref()
                    .map(|selection| selection.branches.as_slice())
                    .unwrap_or_default(),
            )?
        };
        let branches = affected_stack_branches(old_selection.as_ref(), &new_selection);
        self.progress.status("Pushing stack bookmarks…");
        let push = push_syncable_stack_branches(
            self.context,
            self.services,
            &branches,
            SyncPushOptions::default(),
        )?;
        self.progress.status("Syncing pull request descriptions…");
        let pull_requests = self
            .manager
            .sync_pull_requests_with_metadata(&push.pushed, &new_selection.metadata)?;
        self.progress.finish();

        let report = domain::sync_report(self.context, fetch, None, push, pull_requests);
        let mut stdout = render_stack_move(&outcome, &request.target, &request.revisions);
        stdout.push_str(&render_sync(
            &report,
            self.environment.current_dir(),
            self.output.color,
        )?);
        let exit_code = if stack_sync_report_has_conflicts(&report) {
            1
        } else {
            0
        };
        Ok(CommandResult::with_exit_code(stdout, exit_code))
    }

    fn stack_move_sync_selection(
        &self,
        revisions: &[String],
    ) -> Result<PullRequestStackSyncSelection, CommandError> {
        if revisions.is_empty() {
            self.manager.sync_selection_for_selector(None)
        } else {
            self.manager.sync_selection_for_selectors(revisions)
        }
    }
}

struct StackPublishExecution<'a> {
    environment: &'a RuntimeEnvironment,
    services: &'a dyn CommandServices,
    progress: &'a dyn ProgressSink,
    prompts: &'a PromptHandlers<'a>,
    context: &'a RepositoryContext,
    manager: &'a PullRequestStackManager<'a>,
    output: OutputMode,
    perf: &'a PerfLog,
}

impl StackPublishExecution<'_> {
    fn run(&self, request: StackPublishRequest) -> Result<CommandResult, CommandError> {
        let mut span = self.perf.start(
            "stack.publish",
            [
                perf_attr("repo", self.context.origin.github.slug()),
                perf_attr("revision_count", request.revisions.len()),
                perf_attr("explicit_revisions", !request.revisions.is_empty()),
                perf_attr("label_count", request.labels.len()),
                perf_attr("reviewer_arg_count", request.reviewers.len()),
                perf_attr("fixes_count", request.fixes.len()),
                perf_attr("fixes_attached", request.fixes_attached),
                perf_attr("ready_selector_count", request.ready.len()),
                perf_attr("draft_selector_count", request.draft.len()),
                perf_attr("event_handlers", !request.no_event_handlers),
            ],
        );
        let result = self.run_traced(request, &mut span);
        if let Err(error) = &result {
            span.record_error(error);
        }
        span.end();
        result
    }

    fn run_traced(
        &self,
        request: StackPublishRequest,
        span: &mut PerfSpan,
    ) -> Result<CommandResult, CommandError> {
        let task_id = match (request.task_id.clone(), request.no_task_id) {
            (Some(task_id), _) => Some(task_id),
            (None, true) => None,
            (None, false) if request.fixes.len() == 1 => request.fixes.first().cloned(),
            (None, false) => read_workspace_metadata(&self.context.workspace_root)?.task_id,
        };
        span.set([perf_attr("has_task_id", task_id.is_some())]);
        let publish_options = PullRequestPublishOptions {
            event_handlers: !request.no_event_handlers,
        };
        let selection = stack_publish_selection(&request.revisions, request.apply_to_stack);

        self.progress.status("Loading publish stack…");
        let (facts, prepare_effects) = span.measure(
            "load_publish_stack",
            [perf_attr("revision_count", request.revisions.len())],
            || self.prepare_stack(selection, task_id.as_deref(), publish_options, &request),
        )?;
        record_stack_publish_metrics(span, &facts.metrics);
        span.set([
            perf_attr("stack_node_count", facts.nodes.len()),
            perf_attr("publish_count", facts.publish_indexes.len()),
            perf_attr("prepare_effect_count", prepare_effects.len()),
        ]);

        let intent_indexes = stack_publish_intent_indexes(&facts, request.apply_to_stack);
        let fix_intent_indexes =
            stack_publish_fix_intent_indexes(&facts, &request, &intent_indexes);
        span.set([
            perf_attr("intent_count", intent_indexes.len()),
            perf_attr("fix_intent_count", fix_intent_indexes.len()),
        ]);
        let readiness = self.stack_readiness_by_index(&facts, &request, &intent_indexes, span)?;
        let confirmation_indexes = stack_publish_confirmation_indexes(&intent_indexes, &readiness);
        span.set([
            perf_attr("readiness_override_count", readiness.len()),
            perf_attr("confirmation_count", confirmation_indexes.len()),
        ]);

        self.progress.status("Planning pull requests…");
        let plan_step = span.start_step(
            "plan_pull_requests",
            [perf_attr("publish_count", facts.publish_indexes.len())],
        );
        let task_ids_by_index =
            stack_publish_task_ids_by_index(&intent_indexes, task_id.as_deref());
        let plans_result = self.plan_pull_requests(
            &facts,
            &task_ids_by_index,
            &intent_indexes,
            &request.labels,
            &readiness,
            span,
        );
        let plan_step_attrs = plans_result
            .as_ref()
            .map(|plans| vec![perf_attr("plan_count", plans.len())])
            .unwrap_or_default();
        span.finish_step(plan_step, plan_step_attrs, plans_result.as_ref().err());
        let mut plans = plans_result?;
        add_projected_stack_context_to_existing_plans(&mut plans);
        span.set([perf_attr("plan_count", plans.len())]);
        let status = span.measure("workspace_status", Vec::new(), || {
            self.services
                .workspace_status(self.environment.current_dir(), io::stderr().is_terminal())
        })?;
        self.progress.finish();

        let intent_plan_positions = stack_publish_intent_plan_positions(&facts, &intent_indexes);
        let confirmation_plan_positions =
            stack_publish_intent_plan_positions(&facts, &confirmation_indexes);

        let reviewer_candidates = stack_reviewer_candidates(&plans, &intent_plan_positions);
        let preselected_reviewers =
            stack_preselected_reviewers(&plans, &intent_plan_positions, &request.reviewers);
        let reviewers = span.measure(
            "reviewer_selection",
            [
                perf_attr("candidate_count", reviewer_candidates.len()),
                perf_attr("reviewer_arg_count", request.reviewers.len()),
                perf_attr("preselected_reviewer_count", preselected_reviewers.len()),
            ],
            || match self
                .prompts
                .reviewer_selector
                .select_reviewers(&reviewer_candidates, &preselected_reviewers)
            {
                Ok(reviewers) => Ok(Some(reviewers)),
                Err(ReviewerSelectionError::Cancelled) => Ok(None),
                Err(error) => Err(CommandError::from(error)),
            },
        )?;
        let Some(reviewers) = reviewers else {
            span.set([
                perf_attr("cancelled", true),
                perf_attr("cancel_stage", "reviewer_selection"),
            ]);
            return Ok(CommandResult::success("cancelled\n".to_owned()));
        };
        let intent_branches = stack_publish_intent_branches(&plans, &intent_plan_positions);
        let fix_intent_plan_positions =
            stack_publish_intent_plan_positions(&facts, &fix_intent_indexes);
        let fix_intent_branches = stack_publish_intent_branches(&plans, &fix_intent_plan_positions);
        for (position, plan) in plans.iter_mut().enumerate() {
            plan.reviewers = if intent_plan_positions.contains(&position) {
                reviewers.clone()
            } else {
                ReviewerSelection::default()
            };
        }

        let plan_selection = span.measure(
            "confirm_pull_requests",
            [
                perf_attr("plan_count", plans.len()),
                perf_attr("intent_plan_count", confirmation_plan_positions.len()),
            ],
            || {
                self.confirm_stack_publish_plans(
                    plans,
                    &confirmation_plan_positions,
                    &status,
                    &prepare_effects,
                )
            },
        )?;
        span.set([
            perf_attr("confirmed_plan_count", plan_selection.plans.len()),
            perf_attr(
                "context_only_pr_count",
                plan_selection.context_only_pull_requests.len(),
            ),
            perf_attr("skipped_plan_count", plan_selection.skipped_count),
        ]);
        let plans = plan_selection.plans;
        let context_only_pull_requests = plan_selection.context_only_pull_requests;
        if plans.is_empty() && context_only_pull_requests.is_empty() {
            span.set([
                perf_attr("cancelled", true),
                perf_attr("cancel_stage", "confirm_pull_requests"),
            ]);
            return Ok(CommandResult::success("cancelled\n".to_owned()));
        }

        let bookmark_targets = plans
            .iter()
            .map(|plan| (plan.bookmark.branch.clone(), plan.target_commit_id.clone()))
            .collect::<Vec<_>>();
        self.progress.status("Creating bookmarks…");
        let bookmark_updates = span.measure(
            "ensure_bookmarks",
            [perf_attr("bookmark_count", bookmark_targets.len())],
            || {
                self.services
                    .ensure_bookmarks(self.context, &bookmark_targets)
            },
        )?;
        let branches = plans
            .iter()
            .map(|plan| plan.bookmark.branch.clone())
            .collect::<Vec<_>>();
        self.progress.status("Pushing branches…");
        let push_step = span.start_step(
            "push_bookmarks",
            [perf_attr("branch_count", branches.len())],
        );
        let push_result = self
            .services
            .push_bookmarks_with_metrics(self.context, &branches);
        span.finish_step(
            push_step,
            push_bookmarks_result_attrs(&push_result),
            push_result.as_ref().err(),
        );
        let push_result = push_result?;
        record_push_bookmarks_metrics(span, &push_result.metrics);
        let pushes = push_result.outcomes;
        let metadata_only = stack_publish_metadata_only(&plans, &pushes);
        span.set([perf_attr("metadata_only", metadata_only)]);

        self.progress.status(if metadata_only {
            "Updating pull request metadata…"
        } else {
            "Publishing pull requests…"
        });
        let mut reports = Vec::new();
        let publish_step_name = if metadata_only {
            "publish_pull_request_metadata_only"
        } else {
            "publish_pull_request"
        };
        for ((plan, bookmark_update), push) in plans.into_iter().zip(bookmark_updates).zip(pushes) {
            let branch = plan.bookmark.branch.clone();
            let plan_publish_options = PullRequestPublishOptions {
                event_handlers: publish_options.event_handlers && intent_branches.contains(&branch),
            };
            reports.push(span.measure_with_result_attrs(
                publish_step_name,
                [perf_attr("branch", branch)],
                || {
                    if metadata_only {
                        self.services.publish_pull_request_metadata_only(
                            self.context,
                            plan,
                            bookmark_update,
                            push,
                        )
                    } else {
                        self.services.publish_pull_request(
                            self.context,
                            plan,
                            bookmark_update,
                            push,
                            plan_publish_options,
                        )
                    }
                },
                publish_pull_request_result_attrs,
            )?);
        }
        span.set([perf_attr("published_pr_count", reports.len())]);

        self.progress.status("Updating pull request stack…");
        let stack_update = span.measure(
            "update_stack",
            [
                perf_attr("report_count", reports.len()),
                perf_attr("context_only_pr_count", context_only_pull_requests.len()),
            ],
            || {
                self.manager.update_after_stack_publish_with_context_only(
                    &reports,
                    &context_only_pull_requests,
                )
            },
        )?;
        span.measure(
            "record_work_items",
            [
                perf_attr("report_count", reports.len()),
                perf_attr("fixes_count", request.fixes.len()),
                perf_attr("fixes_attached", request.fixes_attached),
            ],
            || {
                self.manager.record_published_work_items(
                    &reports,
                    &request.fixes,
                    request.fixes_attached,
                    &fix_intent_branches,
                )
            },
        )?;
        span.set([perf_attr(
            "stack_update_pr_count",
            stack_update.pull_requests.len(),
        )]);
        self.progress.finish();
        if reports.is_empty() && stack_update.is_empty() {
            return Ok(CommandResult::success("cancelled\n".to_owned()));
        }
        render_stack_publish(
            &self.context.origin.github.https_url(),
            &reports,
            &stack_update,
            self.services,
            self.output.color,
        )
        .map(CommandResult::success)
    }

    fn prepare_stack(
        &self,
        mut selection: StackPublishSelection,
        task_id: Option<&str>,
        publish_options: PullRequestPublishOptions,
        request: &StackPublishRequest,
    ) -> Result<(StackPublishFacts, Vec<PullRequestEventEffect>), CommandError> {
        let mut prepare_effects = Vec::new();
        loop {
            let facts = self
                .services
                .stack_publish_facts(self.context, &selection)?;
            let facts = non_empty_stack_publish_facts(facts)?;
            let intent_indexes = stack_publish_intent_indexes(&facts, request.apply_to_stack);
            let fix_intent_indexes =
                stack_publish_fix_intent_indexes(&facts, request, &intent_indexes);
            validate_stack_publish_intent(&facts, request, &intent_indexes)?;
            validate_stack_publish_fixes(
                &facts,
                &request.fixes,
                request.fixes_attached,
                task_id,
                &fix_intent_indexes,
            )?;
            run_repo_checks(
                self.context,
                self.services,
                RepoCheckTrigger::PullRequest,
                &Self::stack_publish_changed_files(&facts, &facts.publish_indexes),
            )?;
            let stable_selection = stable_stack_publish_selection(&selection, &facts);
            let mut rewrote = false;
            for index in &facts.publish_indexes {
                let workspace = &facts.nodes[*index].workspace;
                let prepare_report = domain::prepare_pull_request_change(
                    self.context,
                    workspace,
                    intent_indexes.contains(index).then_some(task_id).flatten(),
                    PullRequestPublishOptions {
                        event_handlers: publish_options.event_handlers
                            && intent_indexes.contains(index),
                    },
                );
                if prepare_report.changed {
                    self.services.rewrite_commit_description(
                        self.context,
                        &workspace.target_change.commit_id,
                        &prepare_report.description,
                    )?;
                    prepare_effects.extend(prepare_report.event_effects);
                    selection = stable_selection;
                    rewrote = true;
                    break;
                }
            }
            if rewrote {
                continue;
            }

            for index in &facts.publish_indexes {
                let workspace = &facts.nodes[*index].workspace;
                prepare_effects.extend(
                    domain::prepare_pull_request_change(
                        self.context,
                        workspace,
                        intent_indexes.contains(index).then_some(task_id).flatten(),
                        PullRequestPublishOptions {
                            event_handlers: publish_options.event_handlers
                                && intent_indexes.contains(index),
                        },
                    )
                    .event_effects,
                );
            }
            return Ok((facts, prepare_effects));
        }
    }

    fn stack_publish_changed_files(facts: &StackPublishFacts, indexes: &[usize]) -> Vec<String> {
        let mut changed_files = indexes
            .iter()
            .flat_map(|index| facts.nodes[*index].workspace.changed_files.clone())
            .collect::<Vec<_>>();
        changed_files.sort();
        changed_files.dedup();
        changed_files
    }

    fn plan_pull_requests(
        &self,
        facts: &StackPublishFacts,
        task_ids_by_index: &BTreeMap<usize, String>,
        intent_indexes: &BTreeSet<usize>,
        labels: &[String],
        readiness_by_index: &BTreeMap<usize, PullRequestReadiness>,
        span: &mut PerfSpan,
    ) -> Result<Vec<PullRequestPlan>, CommandError> {
        let mut plans_by_index = BTreeMap::new();
        let mut plans = Vec::new();
        for index in &facts.publish_indexes {
            let mut workspace = facts.nodes[*index].workspace.clone();
            workspace.nearest_ancestor_bookmark =
                stack_publish_base(facts, *index, &plans_by_index);
            let readiness = readiness_by_index.get(index).copied().unwrap_or_default();
            let plan_labels = if intent_indexes.contains(index) {
                labels.to_vec()
            } else {
                Vec::new()
            };
            let step = span.start_step(
                "pull_request_plan",
                [
                    perf_attr("stack_index", workspace.stack_index),
                    perf_attr("commit", &workspace.target_change.short_commit_id),
                    perf_attr("changed_file_count", workspace.changed_files.len()),
                    perf_attr(
                        "has_stack_base",
                        workspace.nearest_ancestor_bookmark.is_some(),
                    ),
                    perf_attr("label_count", plan_labels.len()),
                    perf_attr("readiness", readiness.label()),
                ],
            );
            let result = self
                .services
                .pull_request_plan(
                    self.context,
                    workspace,
                    task_ids_by_index.get(index).cloned(),
                    plan_labels,
                    readiness,
                )
                .map_err(CommandError::from);
            span.finish_step(
                step,
                pull_request_plan_result_attrs(&result),
                result.as_ref().err(),
            );
            let plan = result?;
            plans_by_index.insert(*index, plan.bookmark.branch.clone());
            plans.push(plan);
        }
        Ok(plans)
    }

    fn confirm_stack_publish_plans(
        &self,
        plans: Vec<PullRequestPlan>,
        intent_plan_positions: &BTreeSet<usize>,
        status: &WorkspaceStatus,
        prepare_effects: &[PullRequestEventEffect],
    ) -> Result<StackPublishPlanSelection, CommandError> {
        let mut selected = StackPublishPlanSelection::default();
        let mut blocked_branches = BTreeSet::new();
        for (position, plan) in plans.into_iter().enumerate() {
            if blocked_branches.contains(&plan.base) {
                blocked_branches.insert(plan.bookmark.branch);
                selected.skipped_count += 1;
                continue;
            }

            if !intent_plan_positions.contains(&position) {
                if plan.existing_pull_request.is_some() {
                    selected.plans.push(plan);
                } else {
                    blocked_branches.insert(plan.bookmark.branch);
                    selected.skipped_count += 1;
                }
                continue;
            }

            self.prompts
                .pull_request_previewer
                .show_preview(&plan, status, prepare_effects);
            if self
                .prompts
                .pull_request_confirmer
                .confirm_pull_request(&plan)?
            {
                selected.plans.push(plan);
            } else if let Some(existing) = plan.existing_pull_request {
                selected.context_only_pull_requests.push(existing);
                selected.skipped_count += 1;
            } else {
                blocked_branches.insert(plan.bookmark.branch);
                selected.skipped_count += 1;
            }
        }
        Ok(selected)
    }

    fn stack_readiness_by_index(
        &self,
        facts: &StackPublishFacts,
        request: &StackPublishRequest,
        intent_indexes: &BTreeSet<usize>,
        span: &mut PerfSpan,
    ) -> Result<BTreeMap<usize, PullRequestReadiness>, CommandError> {
        let ready_all = readiness_selects_all(&request.ready);
        let draft_all = readiness_selects_all(&request.draft);
        if ready_all && draft_all {
            return Err(stack_publish_readiness_usage_error(
                "--ready and --draft cannot both target the entire published stack",
            ));
        }

        let ready_indexes = self.resolve_readiness_indexes(facts, &request.ready, span)?;
        let draft_indexes = self.resolve_readiness_indexes(facts, &request.draft, span)?;
        let overlap = ready_indexes
            .intersection(&draft_indexes)
            .copied()
            .collect::<Vec<_>>();
        if !overlap.is_empty() {
            return Err(stack_publish_readiness_usage_error(format!(
                "--ready and --draft selectors overlap on {}",
                stack_publish_index_summary(facts, &overlap),
            )));
        }

        let mut readiness = BTreeMap::new();
        if ready_all {
            for index in intent_indexes {
                readiness.insert(*index, PullRequestReadiness::Ready);
            }
        } else if draft_all {
            for index in intent_indexes {
                readiness.insert(*index, PullRequestReadiness::Draft);
            }
        }
        for index in ready_indexes {
            readiness.insert(index, PullRequestReadiness::Ready);
        }
        for index in draft_indexes {
            readiness.insert(index, PullRequestReadiness::Draft);
        }
        Ok(readiness)
    }

    fn resolve_readiness_indexes(
        &self,
        facts: &StackPublishFacts,
        selectors: &[StackPublishReadinessSelector],
        span: &mut PerfSpan,
    ) -> Result<BTreeSet<usize>, CommandError> {
        let mut indexes = BTreeSet::new();
        for selector in selectors {
            let StackPublishReadinessSelector::Revisions(revisions) = selector else {
                continue;
            };
            let selected = self.resolve_readiness_revision_indexes(facts, revisions, span)?;
            indexes.extend(selected);
        }
        Ok(indexes)
    }

    fn resolve_readiness_revision_indexes(
        &self,
        facts: &StackPublishFacts,
        revisions: &str,
        span: &mut PerfSpan,
    ) -> Result<Vec<usize>, CommandError> {
        let selection = StackPublishSelection::ExplicitRevisions {
            revisions: vec![revisions.to_owned()],
        };
        let selector_facts = span.measure(
            "resolve_readiness_selector",
            [perf_attr("selector", revisions)],
            || self.services.stack_publish_facts(self.context, &selection),
        )?;
        let indexes_by_change = facts
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.workspace.target_change.change_id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        let published = facts
            .publish_indexes
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut indexes = Vec::new();
        let mut outside = Vec::new();

        for selector_index in selector_facts.publish_indexes {
            let change_id = selector_facts.nodes[selector_index]
                .workspace
                .target_change
                .change_id
                .as_str();
            match indexes_by_change.get(change_id).copied() {
                Some(index) if published.contains(&index) => indexes.push(index),
                Some(index) => outside.push(index),
                None => {
                    return Err(stack_publish_readiness_usage_error(format!(
                        "readiness selector `{revisions}` resolved outside the published stack"
                    )));
                }
            }
        }

        if !outside.is_empty() {
            return Err(stack_publish_readiness_usage_error(format!(
                "readiness selector `{revisions}` targeted unpublished revisions {}; add matching -r selectors or narrow the readiness selector",
                stack_publish_index_summary(facts, &outside),
            )));
        }
        Ok(indexes)
    }
}

fn stack_publish_metadata_only(plans: &[PullRequestPlan], pushes: &[PushOutcome]) -> bool {
    !plans.is_empty()
        && plans.len() == pushes.len()
        && plans
            .iter()
            .all(|plan| plan.existing_pull_request.is_some())
        && pushes.iter().all(stack_publish_push_is_no_op)
}

fn stack_publish_push_is_no_op(push: &PushOutcome) -> bool {
    push.pushed_refs == 0 && push.pushed_commits.is_empty()
}

pub(super) fn add_projected_stack_context_to_existing_plans(plans: &mut [PullRequestPlan]) {
    if plans.len() <= 1
        || plans
            .iter()
            .any(|plan| plan.existing_pull_request.is_none())
    {
        return;
    }

    let projected_pull_requests = plans
        .iter()
        .filter_map(projected_pull_request_for_plan)
        .collect::<Vec<_>>();
    let metadata =
        stack_metadata_from_pull_requests(&projected_pull_requests, &StackMetadata::default());
    for (plan, pull_request) in plans.iter_mut().zip(projected_pull_requests.iter()) {
        plan.body = domain::pull_request_body_with_stack_context(
            &plan.body,
            &metadata,
            pull_request,
            &plan.repository.github_url,
        );
    }
}

fn projected_pull_request_for_plan(plan: &PullRequestPlan) -> Option<PullRequestRecord> {
    let existing = plan.existing_pull_request.as_ref()?;
    Some(PullRequestRecord {
        number: existing.number,
        title: plan.title.clone(),
        body: Some(plan.body.clone()),
        head_branch: plan.bookmark.branch.clone(),
        base_branch: plan.base.clone(),
        html_url: existing.html_url.clone(),
        draft: plan.draft,
        merged: existing.merged,
        reviewers: existing.reviewers.clone(),
    })
}

fn non_empty_stack_publish_facts(
    mut facts: StackPublishFacts,
) -> Result<StackPublishFacts, CommandError> {
    facts.publish_indexes = facts
        .publish_indexes
        .iter()
        .copied()
        .filter(|index| stack_publish_node_has_changes(&facts.nodes[*index]))
        .collect();
    if facts.publish_indexes.is_empty() {
        return Err(WorkflowError::EmptyPullRequestChange.into());
    }
    facts.metrics.publish_count = facts.publish_indexes.len();
    Ok(facts)
}

fn stack_publish_selection(revisions: &[String], apply_to_stack: bool) -> StackPublishSelection {
    match (revisions, apply_to_stack) {
        ([], _) => StackPublishSelection::InferredStack { anchor: None },
        ([anchor], true) => StackPublishSelection::InferredStack {
            anchor: Some(anchor.clone()),
        },
        _ => StackPublishSelection::ExplicitRevisions {
            revisions: revisions.to_vec(),
        },
    }
}

fn readiness_selects_all(selectors: &[StackPublishReadinessSelector]) -> bool {
    selectors
        .iter()
        .any(|selector| matches!(selector, StackPublishReadinessSelector::All))
}

fn validate_stack_publish_intent(
    facts: &StackPublishFacts,
    request: &StackPublishRequest,
    intent_indexes: &BTreeSet<usize>,
) -> Result<(), CommandError> {
    if !intent_indexes.is_empty() {
        return Ok(());
    }

    let has_bare_readiness =
        readiness_selects_all(&request.ready) || readiness_selects_all(&request.draft);
    let has_explicit_intent = request.task_id.is_some()
        || !request.labels.is_empty()
        || !request.reviewers.is_empty()
        || !request.fixes.is_empty()
        || request.fixes_attached
        || has_bare_readiness;
    if has_explicit_intent {
        return Err(stack_publish_intent_usage_error(format!(
            "publish intent flags require a current commit or single selected revision; selected {}",
            stack_publish_index_summary(facts, &facts.publish_indexes),
        )));
    }

    Ok(())
}

fn validate_stack_publish_fixes(
    facts: &StackPublishFacts,
    fixes: &[String],
    fixes_attached: bool,
    task_id: Option<&str>,
    intent_indexes: &BTreeSet<usize>,
) -> Result<(), CommandError> {
    if fixes.is_empty() && !fixes_attached {
        return Ok(());
    }
    if intent_indexes.len() != 1 {
        return Err(stack_publish_fixes_usage_error(
            "--fixes requires a current commit or single selected revision",
        ));
    }
    let intent_index = *intent_indexes
        .iter()
        .next()
        .expect("length check ensures one intent index");
    if fixes_attached {
        let workspace = &facts.nodes[intent_index].workspace;
        if domain::pull_request_work_ids_from_description(
            &workspace.target_change.description,
            task_id,
            fixes,
        )
        .is_empty()
        {
            return Err(stack_publish_fixes_usage_error(
                "--fixes without WORK_ID requires a task id or ticket prefix in the selected commit title",
            ));
        }
    }

    Ok(())
}

fn stack_publish_readiness_usage_error(message: impl Into<String>) -> CommandError {
    CommandError::Usage(clap::Error::raw(ErrorKind::ValueValidation, message.into()))
}

fn stack_publish_intent_usage_error(message: impl Into<String>) -> CommandError {
    CommandError::Usage(clap::Error::raw(ErrorKind::ValueValidation, message.into()))
}

fn stack_publish_fixes_usage_error(message: impl Into<String>) -> CommandError {
    CommandError::Usage(clap::Error::raw(ErrorKind::ValueValidation, message.into()))
}

fn stack_publish_index_summary(facts: &StackPublishFacts, indexes: &[usize]) -> String {
    indexes
        .iter()
        .map(|index| {
            let change = &facts.nodes[*index].workspace.target_change;
            format!("{} ({})", change.short_commit_id, change.change_id)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn stack_plan_selection(revisions: &[String]) -> StackPlanSelection {
    if revisions.is_empty() {
        StackPlanSelection::InferredStack { anchor: None }
    } else {
        StackPlanSelection::ExplicitRevisions {
            revisions: revisions.to_vec(),
        }
    }
}

fn render_stack_plan(facts: &StackPlanFacts) -> Result<String, CommandError> {
    let selected = facts
        .selected_indexes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let root = facts
        .nodes
        .iter()
        .position(|node| node.parent_index.is_none())
        .unwrap_or(0);
    let mut output = format!(
        "Stack plan: {} commits, {} selected\nBase: {} @ {}\nRoot: {} {}\n\n",
        facts.nodes.len(),
        facts.selected_indexes.len(),
        facts.trunk.branch,
        facts.trunk.short_commit_id,
        facts.nodes[root].workspace.target_change.short_commit_id,
        stack_plan_title(&facts.nodes[root]),
    );
    for (prefix, index) in stack_plan_rows(facts) {
        let marker = if selected.contains(&index) {
            "◉"
        } else {
            "◯"
        };
        let role = if selected.contains(&index) {
            "selected"
        } else {
            "context"
        };
        let node = &facts.nodes[index];
        output.push_str(&format!(
            "{prefix}{marker} {} {}  {role}\n",
            node.workspace.target_change.short_commit_id,
            stack_plan_title(node),
        ));
    }
    output.push_str("\nSelected revisions share one stack root. Publish would create/update PRs for selected rows.\n");
    Ok(output)
}

fn stack_plan_title(node: &crate::jj::StackPlanNodeFacts) -> &str {
    node.workspace
        .target_change
        .description
        .lines()
        .find_map(|line| {
            let line = line.trim();
            (!line.is_empty()).then_some(line)
        })
        .unwrap_or("(no description)")
}

fn stack_plan_rows(facts: &StackPlanFacts) -> Vec<(String, usize)> {
    let mut children = vec![Vec::new(); facts.nodes.len()];
    let mut roots = Vec::new();
    for (index, node) in facts.nodes.iter().enumerate() {
        match node.parent_index {
            Some(parent) => children[parent].push(index),
            None => roots.push(index),
        }
    }

    let mut rows = Vec::new();
    append_stack_plan_rows(&mut rows, &children, &roots, "", 0);
    rows
}

fn append_stack_plan_rows(
    rows: &mut Vec<(String, usize)>,
    children: &[Vec<usize>],
    indexes: &[usize],
    prefix: &str,
    depth: usize,
) {
    for (position, index) in indexes.iter().copied().enumerate() {
        let last = position + 1 == indexes.len();
        let connector = if depth == 0 {
            String::new()
        } else if last {
            "└ ".to_owned()
        } else {
            "├ ".to_owned()
        };
        rows.push((format!("{prefix}{connector}"), index));
        let child_prefix = if depth == 0 {
            String::new()
        } else if last {
            format!("{prefix}  ")
        } else {
            format!("{prefix}│ ")
        };
        append_stack_plan_rows(rows, children, &children[index], &child_prefix, depth + 1);
    }
}

fn push_bookmarks_result_attrs(result: &Result<PushBookmarksOutcome, JjError>) -> Vec<PerfAttr> {
    match result {
        Ok(outcome) => push_bookmarks_metric_attrs(&outcome.metrics),
        Err(_) => Vec::new(),
    }
}

fn record_push_bookmarks_metrics(span: &mut PerfSpan, metrics: &PushBookmarksMetrics) {
    let attrs = push_bookmarks_metric_attrs(metrics);
    record_push_bookmarks_metric_step(
        span,
        "classify_updates",
        metrics.classify_updates_us,
        attrs.clone(),
    );
    record_push_bookmarks_metric_step(
        span,
        "pushed_commits_for_updates",
        metrics.pushed_commits_for_updates_us,
        attrs.clone(),
    );
    record_push_bookmarks_metric_step(
        span,
        "git_push_refs",
        metrics.git_push_refs_us,
        attrs.clone(),
    );
    record_push_bookmarks_metric_step(
        span,
        "commit_transaction",
        metrics.commit_transaction_us,
        attrs.clone(),
    );
    record_push_bookmarks_metric_step(span, "total", metrics.total_us, attrs);
}

fn push_bookmarks_metric_attrs(metrics: &PushBookmarksMetrics) -> Vec<PerfAttr> {
    vec![
        perf_attr("branch_count", metrics.branch_count),
        perf_attr("update_count", metrics.update_count),
        perf_attr("no_op_branch_count", metrics.no_op_branch_count),
        perf_attr("pushed_ref_count", metrics.pushed_ref_count),
        perf_attr("pushed_commit_count", metrics.pushed_commit_count),
        perf_attr("jj_total_us", metrics.total_us),
    ]
}

fn record_push_bookmarks_metric_step(
    span: &mut PerfSpan,
    phase: &str,
    duration_us: u64,
    attrs: Vec<PerfAttr>,
) {
    span.record_step_us(
        format!("push_bookmarks.{phase}"),
        duration_us,
        attrs,
        None::<&CommandError>,
    );
}

fn pull_request_plan_result_attrs(result: &Result<PullRequestPlan, CommandError>) -> Vec<PerfAttr> {
    match result {
        Ok(plan) => vec![
            perf_attr("branch", &plan.bookmark.branch),
            perf_attr("base", &plan.base),
            perf_attr("existing_pr", plan.existing_pull_request.is_some()),
            perf_attr("base_pr", plan.base_pull_request.is_some()),
            perf_attr("reviewer_candidate_count", plan.reviewer_candidates.len()),
        ],
        Err(_) => Vec::new(),
    }
}

fn publish_pull_request_result_attrs(
    result: &Result<PullRequestReport, WorkflowError>,
) -> Vec<PerfAttr> {
    match result {
        Ok(report) => vec![
            perf_attr("action", pull_request_action_name(report.action)),
            perf_attr("number", report.pull_request.number),
            perf_attr("base", &report.base),
            perf_attr(
                "label_count",
                report
                    .labels
                    .as_ref()
                    .map_or(0, |labels| labels.labels.len()),
            ),
            perf_attr("reviewer_synced", report.reviewers.is_some()),
            perf_attr("event_effect_count", report.event_effects.len()),
        ],
        Err(_) => Vec::new(),
    }
}

fn pull_request_action_name(action: PullRequestAction) -> &'static str {
    match action {
        PullRequestAction::Created => "created",
        PullRequestAction::Updated => "updated",
    }
}

fn record_stack_publish_metrics(span: &mut PerfSpan, metrics: &StackPublishMetrics) {
    let attrs = stack_publish_metric_attrs(metrics);
    record_stack_publish_metric_step(
        span,
        "target_resolution",
        metrics.target_resolution_us,
        attrs.clone(),
    );
    record_stack_publish_metric_step(
        span,
        "resolve_revisions",
        metrics.resolve_revisions_us,
        attrs.clone(),
    );
    record_stack_publish_metric_step(
        span,
        "resolve_trunk",
        metrics.resolve_trunk_us,
        attrs.clone(),
    );
    record_stack_publish_metric_step(
        span,
        "linear_stack_path",
        metrics.linear_stack_path_us,
        attrs.clone(),
    );
    record_stack_publish_metric_step(
        span,
        "collect_child_ids",
        metrics.collect_child_ids_us,
        attrs.clone(),
    );
    record_stack_publish_metric_step(
        span,
        "load_child_commit",
        metrics.load_child_commit_us,
        attrs.clone(),
    );
    record_stack_publish_metric_step(
        span,
        "workspace_facts",
        metrics.workspace_facts_us,
        attrs.clone(),
    );
    record_stack_publish_metric_step(span, "total", metrics.total_us, attrs);
}

fn stack_publish_metric_attrs(metrics: &StackPublishMetrics) -> Vec<PerfAttr> {
    vec![
        perf_attr("target_resolution_count", metrics.target_resolution_count),
        perf_attr("resolved_revision_count", metrics.resolved_revision_count),
        perf_attr("resolved_trunk_count", metrics.resolved_trunk_count),
        perf_attr("stack_path_count", metrics.stack_path_count),
        perf_attr("collected_child_count", metrics.collected_child_count),
        perf_attr("loaded_child_count", metrics.loaded_child_count),
        perf_attr("workspace_fact_count", metrics.workspace_fact_count),
        perf_attr("node_count", metrics.node_count),
        perf_attr("publish_count", metrics.publish_count),
        perf_attr("jj_total_us", metrics.total_us),
    ]
}

fn record_stack_publish_metric_step(
    span: &mut PerfSpan,
    phase: &str,
    duration_us: u64,
    attrs: Vec<PerfAttr>,
) {
    span.record_step_us(
        format!("stack_publish_facts.{phase}"),
        duration_us,
        attrs,
        None::<&CommandError>,
    );
}

fn stack_publish_intent_indexes(
    facts: &StackPublishFacts,
    apply_to_stack: bool,
) -> BTreeSet<usize> {
    if apply_to_stack {
        return facts.publish_indexes.iter().copied().collect();
    }
    if let Some(anchor_index) = facts
        .anchor_index
        .filter(|index| facts.publish_indexes.contains(index))
    {
        return BTreeSet::from([anchor_index]);
    }
    match facts.publish_indexes.as_slice() {
        [index] => BTreeSet::from([*index]),
        _ => BTreeSet::new(),
    }
}

fn stack_publish_fix_intent_indexes(
    facts: &StackPublishFacts,
    request: &StackPublishRequest,
    intent_indexes: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    if request.fixes.is_empty() && !request.fixes_attached {
        return BTreeSet::new();
    }
    if request.apply_to_stack {
        return facts.publish_indexes.last().copied().into_iter().collect();
    }
    intent_indexes.clone()
}

fn stack_publish_confirmation_indexes(
    intent_indexes: &BTreeSet<usize>,
    readiness: &BTreeMap<usize, PullRequestReadiness>,
) -> BTreeSet<usize> {
    intent_indexes
        .iter()
        .copied()
        .chain(readiness.keys().copied())
        .collect()
}

fn stack_publish_task_ids_by_index(
    intent_indexes: &BTreeSet<usize>,
    task_id: Option<&str>,
) -> BTreeMap<usize, String> {
    let Some(task_id) = task_id else {
        return BTreeMap::new();
    };
    intent_indexes
        .iter()
        .map(|index| (*index, task_id.to_owned()))
        .collect()
}

fn stack_publish_intent_plan_positions(
    facts: &StackPublishFacts,
    intent_indexes: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    facts
        .publish_indexes
        .iter()
        .enumerate()
        .filter_map(|(position, index)| intent_indexes.contains(index).then_some(position))
        .collect()
}

fn stack_publish_intent_branches(
    plans: &[PullRequestPlan],
    intent_plan_positions: &BTreeSet<usize>,
) -> BTreeSet<String> {
    plans
        .iter()
        .enumerate()
        .filter(|(position, _)| intent_plan_positions.contains(position))
        .map(|(_, plan)| plan.bookmark.branch.clone())
        .collect()
}

fn stable_stack_publish_selection(
    selection: &StackPublishSelection,
    facts: &StackPublishFacts,
) -> StackPublishSelection {
    match selection {
        StackPublishSelection::InferredStack { .. } => StackPublishSelection::InferredStack {
            anchor: facts
                .anchor_index
                .map(|index| facts.nodes[index].workspace.target_change.change_id.clone()),
        },
        StackPublishSelection::ExplicitRevisions { .. } => {
            StackPublishSelection::ExplicitRevisions {
                revisions: facts
                    .publish_indexes
                    .iter()
                    .map(|index| {
                        facts.nodes[*index]
                            .workspace
                            .target_change
                            .change_id
                            .clone()
                    })
                    .collect(),
            }
        }
    }
}

fn stack_publish_base(
    facts: &StackPublishFacts,
    index: usize,
    planned_branches: &BTreeMap<usize, String>,
) -> Option<String> {
    let mut skipped_empty_bookmarks = BTreeSet::new();
    let mut parent = facts.nodes[index].parent_index;
    while let Some(parent_index) = parent {
        if let Some(branch) = planned_branches.get(&parent_index) {
            return Some(branch.clone());
        }
        let parent_node = &facts.nodes[parent_index];
        if stack_publish_node_has_changes(parent_node) {
            if let Some(branch) = parent_node.workspace.local_bookmarks_at_target.first() {
                return Some(branch.clone());
            }
        } else {
            skipped_empty_bookmarks.extend(parent_node.workspace.local_bookmarks_at_target.clone());
        }
        parent = parent_node.parent_index;
    }
    facts.nodes[index]
        .workspace
        .nearest_ancestor_bookmark
        .clone()
        .filter(|bookmark| !skipped_empty_bookmarks.contains(bookmark))
}

fn stack_publish_node_has_changes(node: &StackPublishNodeFacts) -> bool {
    !node.workspace.target_change.is_empty && !node.workspace.changed_files.is_empty()
}

fn stack_reviewer_candidates(
    plans: &[PullRequestPlan],
    intent_plan_positions: &BTreeSet<usize>,
) -> Vec<ReviewerCandidate> {
    let mut candidates: Vec<ReviewerCandidate> = Vec::new();
    for candidate in plans
        .iter()
        .enumerate()
        .filter(|(position, _)| intent_plan_positions.contains(position))
        .flat_map(|(_, plan)| plan.reviewer_candidates.iter().cloned())
    {
        if let Some(existing) = candidates
            .iter_mut()
            .find(|existing| existing.target.matches_identity(&candidate.target))
        {
            for reason in candidate.reasons {
                if !existing.reasons.contains(&reason) {
                    existing.reasons.push(reason);
                }
            }
        } else {
            candidates.push(candidate);
        }
    }
    candidates
}

fn stack_preselected_reviewers(
    plans: &[PullRequestPlan],
    intent_plan_positions: &BTreeSet<usize>,
    cli_reviewers: &[ReviewerTarget],
) -> Vec<ReviewerTarget> {
    let mut reviewers = Vec::new();
    for reviewer in cli_reviewers {
        push_reviewer_target(&mut reviewers, reviewer.clone());
    }
    for plan in plans
        .iter()
        .enumerate()
        .filter(|(position, _)| intent_plan_positions.contains(position))
        .map(|(_, plan)| plan)
    {
        if let Some(existing) = &plan.existing_pull_request {
            for user in &existing.reviewers.users {
                push_reviewer_target(&mut reviewers, ReviewerTarget::user(user.clone()));
            }
            for team in &existing.reviewers.teams {
                push_reviewer_target(
                    &mut reviewers,
                    ReviewerTarget::team(team.clone(), team.clone()),
                );
            }
        }
        for candidate in &plan.reviewer_candidates {
            if reviewer_candidate_keeps_existing_review_selection(candidate) {
                push_reviewer_target(&mut reviewers, candidate.target.clone());
            }
        }
    }
    reviewers
}

/// Returns whether prior PR activity should keep a reviewer checked after GitHub clears a request.
fn reviewer_candidate_keeps_existing_review_selection(candidate: &ReviewerCandidate) -> bool {
    candidate.reasons.iter().any(|reason| {
        matches!(
            reason.as_str(),
            "already requested" | "already approved" | "commented" | "comments addressed"
        )
    })
}

fn push_reviewer_target(reviewers: &mut Vec<ReviewerTarget>, reviewer: ReviewerTarget) {
    if !reviewers
        .iter()
        .any(|existing| existing.matches_identity(&reviewer))
    {
        reviewers.push(reviewer);
    }
}

fn render_stack_publish(
    repository_url: &str,
    reports: &[PullRequestReport],
    stack_update: &PullRequestStackPublishUpdate,
    services: &dyn CommandServices,
    color: bool,
) -> Result<String, CommandError> {
    match reports {
        [report] => render_pull_request_with_effects(report, stack_update, services, color),
        reports => {
            let mut output = String::new();
            for report in reports {
                output.push_str(&render_pull_request(report));
                append_pull_request_event_effects(&mut output, report, services, color);
            }
            append_stack_update(&mut output, repository_url, stack_update, color);
            Ok(output)
        }
    }
}

fn append_stack_update(
    output: &mut String,
    repository_url: &str,
    stack_update: &PullRequestStackPublishUpdate,
    color: bool,
) {
    if stack_update.is_empty() {
        return;
    }
    let pull_requests = stack_update
        .pull_requests
        .iter()
        .map(|pull_request| linked_pull_request_text(repository_url, pull_request))
        .collect::<Vec<_>>()
        .join(", ");
    let line = format!("Stack: refreshed stack context on {pull_requests}");
    output.push_str(&style_log_line(&line, color));
    output.push('\n');
}

fn append_pull_request_event_effects(
    output: &mut String,
    report: &PullRequestReport,
    services: &dyn CommandServices,
    color: bool,
) {
    let pull_request =
        linked_pull_request_text(&report.repository.github_url, &report.pull_request);
    for effect in &report.event_effects {
        if !pull_request_event_effect_is_default_visible(effect) {
            continue;
        }

        let summary = match &effect.kind {
            PullRequestEventEffectKind::AddLabels { labels } => added_labels_summary(labels),
            PullRequestEventEffectKind::LabelsAlreadyPresent { .. } => continue,
            PullRequestEventEffectKind::OpenPullRequest { url } => match services.open_url(url) {
                Ok(()) => format!("opened {pull_request}"),
                Err(error) => format!("could not open {pull_request}: {error}"),
            },
            PullRequestEventEffectKind::TitleAlready { .. } => continue,
            PullRequestEventEffectKind::UpdatedTitle { .. } => {
                "added task ID to the title".to_owned()
            }
        };
        let line = format!(
            "Event[{}]: {summary}",
            pull_request_event_display_name(effect)
        );
        output.push_str(&style_log_line(&line, color));
        output.push('\n');
    }
}

fn affected_stack_branches(
    old_selection: Option<&PullRequestStackSyncSelection>,
    new_selection: &PullRequestStackSyncSelection,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut branches = Vec::new();
    for branch in old_selection
        .into_iter()
        .flat_map(|selection| selection.branches.iter())
        .chain(new_selection.branches.iter())
    {
        if seen.insert(branch.clone()) {
            branches.push(branch.clone());
        }
    }
    branches
}

fn stack_sync_report_has_conflicts(report: &SyncReport) -> bool {
    report
        .fetch
        .rebased_commits
        .iter()
        .any(|commit| commit.has_conflict)
        || !report.skipped_conflicted_bookmarks.is_empty()
}

fn render_stack_move(
    outcome: &StackMoveOutcome,
    target: &StackMoveTarget,
    revisions: &[String],
) -> String {
    let target_label = match target {
        StackMoveTarget::Onto(target) => target.as_str(),
        StackMoveTarget::Trunk => "trunk",
    };
    let subject = stack_move_subject(revisions, outcome);
    if outcome.source_short_commit_ids.is_empty() {
        return format!("No {subject} to move onto {target_label}\n");
    }
    if outcome.rebased_commits == 0 {
        format!("Stack unchanged: {subject} is already on {target_label}\n")
    } else {
        format!(
            "Moved {subject} from {} onto {target_label}\n",
            stack_move_source_label(&outcome.source_short_commit_ids)
        )
    }
}

fn stack_move_load_status(revisions: &[String]) -> &'static str {
    if revisions.is_empty() {
        "Loading current stack…"
    } else {
        "Loading selected revisions…"
    }
}

fn stack_move_status(revisions: &[String]) -> &'static str {
    if revisions.is_empty() {
        "Moving current stack…"
    } else {
        "Moving selected revisions…"
    }
}

fn stack_move_subject(revisions: &[String], outcome: &StackMoveOutcome) -> &'static str {
    if revisions.is_empty() {
        "current stack"
    } else if revisions.len() == 1 && outcome.source_short_commit_ids.len() == 1 {
        "selected revision"
    } else {
        "selected revisions"
    }
}

fn stack_move_source_label(short_commit_ids: &[String]) -> String {
    match short_commit_ids {
        [] => String::new(),
        [short_commit_id] => short_commit_id.clone(),
        [first, rest @ ..] => format!("{first} and {} more", rest.len()),
    }
}

fn open_stack_pull_request(
    context: &RepositoryContext,
    services: &dyn CommandServices,
    selector: &dyn PullRequestSelector,
    snapshot: &PullRequestStackSnapshot,
    print: bool,
) -> Result<String, CommandError> {
    let choices = pull_request_choice_rows(snapshot);
    if choices.is_empty() {
        return Err(WorkflowError::MissingLocalBookmarkPullRequests {
            repository: context.origin.github.slug(),
        }
        .into());
    }

    let selected = match selector.select_pull_request(&choices) {
        Ok(selected) => selected,
        Err(PullRequestSelectionError::Cancelled) => return Ok("cancelled\n".to_owned()),
        Err(error) => return Err(error.into()),
    };
    let url = pull_request_url(&context.origin.github.https_url(), &selected);
    if print {
        return Ok(format!("{url}\n"));
    }

    services.open_url(&url)?;
    Ok(format!("Opened: {url}\n"))
}

fn render_stack_status_output(
    report: &PullRequestStackStatusReport,
    current_dir: &Path,
    color: bool,
    terminal_width: Option<usize>,
    format: StackStatusFormat,
    display_names: &BTreeMap<String, String>,
) -> Result<String, CommandError> {
    match format {
        StackStatusFormat::Human => {
            render_stack_status(report, current_dir, color, terminal_width, display_names)
                .map_err(Into::into)
        }
        StackStatusFormat::Json => Ok(render_stack_status_json(&[
            GlobalStackStatusEntry::current(current_dir.to_path_buf(), report),
        ])),
    }
}

fn render_global_stack_status_output(
    entries: &[GlobalStackStatusEntry],
    total_repositories: usize,
    current_dir: &Path,
    color: bool,
    terminal_width: Option<usize>,
    format: StackStatusFormat,
    display_names: &BTreeMap<String, String>,
) -> Result<String, CommandError> {
    match format {
        StackStatusFormat::Human => render_global_stack_status(
            entries,
            total_repositories,
            current_dir,
            color,
            terminal_width,
            display_names,
        )
        .map_err(Into::into),
        StackStatusFormat::Json => Ok(render_stack_status_json(entries)),
    }
}

fn stack_status_display_names(
    services: &dyn CommandServices,
    token_source: &TokenSource,
    logins: Vec<String>,
    format: StackStatusFormat,
    progress: &dyn ProgressSink,
) -> BTreeMap<String, String> {
    if format != StackStatusFormat::Human || logins.is_empty() {
        return BTreeMap::new();
    }
    progress.status("Loading reviewer names…");
    services.github_user_display_names(token_source, &logins)
}

fn stack_status_entry_user_logins(entries: &[GlobalStackStatusEntry]) -> Vec<String> {
    pull_request_status_user_logins(
        entries
            .iter()
            .filter_map(|entry| entry.result.as_ref().ok())
            .flat_map(|report| report.statuses.values()),
    )
}

fn stack_status_pull_request_numbers(
    snapshot: &PullRequestStackSnapshot,
    discovered_pull_requests: &[PullRequestRecord],
) -> Vec<u64> {
    let mut seen = BTreeSet::new();
    snapshot
        .nodes
        .iter()
        .filter_map(PullRequestStackNode::pull_request_number)
        .chain(
            discovered_pull_requests
                .iter()
                .map(|pull_request| pull_request.number),
        )
        .filter(|number| seen.insert(*number))
        .collect()
}

fn stack_status_missing_pull_requests(
    services: &dyn CommandServices,
    context: &RepositoryContext,
    snapshot: &PullRequestStackSnapshot,
) -> Result<Vec<PullRequestRecord>, WorkflowError> {
    let mut seen = BTreeSet::new();
    let mut pull_requests = Vec::new();
    for branch in snapshot
        .nodes
        .iter()
        .filter(|node| node.pull_request_number().is_none())
        .map(|node| node.branch.as_str())
        .filter(|branch| seen.insert((*branch).to_owned()))
    {
        if let Some(pull_request) = services.find_pull_request_for_head(context, branch)? {
            pull_requests.push(pull_request);
        }
    }
    Ok(pull_requests)
}

fn stack_snapshot_result_attrs(
    result: &Result<PullRequestStackSnapshot, CommandError>,
) -> Vec<PerfAttr> {
    result
        .as_ref()
        .map(|snapshot| {
            vec![
                perf_attr("stack_node_count", snapshot.nodes.len()),
                perf_attr(
                    "pr_count",
                    stack_status_pull_request_numbers(snapshot, &[]).len(),
                ),
            ]
        })
        .unwrap_or_default()
}

fn pull_request_discovery_result_attrs(
    result: &Result<Vec<PullRequestRecord>, WorkflowError>,
) -> Vec<PerfAttr> {
    result
        .as_ref()
        .map(|pull_requests| vec![perf_attr("discovered_pr_count", pull_requests.len())])
        .unwrap_or_default()
}

fn stack_status_format_label(format: StackStatusFormat) -> &'static str {
    match format {
        StackStatusFormat::Human => "human",
        StackStatusFormat::Json => "json",
    }
}

fn render_stack_snapshot(
    snapshot: &PullRequestStackSnapshot,
    color: bool,
) -> Result<String, CommandError> {
    if snapshot.nodes.is_empty() {
        return Ok("No stack state\n".to_owned());
    }

    let mut output = String::new();
    for row in stack_snapshot_rows(snapshot, color) {
        output.push_str(&row);
        output.push('\n');
    }
    Ok(output)
}

fn stack_snapshot_rows(snapshot: &PullRequestStackSnapshot, color: bool) -> Vec<String> {
    snapshot
        .rows()
        .into_iter()
        .map(|row| render_stack_row_label(row, color))
        .collect()
}

#[cfg(test)]
pub(super) fn stack_metadata_rows(nodes: &[StackMetadataNode]) -> Vec<String> {
    let metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: nodes.to_vec(),
    };
    let snapshot = PullRequestStackSnapshot::from_metadata(
        &metadata,
        &[],
        &[],
        PullRequestStackSelection::default(),
    );
    stack_snapshot_rows(&snapshot, false)
}
