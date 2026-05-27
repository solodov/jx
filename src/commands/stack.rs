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

pub(super) fn stack_metadata_from_pull_requests(
    pull_requests: &[PullRequestRecord],
    existing_metadata: &StackMetadata,
) -> StackMetadata {
    let pull_requests_by_head = pull_requests
        .iter()
        .map(|pull_request| (pull_request.head_branch.as_str(), pull_request))
        .collect::<BTreeMap<_, _>>();
    let existing_nodes_by_branch = existing_metadata
        .nodes
        .iter()
        .map(|node| (node.branch.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = pull_requests
        .iter()
        .map(|pull_request| {
            let live_parent = pull_requests_by_head.get(pull_request.base_branch.as_str());
            let existing_node = existing_nodes_by_branch.get(pull_request.head_branch.as_str());
            let parent_branch = live_parent
                .map(|parent| parent.head_branch.clone())
                .or_else(|| existing_node.and_then(|node| node.parent_branch.clone()));
            StackMetadataNode {
                branch: pull_request.head_branch.clone(),
                base_branch: pull_request.base_branch.clone(),
                parent_branch,
                pull_request: Some(pull_request.number),
                parent_pull_request: live_parent
                    .map(|parent| parent.number)
                    .or_else(|| existing_node.and_then(|node| node.parent_pull_request)),
                title: pull_request_title(pull_request).to_owned(),
                url: pull_request.html_url.clone(),
                draft: pull_request.draft,
                merged: pull_request.merged,
            }
        })
        .collect::<Vec<_>>();
    preserve_missing_ancestors(&mut nodes, &existing_nodes_by_branch);
    sort_stack_nodes(&mut nodes);

    StackMetadata { version: 1, nodes }
}

fn preserve_missing_ancestors(
    nodes: &mut Vec<StackMetadataNode>,
    existing_nodes_by_branch: &BTreeMap<&str, &StackMetadataNode>,
) {
    let mut retained_branches = nodes
        .iter()
        .map(|node| node.branch.clone())
        .collect::<BTreeSet<_>>();
    let mut pending = nodes
        .iter()
        .filter_map(|node| node.parent_branch.clone())
        .collect::<Vec<_>>();

    while let Some(branch) = pending.pop() {
        if retained_branches.contains(&branch) {
            continue;
        }
        let Some(existing_node) = existing_nodes_by_branch.get(branch.as_str()) else {
            continue;
        };
        nodes.push((*existing_node).clone());
        retained_branches.insert(branch);
        if let Some(parent_branch) = &existing_node.parent_branch {
            pending.push(parent_branch.clone());
        }
    }
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

    sort_stack_indexes(&mut roots, nodes);
    for child_indexes in &mut children {
        sort_stack_indexes(child_indexes, nodes);
    }

    let mut tree = StackMetadataTree::new(children, nodes);
    tree.append_roots(&roots);

    let mut unvisited = (0..nodes.len())
        .filter(|index| !tree.visited.contains(index))
        .collect::<Vec<_>>();
    sort_stack_indexes(&mut unvisited, nodes);
    tree.append_roots(&unvisited);

    tree.rows
}

pub(super) fn stack_metadata_component_branches(
    nodes: &[StackMetadataNode],
    selected_branches: &[String],
) -> Vec<String> {
    let indexes_by_branch = stack_indexes_by_branch(nodes);
    let mut children = vec![Vec::new(); nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
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
    for child_indexes in &mut children {
        sort_stack_indexes(child_indexes, nodes);
    }

    let mut selected = BTreeSet::new();
    let mut pending = selected_branches
        .iter()
        .filter_map(|branch| indexes_by_branch.get(branch.as_str()).copied())
        .collect::<Vec<_>>();
    while let Some(index) = pending.pop() {
        if !selected.insert(index) {
            continue;
        }
        if let Some(parent) = nodes[index]
            .parent_branch
            .as_deref()
            .and_then(|branch| indexes_by_branch.get(branch).copied())
        {
            pending.push(parent);
        }
        pending.extend(children[index].iter().copied());
    }

    let mut roots = selected
        .iter()
        .copied()
        .filter(|index| {
            nodes[*index]
                .parent_branch
                .as_deref()
                .and_then(|branch| indexes_by_branch.get(branch).copied())
                .is_none_or(|parent| !selected.contains(&parent) || parent == *index)
        })
        .collect::<Vec<_>>();
    sort_stack_indexes(&mut roots, nodes);

    let mut ordered = Vec::new();
    append_selected_stack_branches(&mut ordered, &roots, &children, nodes, &selected);
    ordered
}

fn append_selected_stack_branches(
    ordered: &mut Vec<String>,
    roots: &[usize],
    children: &[Vec<usize>],
    nodes: &[StackMetadataNode],
    selected: &BTreeSet<usize>,
) {
    for root in roots.iter().copied() {
        ordered.push(nodes[root].branch.clone());
        let child_roots = children[root]
            .iter()
            .copied()
            .filter(|child| selected.contains(child))
            .collect::<Vec<_>>();
        append_selected_stack_branches(ordered, &child_roots, children, nodes, selected);
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

fn sort_stack_nodes(nodes: &mut [StackMetadataNode]) {
    nodes.sort_by(|left, right| stack_node_sort_key(left).cmp(&stack_node_sort_key(right)));
}

fn sort_stack_indexes(indexes: &mut [usize], nodes: &[StackMetadataNode]) {
    indexes.sort_by(|left, right| {
        stack_node_sort_key(&nodes[*left]).cmp(&stack_node_sort_key(&nodes[*right]))
    });
}

fn stack_node_sort_key(node: &StackMetadataNode) -> (u8, u64, &str, &str) {
    (
        node.draft as u8,
        node.pull_request.unwrap_or(u64::MAX),
        node.title.as_str(),
        node.branch.as_str(),
    )
}

struct StackMetadataTree<'a> {
    children: Vec<Vec<usize>>,
    nodes: &'a [StackMetadataNode],
    ancestor_has_next: Vec<bool>,
    visited: BTreeSet<usize>,
    rows: Vec<String>,
}

impl<'a> StackMetadataTree<'a> {
    fn new(children: Vec<Vec<usize>>, nodes: &'a [StackMetadataNode]) -> Self {
        Self {
            children,
            nodes,
            ancestor_has_next: Vec::new(),
            visited: BTreeSet::new(),
            rows: Vec::new(),
        }
    }

    fn append_roots(&mut self, roots: &[usize]) {
        for (position, root) in roots.iter().copied().enumerate() {
            self.append(root, 0, position + 1 < roots.len());
        }
    }

    fn append(&mut self, index: usize, depth: usize, has_next_sibling: bool) {
        if !self.visited.insert(index) {
            return;
        }

        self.rows.push(stack_node_label(
            &self.nodes[index],
            &self.ancestor_has_next,
            depth,
            has_next_sibling,
        ));

        let include_current_in_descendant_prefix = depth > 0;
        let children = self.children[index].clone();
        for (position, child) in children.iter().copied().enumerate() {
            let child_has_next_sibling = position + 1 < children.len();
            if include_current_in_descendant_prefix {
                self.ancestor_has_next.push(has_next_sibling);
            }
            self.append(child, depth + 1, child_has_next_sibling);
            if include_current_in_descendant_prefix {
                self.ancestor_has_next.pop();
            }
        }
    }
}

fn stack_node_label(
    node: &StackMetadataNode,
    ancestor_has_next: &[bool],
    depth: usize,
    has_next_sibling: bool,
) -> String {
    let mut label = String::new();
    if depth > 0 {
        for ancestor_has_next in ancestor_has_next {
            label.push_str(if *ancestor_has_next { "│  " } else { "   " });
        }
        label.push_str(if has_next_sibling {
            "├─ "
        } else {
            "└─ "
        });
    }

    label.push_str(stack_node_status(node));
    label.push(' ');
    if let Some(number) = node.pull_request {
        label.push_str(&format!("#{number:<6} "));
    }
    label.push_str(if node.title.trim().is_empty() {
        "(untitled)"
    } else {
        node.title.trim()
    });
    label
}

fn stack_node_status(node: &StackMetadataNode) -> &'static str {
    if node.merged {
        "✓"
    } else if node.draft {
        "◌"
    } else {
        "◯"
    }
}

fn pull_request_title(pull_request: &PullRequestRecord) -> &str {
    let title = pull_request.title.trim();
    if title.is_empty() {
        "(untitled)"
    } else {
        title
    }
}
