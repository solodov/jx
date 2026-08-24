use super::*;
use chrono::{DateTime, Duration, Utc};

const COMPLETED_STACK_STATUS_RETENTION_DAYS: i64 = 1;

/// Builds a read-only health report for a pull-request stack snapshot.
pub fn pull_request_stack_status_report(
    context: &RepositoryContext,
    snapshot: PullRequestStackSnapshot,
    statuses: Vec<PullRequestStatusRecord>,
    trunk: Option<RemoteStatusReport>,
) -> PullRequestStackStatusReport {
    let stack_status_config = context.config.repo.stack_status_for(&context.origin.github);
    PullRequestStackStatusReport {
        repository: repository_summary(context),
        snapshot,
        statuses: statuses
            .into_iter()
            .map(|status| apply_pull_request_status_policy(status, &stack_status_config))
            .map(|status| (status.number, status))
            .collect(),
        trunk,
        review_wait_threshold_seconds: stack_status_config.review_wait_threshold_seconds,
    }
}

/// Applies repository-specific PR status policy to raw GitHub check and review facts.
pub fn apply_pull_request_status_policy(
    status: PullRequestStatusRecord,
    config: &RepoStackStatusConfig,
) -> PullRequestStatusRecord {
    apply_pull_request_status_policy_inner(status, config, true)
}

fn apply_pull_request_status_policy_inner(
    mut status: PullRequestStatusRecord,
    config: &RepoStackStatusConfig,
    rewrite_labels: bool,
) -> PullRequestStatusRecord {
    status.title = config.rewrite_title(&status.title);
    let label_names = status
        .labels
        .iter()
        .map(|label| label.name.clone())
        .collect::<Vec<_>>();
    let had_checks = !status.checks.is_empty();
    apply_ignored_pull_request_status_facts(&mut status, config);

    if status.checks.is_empty() {
        if had_checks {
            status.check_status = aggregate_check_status(&[]);
        }
        if !config.review_gate_checks.is_empty() {
            status.review_status =
                review_status_with_review_gate(status.review_status, &[], config);
        }
        status.auto_merge_status =
            pull_request_auto_merge_status(&status, config, &label_names, false);
        if rewrite_labels {
            rewrite_pull_request_status_labels(&mut status, config);
        }
        return status;
    }

    let checks = latest_checks_by_name(&status.checks);
    let auto_merge_prerequisites_require_action =
        auto_merge_prerequisite_checks_require_action(&checks, config);
    let remaining_checks = checks
        .iter()
        .filter(|check| {
            !config.matches_review_gate_check(&check.name)
                && !config.matches_auto_merge_prerequisite_check(&check.name)
        })
        .cloned()
        .collect::<Vec<_>>();
    status.check_status = aggregate_required_check_status(&remaining_checks);
    status.checks = remaining_checks;

    if !config.review_gate_checks.is_empty() {
        status.review_status =
            review_status_with_review_gate(status.review_status, &checks, config);
    }
    status.auto_merge_status = pull_request_auto_merge_status(
        &status,
        config,
        &label_names,
        auto_merge_prerequisites_require_action,
    );
    if rewrite_labels {
        rewrite_pull_request_status_labels(&mut status, config);
    }
    status
}

/// Applies shared PR status policy plus review-only presentation filters.
pub fn apply_review_request_status_policy(
    status: PullRequestStatusRecord,
    stack_status_config: &RepoStackStatusConfig,
    review_config: &RepoReviewConfig,
) -> PullRequestStatusRecord {
    let mut status = apply_pull_request_status_policy_inner(status, stack_status_config, false);
    if !review_config.ignored_labels.is_empty()
        || !review_config.ignored_label_patterns.is_empty()
        || !review_config.hidden_labels.is_empty()
    {
        status.labels = status
            .labels
            .clone()
            .into_iter()
            .filter(|label| !review_config.hides_label(&status, &label.name))
            .collect();
    }
    rewrite_pull_request_status_labels(&mut status, stack_status_config);
    if !review_config.ignored_author_response_comments.is_empty() {
        status
            .reviewer_responses
            .retain(|response| !review_config.ignores_author_response_comment(&response.body_text));
    }
    status
}

