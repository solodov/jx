use super::*;

/// jj workspace failures surfaced as actionable diagnostics.
#[derive(Debug, Error)]
pub enum JjError {
    #[error("Could not initialize jj settings: {message}")]
    Settings { message: String },
    #[error("Could not load jj workspace: {message}")]
    WorkspaceLoad { message: String },
    #[error("Could not load jj repository state: {message}")]
    RepoLoad { message: String },
    #[error("Could not resolve commit `{revision}`: {message}")]
    Revision { revision: String, message: String },
    #[error("Revision `{revision}` did not resolve to a commit")]
    RevisionNotFound { revision: String },
    #[error("Revision `{revision}` resolved to multiple commits; pass a single commit")]
    AmbiguousRevision { revision: String },
    #[error("Revision `{target}` did not resolve to a commit or local bookmark")]
    RevisionTargetNotFound { target: String },
    #[error("Revision `{target}` matches multiple local bookmarks: {matches:?}")]
    RevisionTargetAmbiguous {
        target: String,
        matches: Vec<String>,
    },
    #[error("Stack target `{target}` did not resolve to a commit or local bookmark")]
    StackTargetNotFound { target: String },
    #[error("Stack target `{target}` matches multiple local bookmarks: {matches:?}")]
    StackTargetAmbiguous {
        target: String,
        matches: Vec<String>,
    },
    #[error("Cannot move current stack onto one of its descendants")]
    StackTargetDescendant,
    #[error("Cannot move selected revision `{commit_id}` onto itself")]
    StackSourceIsTarget { commit_id: String },
    #[error("Internal PR bookmark target `{commit_id}` is not a valid commit id")]
    InvalidTargetCommitId { commit_id: String },
    #[error("Workspace `{workspace}` does not have a current working-copy change")]
    MissingWorkingCopy { workspace: String },
    #[error(
        "The jj repository uses the `{backend}` backend; `jx` can only publish repositories whose jj state is backed by Git"
    )]
    NotGitBacked { backend: String },
    #[error("Could not read jj backend data: {message}")]
    Backend { message: String },
    #[error("Could not query jj index data: {message}")]
    Index { message: String },
    #[error("Could not complete jj git fetch from `origin`: {message}")]
    Fetch { message: String },
    #[error("Could not import fetched origin refs into jj: {message}")]
    Import { message: String },
    #[error("Could not export jj bookmarks to the backing Git repo: {message}")]
    Export { message: String },
    #[error("Could not complete jj git push for `{branch}`: {message}")]
    Push { branch: String, message: String },
    #[error("Remote rejected push for `{branch}`: {message}")]
    PushRejected { branch: String, message: String },
    #[error("Could not run `{command}`: {source}")]
    DiffStart { command: String, source: io::Error },
    #[error("`jj diff --name-only` returned paths that are not UTF-8: {source}")]
    DiffPathDecode { source: std::string::FromUtf8Error },
    #[error("`{command}` failed with {status}")]
    DiffFailed { command: String, status: String },
    #[error("Could not run `jj status`: {source}")]
    StatusStart { source: io::Error },
    #[error("`jj status` returned output that is not UTF-8: {source}")]
    StatusDecode { source: std::string::FromUtf8Error },
    #[error("`jj status` failed with {status}")]
    StatusFailed { status: String },
    #[error("Could not run `jj git clone`: {source}")]
    CloneStart { source: io::Error },
    #[error("`jj git clone` failed with {status}")]
    CloneFailed { status: String },
    #[error("Could not run `jj git init`: {source}")]
    InitStart { source: io::Error },
    #[error("`jj git init` failed with {status}")]
    InitFailed { status: String },
    #[error("Could not run `jj workspace add`: {source}")]
    WorkspaceAddStart { source: io::Error },
    #[error("`jj workspace add` failed with {status}")]
    WorkspaceAddFailed { status: String },
    #[error("Workspace path already exists: {path}")]
    WorkspacePathExists { path: PathBuf },
    #[error("Refusing to share tracked workspace paths in the selected checkout: {paths:?}")]
    WorkspaceSharedPathsTracked { paths: Vec<String> },
    #[error("Invalid shared workspace path `{path}`: {message}")]
    WorkspaceSharedPathInvalid { path: String, message: String },
    #[error("Could not run `jj workspace list`: {source}")]
    WorkspaceListStart { source: io::Error },
    #[error("`jj workspace list` returned output that is not UTF-8: {source}")]
    WorkspaceListDecode { source: std::string::FromUtf8Error },
    #[error("`jj workspace list` failed with {status}")]
    WorkspaceListFailed { status: String },
    #[error("Could not run `jj workspace root`: {source}")]
    WorkspaceRootStart { source: io::Error },
    #[error("`jj workspace root --name {name}` returned output that is not UTF-8: {source}")]
    WorkspaceRootDecode {
        name: String,
        source: std::string::FromUtf8Error,
    },
    #[error("`jj workspace root --name {name}` failed with {status}")]
    WorkspaceRootFailed { name: String, status: String },
    #[error("Could not run `jj workspace forget {name}`: {source}")]
    WorkspaceForgetStart { name: String, source: io::Error },
    #[error("`jj workspace forget {name}` failed with {status}")]
    WorkspaceForgetFailed { name: String, status: String },
    #[error("Could not {action} `{path}`: {source}")]
    WorkspaceIo {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    #[error("Could not delete workspace `{name}`: {message}")]
    WorkspaceRemove { name: String, message: String },
    #[error("No publishable commit found for repository creation: {message}")]
    NoPublishableCommit { message: String },
    #[error("Could not add Git remote `{remote}`: {message}")]
    RemoteAdd { remote: String, message: String },
    #[error("Could not run `jj git push -b {branch}`: {source}")]
    BootstrapPushStart { branch: String, source: io::Error },
    #[error("`jj git push -b {branch}` failed with {status}")]
    BootstrapPushFailed { branch: String, status: String },
    #[error("Could not write jj operation: {message}")]
    Transaction { message: String },
    #[error("Could not update the working copy after jj mutation: {message}")]
    WorkingCopyCheckout { message: String },
    #[error("Current change has no previous commit in its editable chain")]
    NoPreviousCommit,
    #[error("Current change has no next commit in its editable chain")]
    NoNextCommit,
    #[error("Current change has {count} parent commits; choose one explicitly before moving")]
    AmbiguousPreviousCommit { count: usize },
    #[error("Current change has {count} next commits; choose one explicitly before moving")]
    AmbiguousNextCommit { count: usize },
    #[error("Could not move working copy to {direction} commit: {message}")]
    CommitNavigation {
        direction: &'static str,
        message: String,
    },
    #[error("Local bookmark `{branch}` is conflicted; resolve it before pushing")]
    ConflictedBookmark { branch: String },
    #[error(
        "Remote bookmark `{branch}@{remote}` is conflicted; fetch or resolve it before pushing"
    )]
    ConflictedRemoteBookmark {
        branch: String,
        remote: &'static str,
    },
    #[error("Remote bookmark `{branch}@{remote}` is not tracked locally; fetch or track it before pushing")]
    NonTrackingRemoteBookmark {
        branch: String,
        remote: &'static str,
    },
    #[error("Refusing to push deleted bookmark `{branch}` without tracked-delete mode")]
    DeletedBookmarkNotRequested { branch: String },
    #[error("Refusing to create new remote bookmark `{branch}@{remote}` in tracked-only mode")]
    NewRemoteBookmarkNotAllowed {
        branch: String,
        remote: &'static str,
    },
    #[error("Local bookmark `{branch}` is missing; create it before pushing")]
    MissingLocalBookmark { branch: String },
    #[error("Selected change has no local bookmark; create or choose a bookmark before syncing one target")]
    MissingSyncBookmark,
    #[error(
        "Selected change has multiple local bookmarks: {bookmarks:?}; pass the bookmark name to choose one"
    )]
    AmbiguousSyncBookmark { bookmarks: Vec<String> },
    #[error("Refusing to advance local trunk bookmark `{branch}` because it points outside the current trunk stack")]
    TrunkBookmarkOutsideStack { branch: String },
    #[error("Local bookmark `{branch}` already points at another change")]
    BookmarkExistsOnDifferentChange { branch: String },
    #[error(
        "Could not resolve trunk from `{remote}` remote bookmarks. Fetch that remote or ensure a remote bookmark such as `main` exists."
    )]
    MissingTrunk { remote: String },
    #[error(
        "Could not resolve trunk from `{remote}` because these remote bookmarks are conflicted: {branches:?}"
    )]
    ConflictedTrunk {
        remote: String,
        branches: Vec<String>,
    },
    #[error(
        "Could not choose trunk from `{remote}` because multiple remote bookmarks are ancestors of the selected change: {branches:?}"
    )]
    AmbiguousTrunk {
        remote: String,
        branches: Vec<String>,
    },
    #[error("Could not render workspace jj log: {message}")]
    Log { message: String },
    #[error("Could not render jj-styled output: {message}")]
    Render { message: String },
    #[error("Could not compute stack index: {message}")]
    NonLinearStack { message: String },
    #[error("Selected revisions do not include any stack commits to publish")]
    EmptyStackPublishSelection,
    #[error(
        "Selected revisions span multiple stacks; narrow the revset or publish one stack at a time"
    )]
    StackPublishMultipleStacks,
    #[error("Selected revisions do not form a single linear stack; narrow the revset or publish one stack at a time")]
    StackPublishNonLinearSelection,
}

