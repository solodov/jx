use super::*;

#[cfg(test)]
use crate::repository::StackMetadataNode;

/// Handles repo-local pull-request stack state that survives bookmark movement.
pub(super) fn handle_stack(
    request: StackRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    let manager = PullRequestStackManager::new(&context, services);
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
        StackRequest::Move { target, no_sync } => StackMoveExecution {
            environment,
            services,
            progress,
            context: &context,
            manager: &manager,
            output,
        }
        .run(target, no_sync),
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
        }
        .run(request),
    }
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
    fn run(&self, target: StackMoveTarget, no_sync: bool) -> Result<CommandResult, CommandError> {
        let old_selection = if no_sync {
            None
        } else {
            self.progress.status("Loading current stack…");
            self.manager.refresh_local_stack_metadata()?;
            Some(self.manager.sync_selection_for_selector(None)?)
        };

        self.progress.status("Moving current stack…");
        let outcome = self.services.move_current_stack(self.context, &target)?;

        if no_sync {
            self.progress.status("Refreshing local stack metadata…");
            self.manager.refresh_local_stack_metadata()?;
            self.progress.finish();
            return Ok(CommandResult::success(render_stack_move(
                &outcome, &target, false,
            )));
        }

        self.progress.status("Fetching origin…");
        let fetch = self.services.fetch_origin(self.context)?;
        self.progress.status("Refreshing local stack metadata…");
        self.manager.refresh_local_stack_metadata()?;
        self.progress.status("Selecting affected stack bookmarks…");
        let new_selection = self.manager.sync_selection_for_selector(None)?;
        let branches = affected_stack_branches(old_selection.as_ref(), &new_selection);
        self.progress.status("Pushing stack bookmarks…");
        let push = push_syncable_stack_branches(self.context, self.services, &branches)?;
        self.progress.status("Syncing pull request descriptions…");
        let pull_requests = self
            .manager
            .sync_pull_requests_with_metadata(&push.pushed, &new_selection.metadata)?;
        self.progress.finish();

        let report = domain::sync_report(self.context, fetch, push, pull_requests);
        let mut stdout = render_stack_move(&outcome, &target, true);
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
}

struct StackPublishExecution<'a> {
    environment: &'a RuntimeEnvironment,
    services: &'a dyn CommandServices,
    progress: &'a dyn ProgressSink,
    prompts: &'a PromptHandlers<'a>,
    context: &'a RepositoryContext,
    manager: &'a PullRequestStackManager<'a>,
    output: OutputMode,
}