fn rewrite_pull_request_status_labels(
    status: &mut PullRequestStatusRecord,
    config: &RepoStackStatusConfig,
) {
    if config.label_rewrites.is_empty() {
        return;
    }
    for label in &mut status.labels {
        label.name = config.rewrite_label(&label.name);
    }
}

fn pull_request_auto_merge_status(
    status: &PullRequestStatusRecord,
    config: &RepoStackStatusConfig,
    label_names: &[String],
    auto_merge_prerequisites_require_action: bool,
) -> PullRequestAutoMergeStatus {
    if !config.auto_merge_applies_to(status) {
        return PullRequestAutoMergeStatus::NotConfigured;
    }
    let auto_merge_label_present = label_names
        .iter()
        .any(|label| config.matches_auto_merge_label(status, label));
    if auto_merge_label_present && auto_merge_prerequisites_require_action {
        return PullRequestAutoMergeStatus::PrerequisitesRequired;
    }
    if auto_merge_label_present {
        return PullRequestAutoMergeStatus::Armed;
    }
    if auto_merge_prerequisites_require_action {
        return PullRequestAutoMergeStatus::NotConfigured;
    }
    if pull_request_status_is_stack_green(status) {
        PullRequestAutoMergeStatus::Missing
    } else {
        PullRequestAutoMergeStatus::NotConfigured
    }
}

/// Returns whether policy-normalized checks are green enough to avoid sync churn.
pub fn pull_request_status_has_green_stack_checks(status: &PullRequestStatusRecord) -> bool {
    pull_request_status_is_open_and_mergeable(status)
        && status.check_status == PullRequestCheckStatus::Passing
}

/// Returns whether a policy-normalized PR appears fully ready in stack status.
pub fn pull_request_status_is_stack_green(status: &PullRequestStatusRecord) -> bool {
    pull_request_status_has_green_stack_checks(status)
        && status.review_status == PullRequestReviewStatus::Approved
}

fn pull_request_status_is_open_and_mergeable(status: &PullRequestStatusRecord) -> bool {
    !status.draft
        && !status.merged
        && !status.closed
        && status.merge_status == PullRequestMergeStatus::Mergeable
}

fn apply_ignored_pull_request_status_facts(
    status: &mut PullRequestStatusRecord,
    config: &RepoStackStatusConfig,
) {
    if !config.ignored_checks.is_empty() {
        status
            .checks
            .retain(|check| !config.ignores_check(&check.name));
    }
    if !config.ignored_labels.is_empty()
        || !config.ignored_label_patterns.is_empty()
        || !config.ignored_labels_when_merged.is_empty()
        || !config.hidden_labels.is_empty()
        || !config.auto_merge_labels.is_empty()
    {
        status.labels = status
            .labels
            .clone()
            .into_iter()
            .filter(|label| !config.hides_label(status, &label.name))
            .collect();
    }
    if config.ignored_reviewers.is_empty() {
        return;
    }

    status
        .requested_reviewers
        .users
        .retain(|reviewer| !config.ignores_reviewer(reviewer));
    status.requested_reviewers.teams.retain(|team| {
        !config.ignores_reviewer(team) && !config.ignores_reviewer(&format!("team/{team}"))
    });
    status
        .suggested_reviewers
        .retain(|reviewer| !config.ignores_reviewer(reviewer));
    status
        .approved_reviewers
        .retain(|reviewer| !config.ignores_reviewer(reviewer));
    status
        .changes_requested_reviewers
        .retain(|reviewer| !config.ignores_reviewer(reviewer));
    status
        .commented_reviewers
        .retain(|reviewer| !config.ignores_reviewer(reviewer));
    status
        .addressed_reviewers
        .retain(|reviewer| !config.ignores_reviewer(reviewer));
    status
        .reviewer_responses
        .retain(|response| !config.ignores_reviewer(&response.reviewer));
    status
        .reviewer_mentions
        .retain(|mention| !config.ignores_reviewer(&mention.reviewer));
    status
        .dismissed_reviewers
        .retain(|reviewer| !config.ignores_reviewer(reviewer));
    status
        .review_activity
        .retain(|activity| !config.ignores_reviewer(&activity.reviewer));
    status
        .timeline_events
        .retain(|event| match event.reviewer.as_deref() {
            Some(reviewer) => !config.ignores_reviewer(reviewer),
            None => true,
        });
}

