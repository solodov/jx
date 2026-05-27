use super::*;

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

        let mut live_pull_requests = live_pull_requests.to_vec();
        live_pull_requests.sort_by(|left, right| {
            pull_request_stack_sort_key(left).cmp(&pull_request_stack_sort_key(right))
        });
        for pull_request in &live_pull_requests {
            match indexes_by_branch
                .get(pull_request.head_branch.as_str())
                .copied()
            {
                Some(index) => nodes[index].apply_live_pull_request(pull_request),
                None => {
                    let index = nodes.len();
                    nodes.push(PullRequestStackNode::from_live_pull_request(
                        pull_request,
                        &local_branches,
                    ));
                    indexes_by_branch.insert(pull_request.head_branch.clone(), index);
                }
            }
        }

        resolve_live_parent_edges(&mut nodes, &indexes_by_branch);
        apply_current_selection(nodes, selection)
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

fn pull_request_stack_sort_key(pull_request: &PullRequestRecord) -> (u8, u64, &str, &str) {
    (
        pull_request.draft as u8,
        pull_request.number,
        pull_request.title.as_str(),
        pull_request.head_branch.as_str(),
    )
}
