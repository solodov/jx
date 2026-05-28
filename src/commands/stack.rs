use super::*;

#[cfg(test)]
use crate::repository::StackMetadataNode;

/// Handles repo-local pull-request stack state that survives bookmark movement.
pub(super) fn handle_stack(
    request: StackRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    selector: &dyn PullRequestSelector,
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
                &context, services, selector, &snapshot?, print,
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