/// Collapses duplicate rollup contexts so superseded failures do not outvote newer results.
fn latest_checks_by_name(checks: &[PullRequestCheck]) -> Vec<PullRequestCheck> {
    let mut latest = Vec::<PullRequestCheck>::new();
    for check in checks {
        if let Some(existing) = latest
            .iter_mut()
            .find(|existing| existing.name == check.name)
        {
            *existing = check.clone();
        } else {
            latest.push(check.clone());
        }
    }
    latest
}

fn aggregate_check_status(checks: &[PullRequestCheck]) -> PullRequestCheckStatus {
    if checks.is_empty() {
        return PullRequestCheckStatus::Missing;
    }
    if checks
        .iter()
        .any(|check| check.status == PullRequestCheckStatus::Failing)
    {
        return PullRequestCheckStatus::Failing;
    }
    if checks
        .iter()
        .any(|check| check.status == PullRequestCheckStatus::Pending)
    {
        return PullRequestCheckStatus::Pending;
    }
    if checks
        .iter()
        .any(|check| check.status == PullRequestCheckStatus::Unknown)
    {
        return PullRequestCheckStatus::Unknown;
    }
    PullRequestCheckStatus::Passing
}

fn aggregate_required_check_status(checks: &[PullRequestCheck]) -> PullRequestCheckStatus {
    let required_checks = checks
        .iter()
        .filter(|check| check.required)
        .cloned()
        .collect::<Vec<_>>();
    if required_checks.is_empty() && !checks.is_empty() {
        return PullRequestCheckStatus::Passing;
    }
    aggregate_check_status(&required_checks)
}

fn auto_merge_prerequisite_checks_require_action(
    checks: &[PullRequestCheck],
    config: &RepoStackStatusConfig,
) -> bool {
    checks.iter().any(|check| {
        config.matches_auto_merge_prerequisite_check(&check.name)
            && check.status != PullRequestCheckStatus::Passing
    })
}

fn review_status_with_review_gate(
    status: PullRequestReviewStatus,
    checks: &[PullRequestCheck],
    config: &RepoStackStatusConfig,
) -> PullRequestReviewStatus {
    match status {
        PullRequestReviewStatus::ChangesRequested => PullRequestReviewStatus::ChangesRequested,
        PullRequestReviewStatus::ReviewRequired => PullRequestReviewStatus::ReviewRequired,
        _ if review_gate_checks_approve(checks, config) => PullRequestReviewStatus::Approved,
        _ => PullRequestReviewStatus::ReviewRequested,
    }
}

fn review_gate_checks_approve(checks: &[PullRequestCheck], config: &RepoStackStatusConfig) -> bool {
    config.review_gate_checks.iter().all(|rule| {
        let mut found_match = false;
        for check in checks.iter().filter(|check| rule.matches(&check.name)) {
            found_match = true;
            if check.status != PullRequestCheckStatus::Passing {
                return false;
            }
        }
        found_match
    })
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
        let mut label = self.compact_prefix();
        label.push_str(self.status_symbol());
        label.push(' ');
        if let Some(number) = self.node.pull_request_number() {
            label.push_str(&format!("#{number:<6} "));
        }
        label.push_str(self.display_title());
        label
    }

    /// Returns the compact tree prefix used by terminal and Markdown renderers.
    pub fn compact_prefix(&self) -> String {
        compact_stack_tree_prefix(&self.prefix)
    }
}

