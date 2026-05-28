use super::*;

/// Configured Git remote discovered through the jj Git backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRemote {
    pub name: String,
    pub url: String,
}

/// One jj workspace and its root path for layout management commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub name: String,
    pub root: PathBuf,
    pub is_current: bool,
}

/// Request to add a jj workspace at an already resolved layout destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceAddOptions {
    pub name: String,
    pub destination: PathBuf,
    pub revision: Option<String>,
    pub shared_paths: Vec<String>,
}

/// Request to forget a jj workspace, delete it, and prune empty managed parents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRemoveOptions {
    pub name: String,
    pub root: PathBuf,
    pub cleanup_root: PathBuf,
}

/// Result of rewriting a selected commit description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDescriptionRewrite {
    pub commit_id: String,
    pub changed: bool,
}

/// Commit selected to seed a newly created remote repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialPublishTarget {
    pub commit_id: String,
    pub short_commit_id: String,
    pub description: String,
}

/// Result of pushing the initial `main` branch after repository creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapPushOutcome {
    pub branch: String,
    pub short_commit_id: String,
    pub description: String,
    pub working_copy_short_commit_id: Option<String>,
}

/// Snapshot of the jj working-copy commit after jj has captured pending disk changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingCopySnapshot {
    pub commit_id: String,
}

/// Status of the current working-copy commit rendered by `jj status` plus its description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceStatus {
    pub commit_lines: Vec<String>,
    pub description: String,
    pub change_lines: Vec<String>,
    pub extra_lines: Vec<String>,
}

/// Read-side facts exposed by the jj workspace boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFacts {
    pub workspace_root: PathBuf,
    pub target_change: ChangeSummary,
    pub trunk: TrunkSummary,
    pub trunk_git_commit_sha: String,
    pub origin_branch: String,
    pub local_bookmarks: Vec<String>,
    pub local_bookmarks_at_target: Vec<String>,
    /// Nearest local bookmark on the selected stack after trunk, excluding trunk itself.
    pub nearest_ancestor_bookmark: Option<String>,
    /// Repo-root-relative file paths changed by the selected jj commit.
    pub changed_files: Vec<String>,
    /// Status-style changed-file lines for operator-facing previews.
    pub change_lines: Vec<String>,
    /// Zero-based index of the selected change on the linear path after trunk.
    pub stack_index: usize,
}

/// Revisions that define which stack changes should be published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackPublishSelection {
    /// Publish the full local stack containing the selected anchor, or the working copy.
    InferredStack { anchor: Option<String> },
    /// Publish exactly the commits matched by the supplied jj revsets.
    ExplicitRevisions { revisions: Vec<String> },
}

/// Local jj facts for a stack publish operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPublishFacts {
    pub nodes: Vec<StackPublishNodeFacts>,
    pub publish_indexes: Vec<usize>,
    pub anchor_index: Option<usize>,
    pub metrics: StackPublishMetrics,
}

/// Counters and timings captured while deriving stack publish facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackPublishMetrics {
    pub target_resolution_count: usize,
    pub resolved_revision_count: usize,
    pub resolved_trunk_count: usize,
    pub stack_path_count: usize,
    pub collected_child_count: usize,
    pub loaded_child_count: usize,
    pub workspace_fact_count: usize,
    pub node_count: usize,
    pub publish_count: usize,
    pub target_resolution_us: u64,
    pub resolve_revisions_us: u64,
    pub resolve_trunk_us: u64,
    pub linear_stack_path_us: u64,
    pub collect_child_ids_us: u64,
    pub load_child_commit_us: u64,
    pub workspace_facts_us: u64,
    pub total_us: u64,
}

/// One change in the local stack containing the publish selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPublishNodeFacts {
    pub workspace: WorkspaceFacts,
    pub parent_index: Option<usize>,
}

/// Revisions that define a read-only stack plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackPlanSelection {
    /// Show the local stack neighbourhood containing the selected anchor, or the working copy.
    InferredStack { anchor: Option<String> },
    /// Show the neighbourhood for exactly the commits matched by the supplied jj revsets.
    ExplicitRevisions { revisions: Vec<String> },
}

