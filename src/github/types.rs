use super::ReviewerSelection;

/// GitHub repository identity parsed from the fixed origin URL.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
pub struct GitHubRepository {
    pub owner: String,
    pub name: String,
}

impl GitHubRepository {
    /// Parses common GitHub HTTPS and SSH remote URL forms.
    pub fn parse(url: &str) -> Result<Self, GitHubUrlError> {
        let path = github_path(url.trim()).ok_or(GitHubUrlError)?;
        let path = path.trim_start_matches('/').trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        let mut components = path.split('/');
        let owner = components.next().ok_or(GitHubUrlError)?;
        let name = components.next().ok_or(GitHubUrlError)?;

        if components.next().is_some()
            || !is_valid_github_component(owner)
            || !is_valid_github_component(name)
        {
            return Err(GitHubUrlError);
        }

        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    /// Returns the `owner/repo` form used in concise command output.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// Returns the canonical HTTPS repository URL suitable for clickable output.
    pub fn https_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }
}

/// Marker error for unsuitable GitHub remote URL syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitHubUrlError;

fn github_path(url: &str) -> Option<&str> {
    url.strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("ssh://github.com/"))
}

fn is_valid_github_component(component: &str) -> bool {
    !component.is_empty()
        && !component.starts_with('.')
        && !component.ends_with('.')
        && component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Authenticated GitHub user identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub login: String,
}

/// Public GitHub profile fields used only for human display enrichment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubUserProfile {
    pub login: String,
    pub name: Option<String>,
}

/// High-level repository access facts used by readiness checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryAccess {
    pub repository: GitHubRepository,
    pub default_branch: Option<String>,
    pub can_read: bool,
    pub can_push: bool,
    pub can_admin: bool,
}

/// Source repository metadata for a GitHub fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryFork {
    pub source: GitHubRepository,
    pub source_default_branch: Option<String>,
}

/// Repository created by the GitHub boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryCreation {
    pub repository: GitHubRepository,
    pub html_url: String,
    pub private: bool,
}

/// GitHub comparison result expressed in `jx` domain terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitComparison {
    pub status: ComparisonStatus,
    pub ahead_by: i64,
    pub behind_by: i64,
}

/// Relationship between the base and head refs returned by GitHub comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonStatus {
    Ahead,
    Behind,
    Diverged,
    Identical,
    Unknown,
}

/// Same-repository pull-request head branch.
///
/// The owner is the repository owner used in GitHub's `owner:branch` PR head
/// label. The branch may itself be namespaced by the authenticated user's login,
/// such as `example-user/abc-123-00-a1b2c3d`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestHead {
    pub owner: String,
    pub branch: String,
}

impl PullRequestHead {
    /// Creates a same-repository head label for a branch in `repository_owner`.
    pub fn same_repository(repository_owner: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            owner: repository_owner.into(),
            branch: branch.into(),
        }
    }

    /// Returns the GitHub PR head label, e.g. `example-owner:example-user/abc-123-00-a1b2c3d`.
    pub fn label(&self) -> String {
        format!("{}:{}", self.owner, self.branch)
    }
}

/// Review inbox search results for open pull requests relevant to the authenticated viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestReviewRequests {
    pub viewer: AuthenticatedUser,
    pub requests: Vec<PullRequestReviewRequest>,
}

/// Pull request requesting review from or already reviewed by the authenticated viewer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
pub struct PullRequestReviewRequest {
    pub repository: GitHubRepository,
    pub number: u64,
}

/// Cheap freshness facts for deciding whether a full PR refresh is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestUpdateSummary {
    pub number: u64,
    /// GitHub updated timestamp in RFC3339 form.
    pub updated_at: String,
    pub latest_commit_oid: Option<String>,
    pub checks: Vec<PullRequestCheck>,
}

/// Pull-request data returned by the GitHub boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestRecord {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub head_branch: String,
    pub base_branch: String,
    pub html_url: Option<String>,
    pub draft: bool,
    pub merged: bool,
    pub reviewers: ReviewerSelection,
}

/// GitHub label attached to a pull request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PullRequestLabel {
    pub name: String,
    /// Six-digit RGB hex color as returned by GitHub, without a leading `#`.
    pub color: String,
}

/// GitHub mergeability status for a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum PullRequestMergeStatus {
    Mergeable,
    Conflicting,
    Unknown,
}

impl PullRequestMergeStatus {
    /// Stable lowercase label for CLI and JSON output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mergeable => "mergeable",
            Self::Conflicting => "conflicting",
            Self::Unknown => "unknown",
        }
    }
}

/// Repository-policy interpretation of label-driven auto-merge state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestAutoMergeStatus {
    /// No configured auto-merge signal is currently useful for display.
    #[default]
    NotConfigured,
    /// A configured label indicates the PR should merge automatically once ready.
    Armed,
    /// A configured label is present, but manual prerequisites must be cleared first.
    PrerequisitesRequired,
    /// The PR appears ready to merge, but no configured auto-merge label is present.
    Missing,
}