/// Compacts tree drawing whitespace so stack connectors stay readable in fixed-width views.
pub fn compact_stack_tree_prefix(prefix: &str) -> String {
    prefix
        .replace("│  ", "│ ")
        .replace("   ", "  ")
        .replace("├─ ", "├ ")
        .replace("└─ ", "└ ")
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

    stack_metadata_with_nodes(existing_metadata, nodes)
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

    stack_metadata_with_nodes(existing_metadata, nodes)
}

/// Refreshes cached PR status facts and prunes rows that no longer identify a pull request.
pub fn maintain_stack_metadata_pull_request_statuses(
    statuses: &[PullRequestStatusRecord],
    existing_metadata: &StackMetadata,
) -> StackMetadata {
    maintain_stack_metadata_pull_request_statuses_at(statuses, existing_metadata, Utc::now())
}

/// Refreshes status metadata with an explicit clock for deterministic retention tests.
pub fn maintain_stack_metadata_pull_request_statuses_at(
    statuses: &[PullRequestStatusRecord],
    existing_metadata: &StackMetadata,
    now: DateTime<Utc>,
) -> StackMetadata {
    let refreshed = refresh_stack_metadata_pull_request_statuses(statuses, existing_metadata);
    let with_pull_requests_only = prune_unresolved_stack_metadata_nodes(&refreshed);
    let without_expired_closed =
        prune_expired_closed_stack_metadata_nodes(statuses, &with_pull_requests_only, now);
    prune_expired_merged_stack_metadata_trees(statuses, &without_expired_closed, now)
}

/// Refreshes durable stack metadata from read-only PR status facts matched by PR number or branch.
pub fn refresh_stack_metadata_pull_request_statuses(
    statuses: &[PullRequestStatusRecord],
    existing_metadata: &StackMetadata,
) -> StackMetadata {
    let statuses_by_number = statuses
        .iter()
        .map(|status| (status.number, status))
        .collect::<BTreeMap<_, _>>();
    let statuses_by_head_branch = statuses
        .iter()
        .map(|status| (status.head_branch.as_str(), status))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = existing_metadata
        .nodes
        .iter()
        .map(|node| {
            let status = node
                .pull_request
                .and_then(|number| statuses_by_number.get(&number).copied())
                .or_else(|| statuses_by_head_branch.get(node.branch.as_str()).copied());
            match status {
                Some(status) => refreshed_stack_metadata_node_from_status(node, status),
                None => node.clone(),
            }
        })
        .collect::<Vec<_>>();
    sort_stack_metadata_nodes(&mut nodes);

    stack_metadata_with_nodes(existing_metadata, nodes)
}

/// Drops branch-only rows after status lookup fails to attach a PR identity.
fn prune_unresolved_stack_metadata_nodes(metadata: &StackMetadata) -> StackMetadata {
    let removed_branches = metadata
        .nodes
        .iter()
        .filter(|node| node.pull_request.is_none())
        .map(|node| node.branch.clone())
        .collect::<BTreeSet<_>>();
    if removed_branches.is_empty() {
        return metadata.clone();
    }

    let mut nodes = metadata
        .nodes
        .iter()
        .filter(|node| node.pull_request.is_some())
        .cloned()
        .map(|mut node| {
            if node
                .parent_branch
                .as_ref()
                .is_some_and(|parent| removed_branches.contains(parent))
            {
                node.parent_branch = None;
                node.parent_pull_request = None;
            }
            node
        })
        .collect::<Vec<_>>();
    sort_stack_metadata_nodes(&mut nodes);

    stack_metadata_with_nodes(metadata, nodes)
}

/// Drops closed-unmerged PR nodes after the reminder retention window expires.
fn prune_expired_closed_stack_metadata_nodes(
    statuses: &[PullRequestStatusRecord],
    metadata: &StackMetadata,
    now: DateTime<Utc>,
) -> StackMetadata {
    let expired_closed_numbers = statuses
        .iter()
        .filter(|status| status.closed && !status.merged)
        .filter(|status| {
            status
                .closed_at
                .as_deref()
                .is_some_and(|closed_at| timestamp_is_outside_completed_retention(closed_at, now))
        })
        .map(|status| status.number)
        .collect::<BTreeSet<_>>();
    if expired_closed_numbers.is_empty() {
        return metadata.clone();
    }

    prune_stack_metadata_nodes_by_number(metadata, &expired_closed_numbers)
}