/// Local jj facts for a read-only stack plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPlanFacts {
    pub trunk: TrunkSummary,
    pub nodes: Vec<StackPlanNodeFacts>,
    pub selected_indexes: Vec<usize>,
    pub anchor_index: Option<usize>,
}

/// One change in the local stack neighbourhood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPlanNodeFacts {
    pub workspace: WorkspaceFacts,
    pub parent_index: Option<usize>,
}

/// Local cached remote-trunk facts used by `jx remote-status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusWorkspaceFacts {
    pub remotes: Vec<StatusRemoteFacts>,
}

/// Local cached remote-trunk facts with timing detail for command-level perf tracing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusWorkspaceFactsWithMetrics {
    pub facts: StatusWorkspaceFacts,
    pub metrics: StatusWorkspaceMetrics,
}

/// Timing detail for local status fact collection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusWorkspaceMetrics {
    pub current_commit_us: u64,
    pub remotes: Vec<StatusRemoteMetrics>,
}

/// Timing detail for resolving one remote's local status facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRemoteMetrics {
    pub remote: String,
    pub branch: String,
    pub stack_path_len: usize,
    pub non_empty_count: i64,
    pub resolve_trunk_us: u64,
    pub trunk: TrunkResolveMetrics,
    pub linear_stack_path_us: u64,
    pub count_non_empty_commits_us: u64,
}

/// Timing and cardinality detail for resolving a remote trunk commit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrunkResolveMetrics {
    pub fast_path: bool,
    pub preferred_branch_check_count: usize,
    pub remote_bookmark_count: usize,
    pub conflicted_bookmark_count: usize,
    pub normal_bookmark_count: usize,
    pub ancestor_check_count: usize,
    pub candidate_count: usize,
    pub scan_remote_bookmarks_us: u64,
    pub select_candidate_us: u64,
    pub load_trunk_commit_us: u64,
}

/// Local cached trunk facts for one configured remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRemoteFacts {
    pub remote: String,
    pub branch: String,
    pub trunk_git_commit_sha: String,
    pub trunk_short_commit_id: String,
    pub local_ahead_by: i64,
}

/// Summary of a jj change/commit relevant to workflow planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSummary {
    /// User-facing jj change id, using jj's reverse-hex alphabet.
    pub change_id: String,
    pub commit_id: String,
    pub short_commit_id: String,
    pub description: String,
    pub is_empty: bool,
}

/// Summary of the resolved trunk commit and its origin branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrunkSummary {
    pub branch: String,
    pub commit_id: String,
    pub short_commit_id: String,
}

/// One timed fetch substep emitted as soon as it completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchTraceStep {
    pub name: String,
    pub duration_us: u64,
    pub attrs: Vec<FetchTraceAttr>,
    pub error: Option<String>,
}

/// One structured fetch trace attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchTraceAttr {
    pub key: String,
    pub value: FetchTraceValue,
}

/// Creates a structured fetch trace attribute.
pub fn fetch_trace_attr(
    key: impl Into<String>,
    value: impl Into<FetchTraceValue>,
) -> FetchTraceAttr {
    FetchTraceAttr {
        key: key.into(),
        value: value.into(),
    }
}

/// Scalar values supported by fetch tracing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchTraceValue {
    String(String),
    U64(u64),
    I64(i64),
    Bool(bool),
}

impl From<&str> for FetchTraceValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for FetchTraceValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&String> for FetchTraceValue {
    fn from(value: &String) -> Self {
        Self::String(value.clone())
    }
}

impl From<usize> for FetchTraceValue {
    fn from(value: usize) -> Self {
        Self::U64(value as u64)
    }
}

impl From<u64> for FetchTraceValue {
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}

impl From<i64> for FetchTraceValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

impl From<bool> for FetchTraceValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

/// Outcome of fetching and importing fixed `origin` through the jj boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOutcome {
    pub branch: String,
    pub changed_remote_bookmarks: usize,
    pub changed_remote_tags: usize,
    pub abandoned_commits: usize,
    pub rebased_trunk_children: usize,
    pub rebased_descendants: usize,
    pub skipped_trunk_children: usize,
    pub current_repaired: bool,
    pub rebased_commits: Vec<RebasedCommitSummary>,
}