impl PullRequestAutoMergeStatus {
    /// Stable lowercase label for CLI and JSON output.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::Armed => "armed",
            Self::PrerequisitesRequired => "prerequisites_required",
            Self::Missing => "missing",
        }
    }

    /// Returns whether no configured auto-merge state should be reported.
    pub fn is_not_configured(&self) -> bool {
        matches!(self, Self::NotConfigured)
    }
}

/// Read-only status facts for a pull request in a stack triage view.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PullRequestStatusRecord {
    pub number: u64,
    pub title: String,
    pub url: Option<String>,
    /// GitHub creation timestamp in RFC3339 form, when available.
    pub created_at: Option<String>,
    pub head_branch: String,
    pub base_branch: String,
    /// Base repository default branch name, when available in the snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    pub author: Option<String>,
    pub draft: bool,
    pub merged: bool,
    pub closed: bool,
    /// GitHub merge timestamp in RFC3339 form, when the pull request has merged.
    pub merged_at: Option<String>,
    /// GitHub close timestamp in RFC3339 form, when the pull request is closed.
    pub closed_at: Option<String>,
    pub check_status: PullRequestCheckStatus,
    pub checks: Vec<PullRequestCheck>,
    pub merge_status: PullRequestMergeStatus,
    pub review_status: PullRequestReviewStatus,
    /// Repo-configured label-driven auto-merge presentation state.
    #[serde(
        default,
        skip_serializing_if = "PullRequestAutoMergeStatus::is_not_configured"
    )]
    pub auto_merge_status: PullRequestAutoMergeStatus,
    pub requested_reviewers: ReviewerSelection,
    pub suggested_reviewers: Vec<String>,
    pub approved_reviewers: Vec<String>,
    pub changes_requested_reviewers: Vec<String>,
    pub commented_reviewers: Vec<String>,
    pub addressed_reviewers: Vec<String>,
    /// PR-author responses to reviewer activity that may resurface dismissed reviews.
    pub reviewer_responses: Vec<PullRequestReviewerResponse>,
    /// Latest comments that explicitly mention a reviewer.
    pub reviewer_mentions: Vec<PullRequestReviewerMention>,
    /// Reviewers whose latest submitted review was dismissed by GitHub.
    pub dismissed_reviewers: Vec<String>,
    pub review_activity: Vec<PullRequestReviewActivity>,
    pub timeline_events: Vec<PullRequestTimelineEvent>,
    pub labels: Vec<PullRequestLabel>,
    pub latest_commit_oid: Option<String>,
}

/// Latest known review activity for one reviewer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PullRequestReviewActivity {
    pub reviewer: String,
    /// GitHub review or review-comment timestamp in RFC3339 form.
    pub reviewed_at: String,
}

/// PR-author response to a reviewer's prior activity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PullRequestReviewerResponse {
    pub reviewer: String,
    /// GitHub author-response comment timestamp in RFC3339 form.
    pub responded_at: String,
    /// Plain-text author response body used by review dismissal policy.
    pub body_text: String,
}

/// Latest explicit mention of a reviewer in PR discussion.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PullRequestReviewerMention {
    pub reviewer: String,
    /// GitHub comment timestamp in RFC3339 form.
    pub mentioned_at: String,
}

/// Pull-request lifecycle event needed to explain review wait time.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PullRequestTimelineEvent {
    pub kind: PullRequestTimelineEventKind,
    /// GitHub event timestamp in RFC3339 form.
    pub created_at: String,
    /// User login or `team/<slug>` for review-request events.
    pub reviewer: Option<String>,
}

/// Supported pull-request lifecycle event kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum PullRequestTimelineEventKind {
    ReadyForReview,
    ConvertToDraft,
    ReviewRequested,
}

/// Summary of the latest commit's GitHub check rollup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum PullRequestCheckStatus {
    Passing,
    Failing,
    Pending,
    Missing,
    Unknown,
}

impl PullRequestCheckStatus {
    /// Stable lowercase label for CLI and JSON output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Passing => "passing",
            Self::Failing => "failing",
            Self::Pending => "pending",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}

/// One latest-commit check run or commit status context in GitHub's rollup.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PullRequestCheck {
    pub name: String,
    pub status: PullRequestCheckStatus,
}

/// Summary of GitHub's review decision and outstanding review requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum PullRequestReviewStatus {
    Approved,
    ChangesRequested,
    ReviewRequired,
    ReviewRequested,
    NotReviewed,
    Unknown,
}

impl PullRequestReviewStatus {
    /// Stable lowercase label for CLI and JSON output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::ChangesRequested => "changes_requested",
            Self::ReviewRequired => "review_required",
            Self::ReviewRequested => "review_requested",
            Self::NotReviewed => "not_reviewed",
            Self::Unknown => "unknown",
        }
    }
}

/// Domain input for creating a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestCreate {
    pub title: String,
    pub body: Option<String>,
    pub head: PullRequestHead,
    pub base: String,
    pub draft: bool,
}

/// Domain input for updating a pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestUpdate {
    pub title: Option<String>,
    pub body: Option<String>,
    pub base: Option<String>,
}
