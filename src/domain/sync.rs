use super::*;

const STACK_CONTEXT_START: &str = "<!-- jx-stack:start -->";
const STACK_CONTEXT_END: &str = "<!-- jx-stack:end -->";

/// Builds the operator-facing report for a completed `jx fetch` mutation.
pub fn fetch_report(context: &RepositoryContext, outcome: FetchOutcome) -> FetchReport {
    FetchReport {
        repository: repository_summary(context),
        outcome,
    }
}

/// Builds the operator-facing report for a completed fetch-and-push synchronization.
pub fn sync_report(
    context: &RepositoryContext,
    fetch: FetchOutcome,
    push: SyncPushOutcome,
    pull_requests: Vec<PullRequestRecord>,
) -> SyncReport {
    SyncReport {
        repository: repository_summary(context),
        fetch,
        push: push.pushed,
        skipped_conflicted_bookmarks: push.skipped_conflicted_bookmarks,
        skipped_same_tree_bookmarks: push.skipped_same_tree_bookmarks,
        pull_requests,
    }
}

/// Updates PR descriptions and stack bases for pushed tracked bookmarks.
pub async fn sync_pull_requests(
    context: &RepositoryContext,
    push: &TrackedPushOutcome,
    stack_metadata: &StackMetadata,
    github: &dyn GitHubClient,
) -> Result<Vec<PullRequestRecord>, WorkflowError> {
    let mut seen = BTreeSet::new();
    let mut pull_requests = Vec::new();
    for bookmark in &push.bookmarks {
        if !seen.insert(bookmark.branch.as_str()) {
            continue;
        }

        let Some(pull_request) =
            sync_bookmark_pull_request(context, github, stack_metadata, bookmark.branch.as_str())
                .await?
        else {
            continue;
        };

        let pull_request = sync_pull_request_metadata(
            context,
            github,
            pull_request,
            bookmark.pull_request_description.as_deref(),
            bookmark.pull_request_base.as_deref(),
            stack_metadata,
        )
        .await?;
        pull_requests.push(pull_request);
    }

    Ok(pull_requests)
}

async fn sync_bookmark_pull_request(
    context: &RepositoryContext,
    github: &dyn GitHubClient,
    stack_metadata: &StackMetadata,
    branch: &str,
) -> Result<Option<PullRequestRecord>, WorkflowError> {
    let head = PullRequestHead::same_repository(&context.origin.github.owner, branch);
    if let Some(pull_request) = github
        .find_open_pull_request(&context.origin.github, &head)
        .await?
    {
        return Ok(Some(pull_request));
    }

    // Newly created PRs can lag behind head-branch search, so durable stack metadata
    // gives immediate post-publish stack-context sync a stable lookup key.
    let Some(number) = stack_metadata
        .nodes
        .iter()
        .find(|node| node.branch == branch && !node.merged)
        .and_then(|node| node.pull_request)
    else {
        return Ok(None);
    };

    Ok(github
        .find_pull_request_by_number(&context.origin.github, number)
        .await?
        .filter(|pull_request| pull_request.head_branch == branch))
}

async fn sync_pull_request_metadata(
    context: &RepositoryContext,
    github: &dyn GitHubClient,
    pull_request: PullRequestRecord,
    description: Option<&str>,
    base: Option<&str>,
    stack_metadata: &StackMetadata,
) -> Result<PullRequestRecord, WorkflowError> {
    let mut update = PullRequestUpdate::default();

    if let Some(description) = description {
        let (title, body) = pull_request_description_from_text(description)?;
        let body = pull_request_body_with_stack_context(
            &body,
            stack_metadata,
            &pull_request,
            &context.origin.github.https_url(),
        );
        if pull_request.title != title {
            update.title = Some(title);
        }
        if !pull_request_body_matches(pull_request.body.as_deref(), &body) {
            update.body = Some(body);
        }
    }

    if let Some(base) = base {
        if pull_request.base_branch != base {
            update.base = Some(base.to_owned());
        }
    }

    if update.title.is_none() && update.body.is_none() && update.base.is_none() {
        return Ok(pull_request);
    }

    Ok(github
        .update_pull_request(&context.origin.github, pull_request.number, update)
        .await?)
}

/// Returns a PR body with generated stack context matching stack sync behavior.
pub fn pull_request_body_with_stack_context(
    body: &str,
    metadata: &StackMetadata,
    current: &PullRequestRecord,
    repository_url: &str,
) -> String {
    match render_pull_request_stack_context(metadata, current, repository_url) {
        Some(block) => replace_generated_stack_context(body, Some(&block)),
        None => replace_generated_stack_context(body, None),
    }
}

