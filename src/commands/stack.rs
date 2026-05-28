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
    color: bool,
) -> Result<String, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    let manager = PullRequestStackManager::new(&context, services);
    match request {
        StackRequest::Show => {
            progress.status("Loading pull request stack…");
            let snapshot = manager.stored_snapshot(PullRequestStackSelection::default());
            progress.finish();
            render_stack_snapshot(&snapshot?, color)
        }
        StackRequest::Open { print } => {
            progress.status("Loading pull request stack…");
            let snapshot = manager.cached_open_snapshot();
            progress.finish();
            open_stack_pull_request(&context, services, selector, &snapshot?, print)
        }
        StackRequest::Refresh => {
            progress.status("Refreshing pull request stack…");
            let snapshot = manager.refresh_authored_open_pull_requests();
            progress.finish();
            render_stack_snapshot(&snapshot?, color)
        }
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
