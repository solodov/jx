use super::*;

#[cfg(test)]
use crate::repository::StackMetadataNode;

/// Handles repo-local pull-request stack state that survives bookmark movement.
pub(super) fn handle_stack(
    request: StackRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<String, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    let manager = PullRequestStackManager::new(&context, services);
    match request {
        StackRequest::Show => {
            render_stack_snapshot(&manager.stored_snapshot(PullRequestStackSelection::default())?)
        }
        StackRequest::Track => render_stack_snapshot(&manager.track_authored_open_pull_requests()?),
        StackRequest::Reset => {
            manager.reset()?;
            Ok("Stack state reset\n".to_owned())
        }
    }
}

fn render_stack_snapshot(snapshot: &PullRequestStackSnapshot) -> Result<String, CommandError> {
    if snapshot.nodes.is_empty() {
        return Ok("Stack state: none\n".to_owned());
    }

    let mut output = String::from("Stack state:\n");
    for row in stack_snapshot_rows(snapshot) {
        output.push_str("  ");
        output.push_str(&row);
        output.push('\n');
    }
    Ok(output)
}

fn stack_snapshot_rows(snapshot: &PullRequestStackSnapshot) -> Vec<String> {
    snapshot.rows().into_iter().map(stack_row_label).collect()
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

fn stack_row_label(row: PullRequestStackRow<'_>) -> String {
    let mut label = row.prefix;
    label.push_str(stack_node_status(row.node));
    label.push(' ');
    if let Some(number) = row.node.pull_request_number() {
        label.push_str(&format!("#{number:<6} "));
    }
    label.push_str(stack_node_title(row.node));
    label
}

fn stack_node_status(node: &PullRequestStackNode) -> &'static str {
    if node.merged {
        "✓"
    } else if node.draft {
        "◌"
    } else {
        "◯"
    }
}

fn stack_node_title(node: &PullRequestStackNode) -> &str {
    let title = node.title.trim();
    if title.is_empty() {
        "(untitled)"
    } else {
        title
    }
}