/// Drops stack components whose stored PR nodes are all merged.
pub fn prune_merged_stack_metadata_trees(metadata: &StackMetadata) -> StackMetadata {
    prune_merged_stack_metadata_trees_by(metadata, |_, _| true)
}

fn prune_expired_merged_stack_metadata_trees(
    statuses: &[PullRequestStatusRecord],
    metadata: &StackMetadata,
    now: DateTime<Utc>,
) -> StackMetadata {
    let statuses_by_number = statuses
        .iter()
        .map(|status| (status.number, status))
        .collect::<BTreeMap<_, _>>();
    prune_merged_stack_metadata_trees_by(metadata, |component, snapshot| {
        component.iter().all(|index| {
            let Some(number) = snapshot.nodes[*index].pull_request_number() else {
                return false;
            };
            statuses_by_number
                .get(&number)
                .and_then(|status| status.merged_at.as_deref())
                .is_some_and(|merged_at| timestamp_is_outside_completed_retention(merged_at, now))
        })
    })
}

fn prune_merged_stack_metadata_trees_by(
    metadata: &StackMetadata,
    should_prune: impl Fn(&BTreeSet<usize>, &PullRequestStackSnapshot) -> bool,
) -> StackMetadata {
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
        if component.iter().all(|index| snapshot.nodes[*index].merged)
            && should_prune(&component, &snapshot)
        {
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

    stack_metadata_with_nodes(metadata, nodes)
}

fn prune_stack_metadata_nodes_by_number(
    metadata: &StackMetadata,
    numbers: &BTreeSet<u64>,
) -> StackMetadata {
    let removed_branches = metadata
        .nodes
        .iter()
        .filter(|node| {
            node.pull_request
                .is_some_and(|number| numbers.contains(&number))
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
                .is_none_or(|number| !numbers.contains(&number))
        })
        .cloned()
        .map(|mut node| {
            if node
                .parent_branch
                .as_ref()
                .is_some_and(|parent| removed_branches.contains(parent))
                || node
                    .parent_pull_request
                    .is_some_and(|number| numbers.contains(&number))
            {
                node.parent_branch = None;
                node.parent_pull_request = None;
            }
            node
        })
        .collect::<Vec<_>>();
    sort_stack_metadata_nodes(&mut nodes);

    stack_metadata_with_nodes(metadata, nodes)
}

fn timestamp_is_outside_completed_retention(timestamp: &str, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|timestamp| {
            now.signed_duration_since(timestamp.with_timezone(&Utc))
                >= Duration::days(COMPLETED_STACK_STATUS_RETENTION_DAYS)
        })
        .unwrap_or(false)
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

    stack_metadata_with_nodes(existing_metadata, nodes)
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

    stack_metadata_with_nodes(existing_metadata, nodes)
}

fn stack_metadata_with_nodes(
    existing_metadata: &StackMetadata,
    nodes: Vec<StackMetadataNode>,
) -> StackMetadata {
    StackMetadata {
        version: 1,
        work_item_handler_runs: existing_metadata.work_item_handler_runs.clone(),
        nodes,
    }
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
            // A completed parent may have been rebased out of GitHub's live base branch;
            // keep that historical edge so open descendants stay in the same stack.
            .or_else(|| existing_completed_parent_branch(existing_node, existing_nodes_by_branch))
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
        merged: existing_node
            .map(|node| refreshed_pull_request_merged_state(node, pull_request.merged))
            .unwrap_or(pull_request.merged),
        work_ids: existing_node
            .map(|node| node.work_ids.clone())
            .unwrap_or_default(),
        fixes_work_ids: existing_node
            .map(|node| node.fixes_work_ids.clone())
            .unwrap_or_default(),
    }
}

