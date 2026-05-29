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

/// Outcome of rebasing a jj source revision onto the fixed origin trunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseOnTrunkOutcome {
    pub branch: String,
    pub source_short_commit_ids: Vec<String>,
    pub trunk_short_commit_id: String,
    pub rebased_commits: usize,
    pub skipped_commits: usize,
    pub current_updated: bool,
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

/// Result of pushing tracked bookmark state to fixed `origin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedPushOutcome {
    pub pushed_refs: usize,
    pub bookmarks: Vec<PushedBookmarkSummary>,
    pub pushed_commits: Vec<PushedCommitSummary>,
}

/// Result of pushing only the tracked bookmarks that can be safely synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPushOutcome {
    pub pushed: TrackedPushOutcome,
    pub skipped_conflicted_bookmarks: Vec<SkippedPushBookmarkSummary>,
}

/// One tracked bookmark skipped by sync because its push range contains conflicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPushBookmarkSummary {
    pub branch: String,
    pub conflicted_commits: Vec<ConflictedCommitSummary>,
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
