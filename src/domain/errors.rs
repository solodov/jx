use super::*;

/// Workflow-service failures after repository context has loaded.
#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error(transparent)]
    GitHub(#[from] GitHubError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error("GitHub token cannot read `{repository}`; use a token with repository read access")]
    MissingReadAccess { repository: String },
    #[error(
        "GitHub token cannot push to `{repository}`; same-repository push and PR workflows require push access"
    )]
    MissingPushAccess { repository: String },
    #[error("GitHub authenticated user login is empty; cannot plan a user-scoped bookmark")]
    MissingGitHubLogin,
    #[error("GitHub login `{login}` cannot be used as a bookmark namespace")]
    InvalidGitHubLogin { login: String },
    #[error("Task id `{task_id}` cannot be used in a bookmark name")]
    InvalidTaskId { task_id: String },
    #[error(
        "Local bookmark `{branch}` already exists on another change; refusing to create a duplicate PR head"
    )]
    BookmarkExistsOnDifferentChange { branch: String },
    #[error(
        "Selected change has multiple user-scoped bookmarks {bookmarks:?}; choose one by removing the others before planning a new PR head"
    )]
    AmbiguousSelectedBookmarks { bookmarks: Vec<String> },
    #[error(
        "Generated push bookmark `{branch}` already exists on another change; choose or move a bookmark before pushing"
    )]
    PushBookmarkExistsOnDifferentChange { branch: String },
    #[error("Fetch rebased commits with conflicts: {commits}; resolve them before pushing")]
    FetchConflicts { commits: String },
    #[error("Selected change is empty; write changes before creating or updating a pull request")]
    EmptyPullRequestChange,
    #[error(
        "Selected change description is empty; describe it before creating or updating a pull request"
    )]
    MissingPullRequestDescription,
    #[error("Selected change and its descendant bookmarks do not have a pull request")]
    MissingPullRequest,
    #[error("No local bookmarks in `{repository}` have open pull requests authored by you")]
    MissingLocalBookmarkPullRequests { repository: String },
    #[error(
        "GitHub could not compare `{branch}` with local trunk `{local_sha}`; status is unavailable"
    )]
    UnavailableComparison { branch: String, local_sha: String },
    #[error(
        "GitHub could not compare fork `{fork}` branch `{fork_branch}` with source `{source_repo}` branch `{source_branch}`; fork status is unavailable"
    )]
    UnavailableForkComparison {
        source_repo: String,
        source_branch: String,
        fork: String,
        fork_branch: String,
    },
    #[error("Local status facts did not include configured remote `{remote}`")]
    MissingStatusRemote { remote: String },
}
