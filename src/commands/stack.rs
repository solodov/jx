use super::*;

/// Handles repo-local pull-request stack state that survives bookmark movement.
pub(super) fn handle_stack(
    request: StackRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<String, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    match request {
        StackRequest::Show => {
            render_stack_metadata(&read_stack_metadata(&context.repository_root)?)
        }
        StackRequest::Track => track_current_pull_request_stack(&context, services),
        StackRequest::Reset => {
            reset_stack_metadata(&context.repository_root)?;
            Ok("Stack state reset\n".to_owned())
        }
    }
}

fn track_current_pull_request_stack(
    context: &RepositoryContext,
    services: &dyn CommandServices,
) -> Result<String, CommandError> {
    let branches = services.pull_request_bookmarks(context)?;
    if branches.is_empty() {
        let metadata = StackMetadata::default();
        write_stack_metadata(&context.repository_root, &metadata)?;
        return render_stack_metadata(&metadata);
    }

    let author = services.authenticated_login(&context.token_source)?;
    let mut pull_requests = Vec::new();
    let mut seen_numbers = BTreeSet::new();
    for branch in branches {
        let Some(pull_request) =
            services.find_authored_open_pull_request_for_head(context, &branch, &author)?
        else {
            continue;
        };
        if seen_numbers.insert(pull_request.number) {
            pull_requests.push(pull_request);
        }
    }

    let existing_metadata = read_stack_metadata(&context.repository_root)?;
    let metadata = stack_metadata_from_pull_requests(&pull_requests, &existing_metadata);
    write_stack_metadata(&context.repository_root, &metadata)?;
    render_stack_metadata(&metadata)
}

fn render_stack_metadata(metadata: &StackMetadata) -> Result<String, CommandError> {
    if metadata.nodes.is_empty() {
        return Ok("Stack state: none\n".to_owned());
    }

    let rows = stack_metadata_rows(&metadata.nodes);
    let mut output = String::from("Stack state:\n");
    for row in rows {
        output.push_str("  ");
        output.push_str(&row);
        output.push('\n');
    }
    Ok(output)
}

pub(super) fn stack_metadata_rows(nodes: &[StackMetadataNode]) -> Vec<String> {
    let metadata = StackMetadata {
        version: 1,
        nodes: nodes.to_vec(),
    };
    PullRequestStackSnapshot::from_metadata(
        &metadata,
        &[],
        &[],
        PullRequestStackSelection::default(),
    )
    .rows()
    .into_iter()
    .map(stack_row_label)
    .collect()
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
