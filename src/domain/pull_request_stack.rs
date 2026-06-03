use super::*;

/// Builds a read-only health report for a pull-request stack snapshot.
pub fn pull_request_stack_status_report(
    context: &RepositoryContext,
    snapshot: PullRequestStackSnapshot,
    statuses: Vec<PullRequestStatusRecord>,
) -> PullRequestStackStatusReport {
    PullRequestStackStatusReport {
        repository: repository_summary(context),
        snapshot,
        statuses: statuses
            .into_iter()
            .map(|status| (status.number, status))
            .collect(),
    }
}

/// Renderer-agnostic view of the repository's pull-request stack state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestStackSnapshot {
    pub nodes: Vec<PullRequestStackNode>,
    pub current_branch: Option<String>,
    pub current_pull_request: Option<u64>,
}

impl PullRequestStackSnapshot {
    /// Builds a stack snapshot by layering live PR facts over durable local metadata.
    pub fn from_metadata(
        metadata: &StackMetadata,
        local_branches: &[String],
        live_pull_requests: &[PullRequestRecord],
        selection: PullRequestStackSelection,
    ) -> Self {
        let local_branches = local_branches
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut nodes = metadata
            .nodes
            .iter()
            .map(|node| PullRequestStackNode::from_metadata(node, &local_branches))
            .collect::<Vec<_>>();
        let mut indexes_by_branch = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.branch.clone(), index))
            .collect::<BTreeMap<_, _>>();
        let mut indexes_by_pull_request = nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| node.pull_request_number().map(|number| (number, index)))
            .collect::<BTreeMap<_, _>>();

        let mut live_pull_requests = live_pull_requests.to_vec();
        live_pull_requests.sort_by(|left, right| {
            pull_request_stack_sort_key(left).cmp(&pull_request_stack_sort_key(right))
        });
        for pull_request in &live_pull_requests {
            match (
                indexes_by_branch
                    .get(pull_request.head_branch.as_str())
                    .copied(),
                indexes_by_pull_request.get(&pull_request.number).copied(),
            ) {
                (Some(index), _) => {
                    nodes[index].apply_live_pull_request(pull_request);
                    indexes_by_pull_request.insert(pull_request.number, index);
                }
                (None, Some(index)) => nodes[index].refresh_live_pull_request(pull_request),
                (None, None) => {
                    let index = nodes.len();
                    nodes.push(PullRequestStackNode::from_live_pull_request(
                        pull_request,
                        &local_branches,
                    ));
                    indexes_by_branch.insert(pull_request.head_branch.clone(), index);
                    indexes_by_pull_request.insert(pull_request.number, index);
                }
            }
        }

        resolve_live_parent_edges(&mut nodes, &indexes_by_branch);
        apply_current_selection(nodes, selection)
    }

    /// Returns every stack node in merge order with tree prefixes for renderers.
    pub fn rows(&self) -> Vec<PullRequestStackRow<'_>> {
        pull_request_stack_rows(&self.nodes)
    }

    /// Returns the connected stack component around a selected branch or PR.
    pub fn component_for_selection(&self, selection: PullRequestStackSelection) -> Self {
        let selected = selected_stack_indexes(&self.nodes, &selection);
        self.component_from_indexes(selected, selection)
    }

    /// Returns the connected stack component around one or more selected branches.
    pub fn component_for_branches(&self, branches: &[String]) -> Self {
        let selection = PullRequestStackSelection {
            branch: self.current_branch.clone(),
            pull_request: self.current_pull_request,
        };
        let indexes_by_branch = stack_indexes_by_branch(&self.nodes);
        let mut pending = branches
            .iter()
            .filter_map(|branch| indexes_by_branch.get(branch.as_str()).copied())
            .collect::<Vec<_>>();
        let selected = connected_stack_indexes(&self.nodes, &mut pending);
        self.component_from_indexes(selected, selection)
    }

    /// Returns all branch names in merge order for the selected connected component.
    pub fn component_branches_for(&self, branches: &[String]) -> Vec<String> {
        self.component_for_branches(branches)
            .nodes
            .into_iter()
            .map(|node| node.branch)
            .collect()
    }

    /// Returns local branch names in merge order for the selected connected component.
    pub fn local_component_branches_for(&self, branches: &[String]) -> Vec<String> {
        self.component_for_branches(branches)
            .nodes
            .into_iter()
            .filter(|node| node.is_local)
            .map(|node| node.branch)
            .collect()
    }

    fn component_from_indexes(
        &self,
        selected: BTreeSet<usize>,
        selection: PullRequestStackSelection,
    ) -> Self {
        let ordered_indexes = ordered_selected_stack_indexes(&self.nodes, &selected);
        let nodes = ordered_indexes
            .into_iter()
            .map(|index| self.nodes[index].clone())
            .collect::<Vec<_>>();
        apply_current_selection(nodes, selection)
    }
}

