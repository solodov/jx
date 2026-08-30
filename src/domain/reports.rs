use super::*;

/// Repository facts safe for concise command rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySummary {
    pub origin_name: &'static str,
    pub origin_url: String,
    pub github_slug: String,
    pub github_url: String,
    pub token_source: String,
    pub config: &'static str,
    pub default_reviewers: String,
}

/// Successful readiness result for `jx check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub repository: RepositorySummary,
    pub workspace: CheckWorkspaceSummary,
    pub github: GitHubReadiness,
    pub bookmark: BookmarkPlan,
}

/// jj workspace facts rendered by the non-mutating readiness check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckWorkspaceSummary {
    pub trunk_branch: String,
    pub trunk_short_commit_id: String,
    pub current_short_commit_id: String,
    pub current_is_empty: bool,
    pub stack_index: usize,
}

/// GitHub facts rendered by the non-mutating readiness check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubReadiness {
    pub login: String,
    pub default_branch: Option<String>,
    pub can_push: bool,
}

/// Planned same-repository bookmark intent for stack PR publishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkReport {
    pub repository: RepositorySummary,
    pub task_id: Option<String>,
    pub bookmark: BookmarkPlan,
}

/// Result of fetching fixed `origin` through the jj boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchReport {
    pub repository: RepositorySummary,
    pub outcome: FetchOutcome,
}

/// Planned PR data derived before any jj or GitHub mutation happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestPlan {
    pub repository: RepositorySummary,
    pub task_id: Option<String>,
    pub bookmark: BookmarkPlan,
    pub target_commit_id: String,
    pub title: String,
    pub body: String,
    pub changed_files: Vec<String>,
    pub change_lines: Vec<String>,
    pub base: String,
    pub base_pull_request: Option<PullRequestRecord>,
    pub head: PullRequestHead,
    pub labels: Vec<String>,
    pub draft: bool,
    pub existing_pull_request: Option<PullRequestRecord>,
    pub reviewer_candidates: Vec<ReviewerCandidate>,
    pub(crate) reviewers: ReviewerSelection,
}

/// Operator intent for the final GitHub pull-request readiness state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PullRequestReadiness {
    /// Preserve readiness for existing PRs and create new PRs as ready.
    #[default]
    Preserve,
    /// Ensure the PR is ready for review after publishing.
    Ready,
    /// Ensure the PR is draft after publishing.
    Draft,
}

impl PullRequestReadiness {
    /// Returns the desired final draft bit for a planned create or update.
    pub fn desired_draft(self, existing: Option<&PullRequestRecord>) -> bool {
        match self {
            Self::Preserve => existing.is_some_and(|pull_request| pull_request.draft),
            Self::Ready => false,
            Self::Draft => true,
        }
    }

    /// Stable lowercase label for perf traces and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Preserve => "preserve",
            Self::Ready => "ready",
            Self::Draft => "draft",
        }
    }
}

/// Result of selecting, pushing, and creating or updating a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestReport {
    pub repository: RepositorySummary,
    pub task_id: Option<String>,
    pub bookmark: BookmarkPlan,
    pub bookmark_update: BookmarkUpdate,
    pub push: PushOutcome,
    pub action: PullRequestAction,
    pub pull_request: PullRequestRecord,
    pub base: String,
    pub base_pull_request: Option<PullRequestRecord>,
    pub head: PullRequestHead,
    pub labels: Option<LabelApplyResult>,
    pub reviewers: Option<ReviewerSyncResult>,
    pub event_effects: Vec<PullRequestEventEffect>,
}

/// Command-side effects requested by matching pull-request event handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestEventEffect {
    pub event: RepoEvent,
    pub handler_id: Option<String>,
    pub kind: PullRequestEventEffectKind,
}

/// Effect from a pull-request event handler, reported in handler execution order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestEventEffectKind {
    AddLabels { labels: Vec<String> },
    LabelsAlreadyPresent { labels: Vec<String> },
    OpenPullRequest { url: String },
    TitleAlready { title: String },
    UpdatedTitle { title: String },
}

/// Result of applying pull-request preparation handlers before planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestPrepareReport {
    pub description: String,
    pub changed: bool,
    pub event_effects: Vec<PullRequestEventEffect>,
}

/// Pull-request event handler controls supplied by command-line flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PullRequestPublishOptions {
    pub event_handlers: bool,
}

impl Default for PullRequestPublishOptions {
    fn default() -> Self {
        Self {
            event_handlers: true,
        }
    }
}

/// Planned bookmark push data derived before any jj or Git transport mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushPlan {
    pub repository: RepositorySummary,
    pub bookmark: BookmarkPlan,
    pub target_commit_id: String,
    pub target_short_commit_id: String,
    pub title: String,
}

/// Result of creating or reusing a bookmark and pushing one selected jj change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    pub repository: RepositorySummary,
    pub plan: PushPlan,
    pub bookmark_update: BookmarkUpdate,
    pub push: PushOutcome,
}

/// Result of pushing every tracked origin bookmark, including deletions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedPushReport {
    pub repository: RepositorySummary,
    pub outcome: TrackedPushOutcome,
}

