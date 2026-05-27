/// GitHub repository identity parsed from the fixed origin URL.
#[derive(Debug, Clone, PartialEq, Eq)]
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