/// One stack row with its renderer-neutral tree prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestStackRow<'a> {
    pub node: &'a PullRequestStackNode,
    pub prefix: String,
}

impl PullRequestStackRow<'_> {
    /// Returns the stable status symbol shared by stack renderers.
    pub fn status_symbol(&self) -> &'static str {
        self.node.status_symbol()
    }

    /// Returns the trimmed title fallback shared by stack renderers.
    pub fn display_title(&self) -> &str {
        self.node.display_title()
    }

    /// Returns the stable plain-text row label shared by CLI stack views.
    pub fn plain_label(&self) -> String {
        let mut label = self.prefix.clone();
        label.push_str(self.status_symbol());
        label.push(' ');
        if let Some(number) = self.node.pull_request_number() {
            label.push_str(&format!("#{number:<6} "));
        }
        label.push_str(self.display_title());
        label
    }
}

/// Optional current selection used to mark one stack node as current.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestStackSelection {
    pub branch: Option<String>,
    pub pull_request: Option<u64>,
}

impl PullRequestStackSelection {
    pub fn branch(branch: impl Into<String>) -> Self {
        Self {
            branch: Some(branch.into()),
            pull_request: None,
        }
    }

    pub fn pull_request(number: u64) -> Self {
        Self {
            branch: None,
            pull_request: Some(number),
        }
    }
}

/// One branch in a pull-request stack, with optional GitHub PR identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestStackNode {
    pub branch: String,
    pub base_branch: String,
    pub parent_branch: Option<String>,
    pub parent_pull_request: Option<u64>,
    pub title: String,
    pub pull_request: Option<PullRequestStackPullRequest>,
    pub draft: bool,
    pub merged: bool,
    pub is_local: bool,
    pub is_current: bool,
}

impl PullRequestStackNode {
    fn from_metadata(node: &StackMetadataNode, local_branches: &BTreeSet<&str>) -> Self {
        Self {
            branch: node.branch.clone(),
            base_branch: node.base_branch.clone(),
            parent_branch: node.parent_branch.clone(),
            parent_pull_request: node.parent_pull_request,
            title: node.title.clone(),
            pull_request: node.pull_request.map(|number| PullRequestStackPullRequest {
                number,
                url: node.url.clone(),
            }),
            draft: node.draft,
            merged: node.merged,
            is_local: local_branches.contains(node.branch.as_str()),
            is_current: false,
        }
    }

    fn from_live_pull_request(
        pull_request: &PullRequestRecord,
        local_branches: &BTreeSet<&str>,
    ) -> Self {
        Self {
            branch: pull_request.head_branch.clone(),
            base_branch: pull_request.base_branch.clone(),
            parent_branch: None,
            parent_pull_request: None,
            title: pull_request.title.clone(),
            pull_request: Some(PullRequestStackPullRequest {
                number: pull_request.number,
                url: pull_request.html_url.clone(),
            }),
            draft: pull_request.draft,
            merged: pull_request.merged,
            is_local: local_branches.contains(pull_request.head_branch.as_str()),
            is_current: false,
        }
    }

    fn apply_live_pull_request(&mut self, pull_request: &PullRequestRecord) {
        self.base_branch.clone_from(&pull_request.base_branch);
        self.refresh_live_pull_request(pull_request);
    }