impl StackPublishExecution<'_> {
    fn run(&self, request: StackPublishRequest) -> Result<CommandResult, CommandError> {
        let task_id = match (request.task_id, request.no_task_id) {
            (Some(task_id), _) => Some(task_id),
            (None, true) => None,
            (None, false) => read_workspace_metadata(&self.context.workspace_root)?.task_id,
        };
        let publish_options = PullRequestPublishOptions {
            event_handlers: !request.no_event_handlers,
        };
        let selection = stack_publish_selection(&request.revisions);

        self.progress.status("Loading publish stack…");
        let (facts, prepare_effects) =
            self.prepare_stack(selection, task_id.as_deref(), publish_options)?;
        self.progress.status("Planning pull requests…");
        let mut plans = self.plan_pull_requests(&facts, task_id, request.labels, request.draft)?;
        let status = self
            .services
            .workspace_status(self.environment.current_dir(), io::stderr().is_terminal())?;
        self.progress.finish();

        for plan in &plans {
            self.prompts
                .pull_request_previewer
                .show_preview(plan, &status, &prepare_effects);
        }

        let reviewer_candidates = stack_reviewer_candidates(&plans);
        let reviewers = match self
            .prompts
            .reviewer_selector
            .select_reviewers(&reviewer_candidates, &request.reviewers)
        {
            Ok(reviewers) => reviewers,
            Err(ReviewerSelectionError::Cancelled) => {
                return Ok(CommandResult::success("cancelled\n".to_owned()));
            }
            Err(error) => return Err(error.into()),
        };
        for plan in &mut plans {
            plan.reviewers = reviewers.clone();
        }

        for plan in &plans {
            if !self
                .prompts
                .pull_request_confirmer
                .confirm_pull_request(plan)?
            {
                return Ok(CommandResult::success("cancelled\n".to_owned()));
            }
        }

        let bookmark_targets = plans
            .iter()
            .map(|plan| (plan.bookmark.branch.clone(), plan.target_commit_id.clone()))
            .collect::<Vec<_>>();
        self.progress.status("Creating bookmarks…");
        let bookmark_updates = self
            .services
            .ensure_bookmarks(self.context, &bookmark_targets)?;
        let branches = plans
            .iter()
            .map(|plan| plan.bookmark.branch.clone())
            .collect::<Vec<_>>();
        self.progress.status("Pushing branches…");
        let pushes = self.services.push_bookmarks(self.context, &branches)?;

        self.progress.status("Publishing pull requests…");
        let mut reports = Vec::new();
        for ((plan, bookmark_update), push) in plans.into_iter().zip(bookmark_updates).zip(pushes) {
            reports.push(self.services.publish_pull_request(
                self.context,
                plan,
                bookmark_update,
                push,
                publish_options,
            )?);
        }

        self.progress.status("Updating pull request stack…");
        let stack_update = self.manager.update_after_stack_publish(&reports)?;
        self.progress.finish();
        render_stack_publish(&reports, &stack_update, self.services, self.output.color)
            .map(CommandResult::success)
    }

    fn prepare_stack(
        &self,
        mut selection: StackPublishSelection,
        task_id: Option<&str>,
        publish_options: PullRequestPublishOptions,
    ) -> Result<(StackPublishFacts, Vec<PullRequestEventEffect>), CommandError> {
        let mut prepare_effects = Vec::new();
        loop {
            let facts = self
                .services
                .stack_publish_facts(self.context, &selection)?;
            let stable_selection = stable_stack_publish_selection(&selection, &facts);
            let mut rewrote = false;
            for index in &facts.publish_indexes {
                let workspace = &facts.nodes[*index].workspace;
                let prepare_report = domain::prepare_pull_request_change(
                    self.context,
                    workspace,
                    task_id,
                    publish_options,
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
                        task_id,
                        publish_options,
                    )
                    .event_effects,
                );
            }
            return Ok((facts, prepare_effects));
        }
    }

    fn plan_pull_requests(
        &self,
        facts: &StackPublishFacts,
        task_id: Option<String>,
        labels: Vec<String>,
        draft: bool,
    ) -> Result<Vec<PullRequestPlan>, CommandError> {
        let mut plans_by_index = BTreeMap::new();
        let mut plans = Vec::new();
        for index in &facts.publish_indexes {
            let mut workspace = facts.nodes[*index].workspace.clone();
            workspace.nearest_ancestor_bookmark =
                stack_publish_base(facts, *index, &plans_by_index);
            let plan = self.services.pull_request_plan(
                self.context,
                workspace,
                task_id.clone(),
                labels.clone(),
                draft,
            )?;
            plans_by_index.insert(*index, plan.bookmark.branch.clone());
            plans.push(plan);
        }
        Ok(plans)
    }
}

fn stack_publish_selection(revisions: &[String]) -> StackPublishSelection {
    if revisions.is_empty() {
        StackPublishSelection::InferredStack { anchor: None }
    } else {
        StackPublishSelection::ExplicitRevisions {
            revisions: revisions.to_vec(),
        }
    }
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
            "└─ ".to_owned()
        } else {
            "├─ ".to_owned()
        };
        rows.push((format!("{prefix}{connector}"), index));
        let child_prefix = if depth == 0 {
            String::new()
        } else if last {
            format!("{prefix}   ")
        } else {
            format!("{prefix}│  ")
        };
        append_stack_plan_rows(rows, children, &children[index], &child_prefix, depth + 1);
    }
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
    let mut parent = facts.nodes[index].parent_index;
    while let Some(parent_index) = parent {
        if let Some(branch) = planned_branches.get(&parent_index) {
            return Some(branch.clone());
        }
        parent = facts.nodes[parent_index].parent_index;
    }
    facts.nodes[index]
        .workspace
        .nearest_ancestor_bookmark
        .clone()
}

fn stack_reviewer_candidates(plans: &[PullRequestPlan]) -> Vec<ReviewerCandidate> {
    let mut candidates = Vec::new();
    for candidate in plans
        .iter()
        .flat_map(|plan| plan.reviewer_candidates.iter().cloned())
    {
        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn render_stack_publish(
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
            append_stack_update(&mut output, reports, stack_update, color);
            Ok(output)
        }
    }
}

fn append_stack_update(
    output: &mut String,
    reports: &[PullRequestReport],
    stack_update: &PullRequestStackPublishUpdate,
    color: bool,
) {
    if stack_update.is_empty() {
        return;
    }
    let Some(repository_url) = reports.first().map(|report| &report.repository.github_url) else {
        return;
    };
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

fn render_stack_move(outcome: &StackMoveOutcome, target: &StackMoveTarget, synced: bool) -> String {
    let target_label = match target {
        StackMoveTarget::Onto(target) => target.as_str(),
        StackMoveTarget::Trunk => "trunk",
    };
    let mut output = if outcome.rebased_commits == 0 {
        format!("Stack unchanged: current stack is already on {target_label}\n")
    } else {
        format!(
            "Moved current stack from {} onto {target_label}\n",
            outcome.source_short_commit_id
        )
    };
    if !synced {
        output.push_str("Sync skipped (--no-sync)\n");
    }
    output
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
