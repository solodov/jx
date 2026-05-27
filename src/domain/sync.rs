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

/// Builds the operator-facing report for a completed `jx rebase-on-trunk` mutation.
pub fn rebase_on_trunk_report(
    context: &RepositoryContext,
    outcome: RebaseOnTrunkOutcome,
) -> RebaseOnTrunkReport {
    RebaseOnTrunkReport {
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
        pull_requests,
    }
}

/// Updates PR descriptions and stack bases for pushed tracked bookmarks.
pub async fn sync_pull_requests(
    context: &RepositoryContext,
    push: &TrackedPushOutcome,
    github: &dyn GitHubClient,
) -> Result<Vec<PullRequestRecord>, WorkflowError> {
    let stack_metadata = read_stack_metadata(&context.repository_root)?;
    let mut seen = BTreeSet::new();
    let mut pull_requests = Vec::new();
    for bookmark in &push.bookmarks {
        if !seen.insert(bookmark.branch.as_str()) {
            continue;
        }

        let head = PullRequestHead::same_repository(&context.origin.github.owner, &bookmark.branch);
        let Some(pull_request) = github
            .find_open_pull_request(&context.origin.github, &head)
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
            &stack_metadata,
        )
        .await?;
        pull_requests.push(pull_request);
    }

    Ok(pull_requests)
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
        if pull_request.title != title
            || !pull_request_body_matches(pull_request.body.as_deref(), &body)
        {
            update.title = Some(title);
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

fn pull_request_body_with_stack_context(
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
    let component = stack_component_for_pull_request(metadata, current)?;
    if component.len() <= 1 {
        return None;
    }

    let mut output = String::from(STACK_CONTEXT_START);
    output.push_str("\n### Pull request stack\n\n");
    for row in stack_context_rows(&component, current.number, repository_url) {
        output.push_str(&row);
        output.push('\n');
    }
    output.push('\n');
    output.push_str(STACK_CONTEXT_END);
    Some(output)
}

fn stack_component_for_pull_request(
    metadata: &StackMetadata,
    current: &PullRequestRecord,
) -> Option<Vec<StackMetadataNode>> {
    let indexes_by_branch = stack_indexes_by_branch(&metadata.nodes);
    let current_index = metadata.nodes.iter().position(|node| {
        node.pull_request == Some(current.number) || node.branch == current.head_branch
    })?;
    let mut children = vec![Vec::new(); metadata.nodes.len()];
    for (index, node) in metadata.nodes.iter().enumerate() {
        if let Some(parent) = node
            .parent_branch
            .as_deref()
            .and_then(|branch| indexes_by_branch.get(branch).copied())
        {
            if parent != index {
                children[parent].push(index);
            }
        }
    }

    let mut selected = BTreeSet::new();
    let mut pending = vec![current_index];
    while let Some(index) = pending.pop() {
        if !selected.insert(index) {
            continue;
        }
        if let Some(parent) = metadata.nodes[index]
            .parent_branch
            .as_deref()
            .and_then(|branch| indexes_by_branch.get(branch).copied())
        {
            pending.push(parent);
        }
        pending.extend(children[index].iter().copied());
    }

    Some(
        selected
            .into_iter()
            .map(|index| metadata.nodes[index].clone())
            .collect(),
    )
}

fn stack_context_rows(
    nodes: &[StackMetadataNode],
    current_pull_request: u64,
    repository_url: &str,
) -> Vec<String> {
    let indexes_by_branch = stack_indexes_by_branch(nodes);
    let mut children = vec![Vec::new(); nodes.len()];
    let mut roots = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        match node
            .parent_branch
            .as_deref()
            .and_then(|branch| indexes_by_branch.get(branch).copied())
        {
            Some(parent) if parent != index => children[parent].push(index),
            _ => roots.push(index),
        }
    }

    sort_stack_context_indexes(&mut roots, nodes);
    for child_indexes in &mut children {
        sort_stack_context_indexes(child_indexes, nodes);
    }

    let mut tree = StackContextTree::new(children, nodes, current_pull_request, repository_url);
    tree.append_roots(&roots, 0);
    tree.rows
}

struct StackContextTree<'a> {
    children: Vec<Vec<usize>>,
    nodes: &'a [StackMetadataNode],
    current_pull_request: u64,
    repository_url: &'a str,
    ancestor_has_next: Vec<bool>,
    rows: Vec<String>,
}