/// Result of fetching origin and then pushing syncable tracked bookmark state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub repository: RepositorySummary,
    pub fetch: FetchOutcome,
    pub trunk: Option<TrunkStateSummary>,
    pub push: TrackedPushOutcome,
    pub skipped_conflicted_bookmarks: Vec<SkippedPushBookmarkSummary>,
    pub pull_requests: Vec<PullRequestRecord>,
}

/// Planned fork/source branch synchronization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkSyncPlan {
    pub repository: RepositorySummary,
    pub source: GitHubRepository,
    pub branch: String,
    pub source_branch: String,
    pub upstream_remote: String,
    pub upstream_url: String,
    pub push: bool,
    pub branch_plan: ForkSyncBranchPlan,
}

/// Result of synchronizing a fork branch with its source branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkSyncReport {
    pub plan: ForkSyncPlan,
    pub upstream: GitRemoteUpdate,
    pub outcome: ForkSyncBranchOutcome,
    pub push: Option<PushOutcome>,
}

/// Viewer-specific review-request state for a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewRequestState {
    New,
    ChangesRequested,
    Commented,
    Answered,
    Again,
    Approved,
}

impl ReviewRequestState {
    /// Stable lowercase label for CLI output and perf diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::ChangesRequested => "changes_requested",
            Self::Commented => "comment",
            Self::Answered => "answered",
            Self::Again => "again",
            Self::Approved => "approved",
        }
    }
}

/// Read-only health summary for the locally tracked pull-request stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestStackStatusReport {
    pub repository: RepositorySummary,
    pub snapshot: PullRequestStackSnapshot,
    pub statuses: BTreeMap<u64, PullRequestStatusRecord>,
    pub trunk: Option<RemoteStatusReport>,
    pub review_wait_threshold_seconds: Option<u64>,
}

/// Pull-request mutation applied by stack PR publishing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestAction {
    Created,
    Updated,
}

impl PullRequestAction {
    /// Stable lowercase label for this pull-request action.
    pub fn label(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
        }
    }
}

/// Bookmark selected by the pure planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkPlan {
    pub branch: String,
    pub action: BookmarkAction,
}

/// Whether the jj layer should create a bookmark or reuse an existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkAction {
    Create,
    Reuse,
}

impl BookmarkAction {
    /// Stable lowercase label for this bookmark action.
    pub fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Reuse => "reuse",
        }
    }
}

/// Input to the pure user-scoped bookmark planner.
pub struct BookmarkPlanRequest<'a> {
    pub github_login: &'a str,
    pub task_id: Option<&'a str>,
    pub workspace: &'a WorkspaceFacts,
}

/// Successful freshness result for `jx remote-status` across configured GitHub remotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub remotes: Vec<RemoteStatusReport>,
    pub fork: Option<ForkStatusReport>,
}

/// Freshness result for one configured GitHub remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteStatusReport {
    pub name: String,
    pub url: String,
    pub github_url: String,
    pub branch: String,
    pub local_trunk_sha: String,
    pub local_trunk_short_sha: String,
    pub local_ahead_by: i64,
    pub comparison: StatusComparison,
}

/// GitHub branch relationship to the local trunk commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusComparison {
    pub state: StatusState,
    pub github_ahead_by: i64,
    pub github_behind_by: i64,
    pub counts_exact: bool,
}

impl StatusComparison {
    /// Stable lowercase label for this freshness state.
    pub fn label(&self) -> &'static str {
        match self.state {
            StatusState::UpToDate => "up-to-date",
            StatusState::GithubAhead => "github-ahead",
            StatusState::LocalAhead => "local-ahead",
            StatusState::Diverged => "diverged",
        }
    }
}

/// Operational freshness states reported by `jx remote-status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusState {
    UpToDate,
    GithubAhead,
    LocalAhead,
    Diverged,
}

/// GitHub fork freshness relative to its source repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkStatusReport {
    pub fork: GitHubRepository,
    pub fork_branch: String,
    pub source: GitHubRepository,
    pub source_branch: String,
    pub comparison: ForkStatusComparison,
}

/// Commit counts for fork/source freshness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkStatusComparison {
    pub state: ForkStatusState,
    pub source_ahead_by: i64,
    pub fork_ahead_by: i64,
}

impl ForkStatusComparison {
    /// Stable lowercase label for this fork freshness state.
    pub fn label(&self) -> &'static str {
        match self.state {
            ForkStatusState::Synced => "synced",
            ForkStatusState::SourceAhead => "source-ahead",
            ForkStatusState::ForkAhead => "fork-ahead",
            ForkStatusState::Diverged => "diverged",
        }
    }
}

/// Operational source/fork relationship reported by `jx remote-status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkStatusState {
    Synced,
    SourceAhead,
    ForkAhead,
    Diverged,
}

pub(super) fn repository_summary(context: &RepositoryContext) -> RepositorySummary {
    RepositorySummary {
        origin_name: context.origin.name,
        origin_url: context.origin.url.clone(),
        github_slug: context.origin.github.slug(),
        github_url: context.origin.github.https_url(),
        token_source: context.token_source.summary(),
        config: context.config.summary(),
        default_reviewers: context.config.reviewer_summary(&context.origin.github),
    }
}