/// Result of optional sync preparation that publishes the latest complete trunk descendant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceTrunkOutcome {
    pub branch: String,
    pub old_short_commit_id: String,
    pub new_short_commit_id: String,
    pub current_updated: bool,
}

/// Workspace stacks through which a commit is visible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceVisibility {
    pub names: Vec<String>,
    pub includes_current: bool,
}

/// One commit rewritten by fetch so local work sits on the updated origin trunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebasedCommitSummary {
    pub old_short_commit_id: String,
    pub new_short_commit_id: String,
    pub description: String,
    pub has_conflict: bool,
    pub is_empty: bool,
    pub workspace_visibility: WorkspaceVisibility,
}

/// Destination for moving the current jj stack through `jx stack`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackMoveTarget {
    Onto(String),
    Trunk,
}

/// Outcome of moving the current jj change and descendants to a new parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackMoveOutcome {
    pub source_short_commit_id: String,
    pub target_short_commit_id: String,
    pub rebased_commits: usize,
    pub skipped_commits: usize,
    pub current_updated: bool,
}

/// Local branch ancestry derived from jj after stack mutations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStackBranch {
    pub branch: String,
    pub base_branch: String,
    pub parent_branch: Option<String>,
    pub title: String,
}

/// Local branch ancestry plus jj-internal timings for performance tracing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStackBranchFacts {
    pub branches: Vec<LocalStackBranch>,
    pub metrics: LocalStackBranchMetrics,
}

impl LocalStackBranchFacts {
    pub fn from_branches(branches: Vec<LocalStackBranch>) -> Self {
        let branch_count = branches.len();
        Self {
            branches,
            metrics: LocalStackBranchMetrics {
                branch_count,
                ..LocalStackBranchMetrics::default()
            },
        }
    }
}

/// Counters and timings captured while deriving local stack branch ancestry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalStackBranchMetrics {
    pub local_bookmark_count: usize,
    pub normal_bookmark_count: usize,
    pub skipped_non_normal_bookmark_count: usize,
    pub loaded_commit_count: usize,
    pub resolved_trunk_count: usize,
    pub skipped_missing_trunk_count: usize,
    pub stack_path_count: usize,
    pub skipped_non_linear_count: usize,
    pub skipped_trunk_count: usize,
    pub branch_count: usize,
    pub enumerate_bookmarks_us: u64,
    pub load_commit_us: u64,
    pub resolve_trunk_us: u64,
    pub linear_stack_path_us: u64,
    pub nearest_ancestor_bookmark_us: u64,
    pub sort_dedup_us: u64,
    pub total_us: u64,
}

/// Result of ensuring a planned bookmark points at the selected jj change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkUpdate {
    pub branch: String,
    pub created: bool,
}

/// Result of pushing a local bookmark to fixed `origin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushOutcome {
    pub branch: String,
    pub pushed_refs: usize,
    pub pushed_commits: Vec<PushedCommitSummary>,
}

/// Bulk bookmark push outcome plus jj-internal phase timings for performance tracing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushBookmarksOutcome {
    pub outcomes: Vec<PushOutcome>,
    pub metrics: PushBookmarksMetrics,
}

impl PushBookmarksOutcome {
    pub fn from_outcomes(outcomes: Vec<PushOutcome>) -> Self {
        let mut metrics = PushBookmarksMetrics {
            branch_count: outcomes.len(),
            ..PushBookmarksMetrics::default()
        };
        for outcome in &outcomes {
            if outcome.pushed_refs == 0 {
                metrics.no_op_branch_count += 1;
            } else {
                metrics.update_count += 1;
                metrics.pushed_ref_count += outcome.pushed_refs;
            }
            metrics.pushed_commit_count += outcome.pushed_commits.len();
        }
        Self { outcomes, metrics }
    }
}

/// Counters and timings captured while pushing selected bookmarks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushBookmarksMetrics {
    pub branch_count: usize,
    pub update_count: usize,
    pub no_op_branch_count: usize,
    pub pushed_ref_count: usize,
    pub pushed_commit_count: usize,
    pub classify_updates_us: u64,
    pub pushed_commits_for_updates_us: u64,
    pub git_push_refs_us: u64,
    pub export_git_refs_us: u64,
    pub commit_transaction_us: u64,
    pub total_us: u64,
}

