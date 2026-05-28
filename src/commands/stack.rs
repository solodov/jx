use super::*;

#[cfg(test)]
use crate::repository::StackMetadataNode;

/// Handles repo-local pull-request stack state that survives bookmark movement.
pub(super) fn handle_stack(
    request: StackRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
) -> Result<String, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    let manager = PullRequestStackManager::new(&context, services);
    match request {
        StackRequest::Show => {
            progress.status("Loading pull request stack…");
            let snapshot = manager.stored_snapshot(PullRequestStackSelection::default())?;
            progress.finish();
            render_stack_snapshot(&snapshot)
        }
        StackRequest::Track => {
            progress.status("Tracking pull request stack…");
            let snapshot = manager.track_authored_open_pull_requests()?;
            progress.finish();
            render_stack_snapshot(&snapshot)
        }
        StackRequest::Reset => {
            progress.status("Resetting pull request stack…");
            manager.reset()?;
            progress.finish();
            Ok("Stack state reset\n".to_owned())
        }
    }
}

fn render_stack_snapshot(snapshot: &PullRequestStackSnapshot) -> Result<String, CommandError> {
    if snapshot.nodes.is_empty() {
        return Ok("No stack state\n".to_owned());
    }

    let mut output = String::new();
    for row in stack_snapshot_rows(snapshot) {
        output.push_str(&row);
        output.push('\n');
    }
    Ok(output)
}

fn stack_snapshot_rows(snapshot: &PullRequestStackSnapshot) -> Vec<String> {
    snapshot
        .rows()
        .into_iter()
        .map(|row| row.plain_label())
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
    stack_snapshot_rows(&snapshot)
}