fn stack_metadata_node_from_local_branch(
    local: &LocalStackBranch,
    existing_nodes_by_branch: &BTreeMap<&str, &StackMetadataNode>,
) -> StackMetadataNode {
    let existing_node = existing_nodes_by_branch.get(local.branch.as_str());
    let parent_branch = local
        .parent_branch
        .clone()
        .or_else(|| existing_completed_parent_branch(existing_node, existing_nodes_by_branch));
    let parent_pull_request = parent_branch
        .as_deref()
        .and_then(|branch| {
            existing_nodes_by_branch
                .get(branch)
                .and_then(|node| node.pull_request)
        })
        .or_else(|| {
            existing_node.and_then(|node| {
                (node.parent_branch.as_deref() == parent_branch.as_deref())
                    .then_some(node.parent_pull_request)
                    .flatten()
            })
        });

    StackMetadataNode {
        branch: local.branch.clone(),
        base_branch: local.base_branch.clone(),
        parent_branch,
        pull_request: existing_node.and_then(|node| node.pull_request),
        parent_pull_request,
        title: existing_node
            .map(|node| node.title.clone())
            .unwrap_or_else(|| local_stack_branch_title(local)),
        url: existing_node.and_then(|node| node.url.clone()),
        draft: existing_node.is_some_and(|node| node.draft),
        merged: existing_node.is_some_and(|node| node.merged),
        work_ids: existing_node
            .map(|node| node.work_ids.clone())
            .unwrap_or_default(),
        fixes_work_ids: existing_node
            .map(|node| node.fixes_work_ids.clone())
            .unwrap_or_default(),
    }
}

fn existing_completed_parent_branch(
    existing_node: Option<&&StackMetadataNode>,
    existing_nodes_by_branch: &BTreeMap<&str, &StackMetadataNode>,
) -> Option<String> {
    existing_node
        .and_then(|node| node.parent_branch.as_deref())
        .filter(|parent_branch| {
            existing_nodes_by_branch
                .get(parent_branch)
                .is_some_and(|parent| parent.merged)
        })
        .map(str::to_owned)
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
        merged: refreshed_pull_request_merged_state(node, pull_request.merged),
        work_ids: node.work_ids.clone(),
        fixes_work_ids: node.fixes_work_ids.clone(),
    }
}

fn refreshed_pull_request_merged_state(node: &StackMetadataNode, merged: bool) -> bool {
    // Only status maintenance should advance fixing PRs to merged, so configured
    // work-item handlers still see the false -> true transition.
    if merged && !node.merged && !node.fixes_work_ids.is_empty() {
        false
    } else {
        merged
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
        work_ids: node.work_ids.clone(),
        fixes_work_ids: node.fixes_work_ids.clone(),
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
    sort_stack_root_indexes_newest_first(&mut unvisited, nodes, &tree.children);
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
    sort_stack_root_indexes_newest_first(&mut roots, nodes, &hierarchy.children);

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

        sort_stack_root_indexes_newest_first(&mut roots, nodes, &children);
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

fn sort_stack_root_indexes_newest_first(
    indexes: &mut [usize],
    nodes: &[PullRequestStackNode],
    children: &[Vec<usize>],
) {
    indexes.sort_by(|left, right| {
        stack_root_sort_key(*right, nodes, children)
            .cmp(&stack_root_sort_key(*left, nodes, children))
    });
}

fn stack_root_sort_key<'a>(
    index: usize,
    nodes: &'a [PullRequestStackNode],
    children: &[Vec<usize>],
) -> (u64, &'a str, &'a str) {
    (
        stack_newest_pull_request_number(index, nodes, children, &mut BTreeSet::new()).unwrap_or(0),
        nodes[index].title.as_str(),
        nodes[index].branch.as_str(),
    )
}

fn stack_newest_pull_request_number(
    index: usize,
    nodes: &[PullRequestStackNode],
    children: &[Vec<usize>],
    visited: &mut BTreeSet<usize>,
) -> Option<u64> {
    if !visited.insert(index) {
        return None;
    }
    children[index]
        .iter()
        .filter_map(|child| stack_newest_pull_request_number(*child, nodes, children, visited))
        .chain(nodes[index].pull_request_number())
        .max()
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
