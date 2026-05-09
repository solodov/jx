use super::*;

/// Workflow command names supported by the CLI surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowCommand {
    Check,
    PullRequest,
}

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

/// Planned same-repository bookmark intent for `jx pull-request`.
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

/// Result of rebasing a selected jj source onto the fixed `origin` trunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseOnTrunkReport {
    pub repository: RepositorySummary,
    pub outcome: RebaseOnTrunkOutcome,
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
    pub base: String,
    pub head: PullRequestHead,
    pub labels: Vec<String>,
    pub draft: bool,
    pub existing_pull_request: Option<PullRequestRecord>,
    pub reviewer_candidates: Vec<ReviewerCandidate>,
    pub(crate) reviewers: ReviewerSelection,
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
    pub head: PullRequestHead,
    pub labels: Option<LabelApplyResult>,
    pub reviewers: Option<ReviewerSyncResult>,
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

/// Result of fetching origin and then pushing tracked bookmark state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub repository: RepositorySummary,
    pub fetch: FetchOutcome,
    pub push: TrackedPushOutcome,
    pub pull_requests: Vec<PullRequestRecord>,
}

/// Pull-request mutation applied by `jx pull-request`.
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