/// Result of pushing tracked bookmark state to fixed `origin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedPushOutcome {
    pub pushed_refs: usize,
    pub bookmarks: Vec<PushedBookmarkSummary>,
    pub pushed_commits: Vec<PushedCommitSummary>,
}

/// Options that control experimental sync push behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncPushOptions {
    /// Skips pushing local heads whose file tree already matches GitHub.
    pub skip_same_tree_pushes: bool,
}

/// Result of pushing only the tracked bookmarks that can be safely synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPushOutcome {
    pub pushed: TrackedPushOutcome,
    pub skipped_conflicted_bookmarks: Vec<SkippedPushBookmarkSummary>,
    pub skipped_same_tree_bookmarks: Vec<SkippedSameTreeBookmarkSummary>,
}

/// Sync push outcome plus jj-internal phase timings for performance tracing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPushMetricsOutcome {
    pub outcome: SyncPushOutcome,
    pub metrics: SyncPushMetrics,
}

impl SyncPushMetricsOutcome {
    pub fn from_outcome(outcome: SyncPushOutcome) -> Self {
        let metrics = SyncPushMetrics {
            pushed_ref_count: outcome.pushed.pushed_refs,
            pushed_bookmark_count: outcome.pushed.bookmarks.len(),
            pushed_commit_count: outcome.pushed.pushed_commits.len(),
            skipped_conflicted_count: outcome.skipped_conflicted_bookmarks.len(),
            skipped_same_tree_count: outcome.skipped_same_tree_bookmarks.len(),
            adopted_remote_head_count: outcome
                .skipped_same_tree_bookmarks
                .iter()
                .filter(|bookmark| bookmark.adopted_remote_head)
                .count(),
            ..SyncPushMetrics::default()
        };
        Self { outcome, metrics }
    }
}

/// Counters and timings captured while pushing syncable tracked bookmarks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPushMetrics {
    pub tracked_update_count: usize,
    pub pushable_update_count: usize,
    pub skipped_conflicted_count: usize,
    pub skipped_same_tree_count: usize,
    pub adopted_remote_head_count: usize,
    pub pushed_ref_count: usize,
    pub pushed_bookmark_count: usize,
    pub unchanged_bookmark_count: usize,
    pub pushed_commit_count: usize,
    pub tracked_origin_bookmark_updates_us: u64,
    pub split_conflicted_updates_us: u64,
    pub push_tracked_updates_us: u64,
    pub tracked_push_trunk_us: u64,
    pub pushed_bookmark_summaries_us: u64,
    pub pushed_commits_for_updates_us: u64,
    pub git_push_refs_us: u64,
    pub export_git_refs_us: u64,
    pub commit_transaction_us: u64,
    pub unchanged_tracked_bookmark_summaries_us: u64,
    pub total_us: u64,
}

/// One tracked bookmark skipped by sync because its push range contains conflicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPushBookmarkSummary {
    pub branch: String,
    pub conflicted_commits: Vec<ConflictedCommitSummary>,
}

/// One tracked bookmark whose local and remote heads have identical file trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedSameTreeBookmarkSummary {
    pub branch: String,
    pub local_short_commit_id: String,
    pub remote_short_commit_id: String,
    pub adopted_remote_head: bool,
}

/// One conflicted commit that prevents a bookmark from being synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictedCommitSummary {
    pub short_commit_id: String,
    pub description: String,
    pub workspace_visibility: WorkspaceVisibility,
}

/// One pushed bookmark update rendered by `jx push --tracked` and `jx sync`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushedBookmarkSummary {
    pub branch: String,
    pub old_short_commit_id: Option<String>,
    pub new_short_commit_id: Option<String>,
    pub old_description: Option<String>,
    pub new_description: Option<String>,
    pub pull_request_description: Option<String>,
    pub pull_request_base: Option<String>,
    pub new_workspace_visibility: WorkspaceVisibility,
}

/// One commit made reachable on the remote by a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushedCommitSummary {
    pub short_commit_id: String,
    pub description: String,
}

pub(super) fn short_commit_id(commit_id: &CommitId) -> String {
    commit_id.hex().chars().take(SHORT_COMMIT_ID_LEN).collect()
}