impl<'a> StackContextTree<'a> {
    fn new(
        children: Vec<Vec<usize>>,
        nodes: &'a [StackMetadataNode],
        current_pull_request: u64,
        repository_url: &'a str,
    ) -> Self {
        Self {
            children,
            nodes,
            current_pull_request,
            repository_url,
            ancestor_has_next: Vec::new(),
            rows: Vec::new(),
        }
    }

    fn append_roots(&mut self, roots: &[usize], depth: usize) {
        for (position, root) in roots.iter().copied().enumerate() {
            let has_next_sibling = position + 1 < roots.len();
            self.rows.push(stack_context_row(
                &self.nodes[root],
                self.current_pull_request,
                self.repository_url,
                &self.ancestor_has_next,
                depth,
                has_next_sibling,
            ));

            let include_current_in_descendant_prefix = depth > 0;
            if include_current_in_descendant_prefix {
                self.ancestor_has_next.push(has_next_sibling);
            }
            let children = self.children[root].clone();
            self.append_roots(&children, depth + 1);
            if include_current_in_descendant_prefix {
                self.ancestor_has_next.pop();
            }
        }
    }
}

fn stack_context_row(
    node: &StackMetadataNode,
    current_pull_request: u64,
    repository_url: &str,
    ancestor_has_next: &[bool],
    depth: usize,
    has_next_sibling: bool,
) -> String {
    let current = node.pull_request == Some(current_pull_request);
    let status = stack_context_status(node, current);
    let link = stack_context_link(node, repository_url);
    let entry = if current {
        format!("**{link}** — this PR")
    } else if node.draft {
        format!("{link} — draft")
    } else {
        link
    };
    format!(
        "{}{status} {entry}",
        stack_context_tree_prefix(ancestor_has_next, depth, has_next_sibling)
    )
}

fn stack_context_tree_prefix(
    ancestor_has_next: &[bool],
    depth: usize,
    has_next_sibling: bool,
) -> String {
    if depth == 0 {
        return String::new();
    }

    let mut prefix = String::new();
    for ancestor_has_next in ancestor_has_next {
        prefix.push_str(if *ancestor_has_next {
            "│&nbsp;&nbsp;"
        } else {
            "&nbsp;&nbsp;&nbsp;"
        });
    }
    prefix.push_str(if has_next_sibling {
        "├─ "
    } else {
        "└─ "
    });
    prefix
}

fn stack_context_status(node: &StackMetadataNode, current: bool) -> &'static str {
    if node.merged {
        "✓"
    } else if current {
        "◉"
    } else if node.draft {
        "◌"
    } else {
        "◯"
    }
}

fn stack_context_link(node: &StackMetadataNode, repository_url: &str) -> String {
    let title = if node.title.trim().is_empty() {
        "(untitled)"
    } else {
        node.title.trim()
    };
    let label = match node.pull_request {
        Some(number) => format!("#{} {}", number, title),
        None => title.to_owned(),
    };
    match node.pull_request {
        Some(number) => {
            let url = node
                .url
                .clone()
                .unwrap_or_else(|| format!("{repository_url}/pull/{number}"));
            format!("[{}]({url})", escape_markdown_link_text(&label))
        }
        None => escape_markdown_link_text(&label),
    }
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

fn stack_indexes_by_branch(nodes: &[StackMetadataNode]) -> BTreeMap<&str, usize> {
    let mut indexes_by_branch = BTreeMap::new();
    for (index, node) in nodes.iter().enumerate() {
        indexes_by_branch
            .entry(node.branch.as_str())
            .or_insert(index);
    }
    indexes_by_branch
}

fn sort_stack_context_indexes(indexes: &mut [usize], nodes: &[StackMetadataNode]) {
    indexes.sort_by(|left, right| {
        stack_context_sort_key(&nodes[*left]).cmp(&stack_context_sort_key(&nodes[*right]))
    });
}

fn stack_context_sort_key(node: &StackMetadataNode) -> (u8, u64, &str, &str) {
    (
        node.draft as u8,
        node.pull_request.unwrap_or(u64::MAX),
        node.title.as_str(),
        node.branch.as_str(),
    )
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
        .map(|commit| commit.new_short_commit_id.clone())
        .collect::<Vec<_>>();

    if conflicted.is_empty() {
        Ok(())
    } else {
        Err(WorkflowError::FetchConflicts {
            commits: conflicted.join(", "),
        })
    }
}