pub(super) fn workspace_config_command_error(
    error: jj_cli::command_error::CommandError,
) -> JjError {
    settings_error(error.error)
}

pub(super) fn settings_error(error: impl ToString) -> JjError {
    JjError::Settings {
        message: error.to_string(),
    }
}

pub(super) fn workspace_load_error(error: impl ToString) -> JjError {
    JjError::WorkspaceLoad {
        message: error.to_string(),
    }
}

pub(super) fn log_command_error(error: jj_cli::command_error::CommandError) -> JjError {
    JjError::Log {
        message: error.error.to_string(),
    }
}

pub(super) fn log_error(error: impl ToString) -> JjError {
    JjError::Log {
        message: error.to_string(),
    }
}

pub(super) fn render_command_error(error: jj_cli::command_error::CommandError) -> JjError {
    JjError::Render {
        message: error.error.to_string(),
    }
}

pub(super) fn render_error(error: impl ToString) -> JjError {
    JjError::Render {
        message: error.to_string(),
    }
}

pub(super) fn revision_command_error(
    revision: &str,
    error: jj_cli::command_error::CommandError,
) -> JjError {
    revision_error(revision, error.error)
}

pub(super) fn revision_error(revision: &str, error: impl ToString) -> JjError {
    JjError::Revision {
        revision: revision.to_owned(),
        message: error.to_string(),
    }
}