fn render_pull_request_stack_context(
    metadata: &StackMetadata,
    current: &PullRequestRecord,
    repository_url: &str,
) -> Option<String> {
    let snapshot = PullRequestStackSnapshot::from_metadata(
        metadata,
        std::slice::from_ref(&current.head_branch),
        std::slice::from_ref(current),
        PullRequestStackSelection::pull_request(current.number),
    );
    let component =
        snapshot.component_for_selection(PullRequestStackSelection::pull_request(current.number));
    if component.nodes.len() <= 1 {
        return None;
    }

    let mut output = String::from(STACK_CONTEXT_START);
    output.push_str("\n### Pull request stack\n\n");
    for row in component.rows() {
        output.push_str(&stack_context_row(row, repository_url));
        output.push('\n');
    }
    output.push_str(STACK_CONTEXT_END);
    Some(output)
}

fn stack_context_row(row: PullRequestStackRow<'_>, repository_url: &str) -> String {
    let node = row.node;
    let link = stack_context_link(node, repository_url);
    let entry = if node.is_current {
        format!("**{link}** — this PR")
    } else if node.draft {
        format!("{link} — draft")
    } else {
        link
    };
    format!(
        "{}{status} {entry}",
        markdown_stack_tree_prefix(&row.prefix),
        status = row.status_symbol(),
    )
}

fn markdown_stack_tree_prefix(prefix: &str) -> String {
    compact_stack_tree_prefix(prefix)
        .replace("│ ", "│&nbsp;")
        .replace("  ", "&nbsp;&nbsp;")
}

fn stack_context_link(node: &PullRequestStackNode, repository_url: &str) -> String {
    let label = match node.pull_request_number() {
        Some(number) => format!("#{} {}", number, node.display_title()),
        None => node.display_title().to_owned(),
    };
    match &node.pull_request {
        Some(pull_request) => {
            let url = pull_request
                .url
                .clone()
                .unwrap_or_else(|| format!("{repository_url}/pull/{}", pull_request.number));
            format!("[{}]({url})", escape_markdown_link_text(&label))
        }
        None => escape_markdown_link_text(&label),
    }
}

/// Removes generated stack delimiter comments for renderers that should show only visible PR content.
pub fn pull_request_description_without_stack_context_markers(description: &str) -> String {
    if !description.contains(STACK_CONTEXT_START) && !description.contains(STACK_CONTEXT_END) {
        return description.to_owned();
    }

    let mut lines: Vec<&str> = Vec::new();
    for line in description.lines() {
        match line.trim() {
            STACK_CONTEXT_START => continue,
            STACK_CONTEXT_END => {
                if matches!(lines.last(), Some(previous) if previous.trim().is_empty()) {
                    lines.pop();
                }
            }
            _ => lines.push(line),
        }
    }

    lines.join("\n")
}

fn replace_generated_stack_context(body: &str, block: Option<&str>) -> String {
    let Some(range) = generated_stack_context_range(body) else {
        let Some(block) = block else {
            return body.to_owned();
        };
        if body.trim().is_empty() {
            return block.to_owned();
        }
        return format!("{}\n\n{block}", body.trim_end());
    };

    let prefix = body[..range.start].trim_end();
    let suffix = body[range.end..].trim_start_matches(['\r', '\n']);
    let mut result = String::new();
    if !prefix.is_empty() {
        result.push_str(prefix);
    }
    if let Some(block) = block {
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        result.push_str(block);
    }
    if !suffix.is_empty() {
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        result.push_str(suffix);
    }
    result
}

fn generated_stack_context_range(body: &str) -> Option<std::ops::Range<usize>> {
    let start = body.find(STACK_CONTEXT_START)?;
    let after_start = start + STACK_CONTEXT_START.len();
    let end = body[after_start..].find(STACK_CONTEXT_END)? + after_start + STACK_CONTEXT_END.len();
    Some(start..line_end_after(body, end))
}

fn line_end_after(value: &str, offset: usize) -> usize {
    match value[offset..].chars().next() {
        Some('\r') => offset + 1 + value[offset + 1..].starts_with('\n') as usize,
        Some('\n') => offset + 1,
        _ => offset,
    }
}

fn escape_markdown_link_text(value: &str) -> String {
    value.replace('[', "\\[").replace(']', "\\]")
}

fn pull_request_body_matches(existing: Option<&str>, desired: &str) -> bool {
    if desired.is_empty() {
        existing.unwrap_or_default().is_empty()
    } else {
        existing == Some(desired)
    }
}

/// Returns an error when callers choose to block on conflicts created by fetch.
pub fn ensure_fetch_is_pushable(outcome: &FetchOutcome) -> Result<(), WorkflowError> {
    let conflicted = outcome
        .rebased_commits
        .iter()
        .filter(|commit| commit.has_conflict)
        .map(|commit| commit.short_change_id.clone())
        .collect::<Vec<_>>();

    if conflicted.is_empty() {
        Ok(())
    } else {
        Err(WorkflowError::FetchConflicts {
            commits: conflicted.join(", "),
        })
    }
}