    fn refresh_live_pull_request(&mut self, pull_request: &PullRequestRecord) {
        self.title.clone_from(&pull_request.title);
        self.pull_request = Some(PullRequestStackPullRequest {
            number: pull_request.number,
            url: pull_request.html_url.clone(),
        });
        self.draft = pull_request.draft;
        self.merged = pull_request.merged;
    }

    pub fn pull_request_number(&self) -> Option<u64> {
        self.pull_request
            .as_ref()
            .map(|pull_request| pull_request.number)
    }

    /// Returns the stable status symbol shared by CLI and Markdown stack renderers.
    pub fn status_symbol(&self) -> &'static str {
        if self.merged {
            "✓"
        } else if self.is_current {
            "◉"
        } else if self.draft {
            "◌"
        } else {
            "◯"
        }
    }

    /// Returns a non-empty title for renderer labels.
    pub fn display_title(&self) -> &str {
        let title = self.title.trim();
        if title.is_empty() {
            "(untitled)"
        } else {
            title
        }
    }
}

/// GitHub identity for a stack node that has a known pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestStackPullRequest {
    pub number: u64,
    pub url: Option<String>,
}

fn resolve_live_parent_edges(
    nodes: &mut [PullRequestStackNode],
    indexes_by_branch: &BTreeMap<String, usize>,
) {
    let parent_updates = nodes
        .iter()
        .map(|node| {
            let parent_branch = indexes_by_branch
                .contains_key(node.base_branch.as_str())
                .then(|| node.base_branch.clone());
            let parent_pull_request = parent_branch
                .as_deref()
                .and_then(|branch| indexes_by_branch.get(branch).copied())
                .and_then(|index| nodes[index].pull_request_number());
            (parent_branch, parent_pull_request)
        })
        .collect::<Vec<_>>();

    for (node, (parent_branch, parent_pull_request)) in nodes.iter_mut().zip(parent_updates) {
        if let Some(parent_branch) = parent_branch {
            if parent_branch != node.branch {
                node.parent_branch = Some(parent_branch);
                node.parent_pull_request = parent_pull_request;
            }
        }
    }
}

fn apply_current_selection(
    mut nodes: Vec<PullRequestStackNode>,
    selection: PullRequestStackSelection,
) -> PullRequestStackSnapshot {
    let current_index = selection
        .branch
        .as_deref()
        .and_then(|branch| nodes.iter().position(|node| node.branch == branch))
        .or_else(|| {
            selection.pull_request.and_then(|number| {
                nodes
                    .iter()
                    .position(|node| node.pull_request_number() == Some(number))
            })
        });

    if let Some(index) = current_index {
        nodes[index].is_current = true;
        PullRequestStackSnapshot {
            current_branch: Some(nodes[index].branch.clone()),
            current_pull_request: nodes[index].pull_request_number(),
            nodes,
        }
    } else {
        PullRequestStackSnapshot {
            nodes,
            current_branch: selection.branch,
            current_pull_request: selection.pull_request,
        }
    }
}

/// Upserts live PR records into durable stack metadata without dropping unrelated state.
pub fn upsert_stack_metadata_pull_requests(
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
    let mut nodes = existing_metadata.nodes.clone();
    let mut indexes_by_branch = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.branch.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut pull_requests = pull_requests.to_vec();
    pull_requests.sort_by(|left, right| {
        pull_request_stack_sort_key(left).cmp(&pull_request_stack_sort_key(right))
    });

    for pull_request in &pull_requests {
        let node = stack_metadata_node_from_pull_request(
            pull_request,
            &pull_requests_by_head,
            &existing_nodes_by_branch,
            false,
        );
        if let Some(index) = indexes_by_branch.get(node.branch.as_str()).copied() {
            nodes[index] = node;
        } else {
            indexes_by_branch.insert(node.branch.clone(), nodes.len());
            nodes.push(node);
        }
    }
    sort_stack_metadata_nodes(&mut nodes);

    StackMetadata { version: 1, nodes }
}

/// Refreshes durable stack metadata from PR records matched by pull-request number.
pub fn refresh_stack_metadata_pull_requests(
    pull_requests: &[PullRequestRecord],
    existing_metadata: &StackMetadata,
) -> StackMetadata {
    let pull_requests_by_number = pull_requests
        .iter()
        .map(|pull_request| (pull_request.number, pull_request))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = existing_metadata
        .nodes
        .iter()
        .map(|node| {
            match node
                .pull_request
                .and_then(|number| pull_requests_by_number.get(&number))
            {
                Some(pull_request) => refreshed_stack_metadata_node(node, pull_request),
                None => node.clone(),
            }
        })
        .collect::<Vec<_>>();
    sort_stack_metadata_nodes(&mut nodes);

    StackMetadata { version: 1, nodes }
}

/// Refreshes cached PR status facts, drops closed PR nodes, and prunes completed trees.
pub fn maintain_stack_metadata_pull_request_statuses(
    statuses: &[PullRequestStatusRecord],
    existing_metadata: &StackMetadata,
) -> StackMetadata {
    let refreshed = refresh_stack_metadata_pull_request_statuses(statuses, existing_metadata);
    let without_closed = prune_closed_stack_metadata_nodes(statuses, &refreshed);
    prune_merged_stack_metadata_trees(&without_closed)
}

/// Refreshes durable stack metadata from read-only PR status facts matched by pull-request number.
pub fn refresh_stack_metadata_pull_request_statuses(
    statuses: &[PullRequestStatusRecord],
    existing_metadata: &StackMetadata,
) -> StackMetadata {
    let statuses_by_number = statuses
        .iter()
        .map(|status| (status.number, status))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = existing_metadata
        .nodes
        .iter()
        .map(|node| {
            match node
                .pull_request
                .and_then(|number| statuses_by_number.get(&number))
            {
                Some(status) => refreshed_stack_metadata_node_from_status(node, status),
                None => node.clone(),
            }
        })
        .collect::<Vec<_>>();
    sort_stack_metadata_nodes(&mut nodes);

    StackMetadata { version: 1, nodes }
}

/// Drops closed PR nodes while keeping any still-open descendants visible as roots.
pub fn prune_closed_stack_metadata_nodes(
    statuses: &[PullRequestStatusRecord],
    metadata: &StackMetadata,
) -> StackMetadata {
    let closed_numbers = statuses
        .iter()
        .filter(|status| status.closed)
        .map(|status| status.number)
        .collect::<BTreeSet<_>>();
    if closed_numbers.is_empty() {
        return metadata.clone();
    }

    let removed_branches = metadata
        .nodes
        .iter()
        .filter(|node| {
            node.pull_request
                .is_some_and(|number| closed_numbers.contains(&number))
        })
        .map(|node| node.branch.clone())
        .collect::<BTreeSet<_>>();
    if removed_branches.is_empty() {
        return metadata.clone();
    }

    let mut nodes = metadata
        .nodes
        .iter()
        .filter(|node| {
            node.pull_request
                .is_none_or(|number| !closed_numbers.contains(&number))
        })
        .cloned()
        .map(|mut node| {
            if node
                .parent_branch
                .as_ref()
                .is_some_and(|parent| removed_branches.contains(parent))
                || node
                    .parent_pull_request
                    .is_some_and(|number| closed_numbers.contains(&number))
            {
                node.parent_branch = None;
                node.parent_pull_request = None;
            }
            node
        })
        .collect::<Vec<_>>();
    sort_stack_metadata_nodes(&mut nodes);

    StackMetadata { version: 1, nodes }
}

/// Drops stack components whose stored PR nodes are all merged.
pub fn prune_merged_stack_metadata_trees(metadata: &StackMetadata) -> StackMetadata {
    let snapshot = PullRequestStackSnapshot::from_metadata(
        metadata,
        &[],
        &[],
        PullRequestStackSelection::default(),
    );
    let mut unvisited = (0..snapshot.nodes.len()).collect::<BTreeSet<_>>();
    let mut pruned = BTreeSet::new();
    while let Some(index) = unvisited.iter().next().copied() {
        let mut pending = vec![index];
        let component = connected_stack_indexes(&snapshot.nodes, &mut pending);
        for index in &component {
            unvisited.remove(index);
        }
        if component.iter().all(|index| snapshot.nodes[*index].merged) {
            pruned.extend(component);
        }
    }
    if pruned.is_empty() {
        return metadata.clone();
    }

    let mut nodes = metadata
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| (!pruned.contains(&index)).then_some(node.clone()))
        .collect::<Vec<_>>();
    sort_stack_metadata_nodes(&mut nodes);

    StackMetadata { version: 1, nodes }
}

/// Builds durable stack metadata from live PR records while retaining missing ancestors.
pub fn stack_metadata_from_pull_requests(
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
            stack_metadata_node_from_pull_request(
                pull_request,
                &pull_requests_by_head,
                &existing_nodes_by_branch,
                true,
            )
        })
        .collect::<Vec<_>>();
    preserve_missing_ancestors(&mut nodes, &existing_nodes_by_branch);
    sort_stack_metadata_nodes(&mut nodes);

    StackMetadata { version: 1, nodes }
}

/// Applies local jj branch ancestry while preserving durable PR identity and status.
pub fn apply_local_stack_branches(
    local_branches: &[LocalStackBranch],
    existing_metadata: &StackMetadata,
) -> StackMetadata {
    let existing_nodes_by_branch = existing_metadata
        .nodes
        .iter()
        .map(|node| (node.branch.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = existing_metadata.nodes.clone();
    let mut indexes_by_branch = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.branch.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut local_branches = local_branches.to_vec();
    local_branches.sort_by(|left, right| left.branch.cmp(&right.branch));

    for local in &local_branches {
        let node = stack_metadata_node_from_local_branch(local, &existing_nodes_by_branch);
        if let Some(index) = indexes_by_branch.get(node.branch.as_str()).copied() {
            nodes[index] = node;
        } else {
            indexes_by_branch.insert(node.branch.clone(), nodes.len());
            nodes.push(node);
        }
    }
    sort_stack_metadata_nodes(&mut nodes);

    StackMetadata { version: 1, nodes }
}

fn stack_metadata_node_from_pull_request(
    pull_request: &PullRequestRecord,
    pull_requests_by_head: &BTreeMap<&str, &PullRequestRecord>,
    existing_nodes_by_branch: &BTreeMap<&str, &StackMetadataNode>,
    preserve_existing_parent: bool,
) -> StackMetadataNode {
    let live_parent = pull_requests_by_head.get(pull_request.base_branch.as_str());
    let existing_node = existing_nodes_by_branch.get(pull_request.head_branch.as_str());
    let existing_parent = existing_nodes_by_branch.get(pull_request.base_branch.as_str());
    let parent_branch = if pull_request.base_branch == pull_request.head_branch {
        None
    } else {
        live_parent
            .map(|parent| parent.head_branch.clone())
            .or_else(|| existing_parent.map(|parent| parent.branch.clone()))
            .or_else(|| {
                existing_node.and_then(|node| {
                    preserve_existing_parent
                        .then(|| node.parent_branch.clone())
                        .flatten()
                })
            })
    };
    let parent_pull_request = parent_branch
        .as_deref()
        .and_then(|branch| {
            pull_requests_by_head
                .get(branch)
                .map(|parent| parent.number)
                .or_else(|| {
                    existing_nodes_by_branch
                        .get(branch)
                        .and_then(|node| node.pull_request)
                })
        })
        .or_else(|| {
            existing_node.and_then(|node| {
                (node.parent_branch.as_deref() == parent_branch.as_deref())
                    .then_some(node.parent_pull_request)
                    .flatten()
            })
        });

    StackMetadataNode {
        branch: pull_request.head_branch.clone(),
        base_branch: pull_request.base_branch.clone(),
        parent_branch,
        pull_request: Some(pull_request.number),
        parent_pull_request,
        title: pull_request_title(pull_request).to_owned(),
        url: pull_request.html_url.clone(),
        draft: pull_request.draft,
        merged: pull_request.merged,
    }
}

fn stack_metadata_node_from_local_branch(
    local: &LocalStackBranch,
    existing_nodes_by_branch: &BTreeMap<&str, &StackMetadataNode>,
) -> StackMetadataNode {
    let existing_node = existing_nodes_by_branch.get(local.branch.as_str());
    let parent_pull_request = local.parent_branch.as_deref().and_then(|branch| {
        existing_nodes_by_branch
            .get(branch)
            .and_then(|node| node.pull_request)
    });

    StackMetadataNode {
        branch: local.branch.clone(),
        base_branch: local.base_branch.clone(),
        parent_branch: local.parent_branch.clone(),
        pull_request: existing_node.and_then(|node| node.pull_request),
        parent_pull_request,
        title: existing_node
            .map(|node| node.title.clone())
            .unwrap_or_else(|| local_stack_branch_title(local)),
        url: existing_node.and_then(|node| node.url.clone()),
        draft: existing_node.is_some_and(|node| node.draft),
        merged: existing_node.is_some_and(|node| node.merged),
    }
}

fn local_stack_branch_title(local: &LocalStackBranch) -> String {
    let title = local.title.trim();
    if title.is_empty() {
        "(untitled)".to_owned()
    } else {
        title.to_owned()
    }
}

fn refreshed_stack_metadata_node(
    node: &StackMetadataNode,
    pull_request: &PullRequestRecord,
) -> StackMetadataNode {
    StackMetadataNode {
        branch: node.branch.clone(),
        base_branch: node.base_branch.clone(),
        parent_branch: node.parent_branch.clone(),
        pull_request: Some(pull_request.number),
        parent_pull_request: node.parent_pull_request,
        title: pull_request_title(pull_request).to_owned(),
        url: pull_request.html_url.clone(),
        draft: pull_request.draft,
        merged: pull_request.merged,
    }
}

fn refreshed_stack_metadata_node_from_status(
    node: &StackMetadataNode,
    status: &PullRequestStatusRecord,
) -> StackMetadataNode {
    StackMetadataNode {
        branch: node.branch.clone(),
        base_branch: node.base_branch.clone(),
        parent_branch: node.parent_branch.clone(),
        pull_request: Some(status.number),
        parent_pull_request: node.parent_pull_request,
        title: status.title.clone(),
        url: status.url.clone(),
        draft: status.draft,
        merged: status.merged,
    }
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

fn pull_request_stack_rows(nodes: &[PullRequestStackNode]) -> Vec<PullRequestStackRow<'_>> {
    let hierarchy = StackHierarchy::new(nodes);
    let mut tree = PullRequestStackTree::new(nodes, hierarchy.children);
    tree.append_roots(&hierarchy.roots);

    let mut unvisited = (0..nodes.len())
        .filter(|index| !tree.visited.contains(index))
        .collect::<Vec<_>>();
    sort_stack_node_indexes(&mut unvisited, nodes);
    tree.append_roots(&unvisited);

    tree.rows
}

fn selected_stack_indexes(
    nodes: &[PullRequestStackNode],
    selection: &PullRequestStackSelection,
) -> BTreeSet<usize> {
    let mut pending = Vec::new();
    if let Some(branch) = selection.branch.as_deref() {
        let indexes_by_branch = stack_indexes_by_branch(nodes);
        if let Some(index) = indexes_by_branch.get(branch).copied() {
            pending.push(index);
        }
    }
    if let Some(number) = selection.pull_request {
        pending.extend(nodes.iter().enumerate().filter_map(|(index, node)| {
            (node.pull_request_number() == Some(number)).then_some(index)
        }));
    }
    connected_stack_indexes(nodes, &mut pending)
}

fn connected_stack_indexes(
    nodes: &[PullRequestStackNode],
    pending: &mut Vec<usize>,
) -> BTreeSet<usize> {
    let indexes_by_branch = stack_indexes_by_branch(nodes);
    let hierarchy = StackHierarchy::new(nodes);
    let mut selected = BTreeSet::new();

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
        pending.extend(hierarchy.children[index].iter().copied());
    }

    selected
}

fn ordered_selected_stack_indexes(
    nodes: &[PullRequestStackNode],
    selected: &BTreeSet<usize>,
) -> Vec<usize> {
    let hierarchy = StackHierarchy::new(nodes);
    let mut roots = selected
        .iter()
        .copied()
        .filter(|index| {
            nodes[*index]
                .parent_branch
                .as_deref()
                .and_then(|branch| hierarchy.indexes_by_branch.get(branch).copied())
                .is_none_or(|parent| !selected.contains(&parent) || parent == *index)
        })
        .collect::<Vec<_>>();
    sort_stack_node_indexes(&mut roots, nodes);

    let mut ordered = Vec::new();
    append_selected_stack_indexes(&mut ordered, &roots, &hierarchy.children, selected);
    ordered
}

fn append_selected_stack_indexes(
    ordered: &mut Vec<usize>,
    roots: &[usize],
    children: &[Vec<usize>],
    selected: &BTreeSet<usize>,
) {
    for root in roots.iter().copied() {
        ordered.push(root);
        let child_roots = children[root]
            .iter()
            .copied()
            .filter(|child| selected.contains(child))
            .collect::<Vec<_>>();
        append_selected_stack_indexes(ordered, &child_roots, children, selected);
    }
}

struct StackHierarchy {
    indexes_by_branch: BTreeMap<String, usize>,
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
}

impl StackHierarchy {
    fn new(nodes: &[PullRequestStackNode]) -> Self {
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

        sort_stack_node_indexes(&mut roots, nodes);
        for child_indexes in &mut children {
            sort_stack_node_indexes(child_indexes, nodes);
        }

        Self {
            indexes_by_branch,
            children,
            roots,
        }
    }
}

struct PullRequestStackTree<'a> {
    nodes: &'a [PullRequestStackNode],
    children: Vec<Vec<usize>>,
    ancestor_has_next: Vec<bool>,
    visited: BTreeSet<usize>,
    rows: Vec<PullRequestStackRow<'a>>,
}

impl<'a> PullRequestStackTree<'a> {
    fn new(nodes: &'a [PullRequestStackNode], children: Vec<Vec<usize>>) -> Self {
        Self {
            nodes,
            children,
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

        self.rows.push(PullRequestStackRow {
            node: &self.nodes[index],
            prefix: stack_tree_prefix(&self.ancestor_has_next, depth, has_next_sibling),
        });

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

fn stack_tree_prefix(ancestor_has_next: &[bool], depth: usize, has_next_sibling: bool) -> String {
    if depth == 0 {
        return String::new();
    }

    let mut prefix = String::new();
    for ancestor_has_next in ancestor_has_next {
        prefix.push_str(if *ancestor_has_next { "│  " } else { "   " });
    }
    prefix.push_str(if has_next_sibling {
        "├─ "
    } else {
        "└─ "
    });
    prefix
}

fn stack_indexes_by_branch(nodes: &[PullRequestStackNode]) -> BTreeMap<String, usize> {
    let mut indexes_by_branch = BTreeMap::new();
    for (index, node) in nodes.iter().enumerate() {
        indexes_by_branch
            .entry(node.branch.clone())
            .or_insert(index);
    }
    indexes_by_branch
}

fn sort_stack_node_indexes(indexes: &mut [usize], nodes: &[PullRequestStackNode]) {
    indexes.sort_by(|left, right| {
        stack_node_sort_key(&nodes[*left]).cmp(&stack_node_sort_key(&nodes[*right]))
    });
}

fn stack_node_sort_key(node: &PullRequestStackNode) -> (u8, u64, &str, &str) {
    (
        node.draft as u8,
        node.pull_request_number().unwrap_or(u64::MAX),
        node.title.as_str(),
        node.branch.as_str(),
    )
}

fn sort_stack_metadata_nodes(nodes: &mut [StackMetadataNode]) {
    nodes.sort_by(|left, right| stack_metadata_sort_key(left).cmp(&stack_metadata_sort_key(right)));
}

fn stack_metadata_sort_key(node: &StackMetadataNode) -> (u8, u64, &str, &str) {
    (
        node.draft as u8,
        node.pull_request.unwrap_or(u64::MAX),
        node.title.as_str(),
        node.branch.as_str(),
    )
}

fn pull_request_stack_sort_key(pull_request: &PullRequestRecord) -> (u8, u64, &str, &str) {
    (
        pull_request.draft as u8,
        pull_request.number,
        pull_request.title.as_str(),
        pull_request.head_branch.as_str(),
    )
}

fn pull_request_title(pull_request: &PullRequestRecord) -> &str {
    let title = pull_request.title.trim();
    if title.is_empty() {
        "(untitled)"
    } else {
        title
    }
}
