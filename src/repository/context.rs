use super::*;

/// Local jj repository context before fixed-origin assumptions are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRepositoryContext {
    pub workspace_root: PathBuf,
    pub remotes: Vec<ConfiguredRemote>,
    pub token_source: TokenSource,
    pub config: WorkflowConfig,
}

impl LocalRepositoryContext {
    /// Discovers local workspace config and remotes without requiring `origin`.
    pub fn discover(environment: &RuntimeEnvironment) -> Result<Self, RepositoryError> {
        let workspace_root = find_workspace_root(environment.current_dir())?;
        let remotes = read_remote_urls(&workspace_root)?;
        let config = WorkflowConfig::discover_for_workspace(&workspace_root, environment)?;
        let token_source = TokenSource::discover(environment, &config);

        Ok(Self {
            workspace_root,
            remotes,
            token_source,
            config,
        })
    }

    /// Returns whether the repository has any configured Git remote.
    pub fn has_remotes(&self) -> bool {
        !self.remotes.is_empty()
    }

    /// Converts local context into the fixed-origin context used by established workflows.
    pub fn into_origin_context(self) -> Result<RepositoryContext, RepositoryError> {
        let origin_url = self
            .remotes
            .iter()
            .find(|remote| remote.name == ORIGIN_REMOTE_NAME)
            .map(|remote| remote.url.clone())
            .ok_or(RepositoryError::MissingOrigin)?;
        let origin_github =
            GitHubRepository::parse(&origin_url).map_err(|_| RepositoryError::OriginNotGitHub {
                url: origin_url.clone(),
            })?;
        let github_remotes = github_remotes(self.remotes);

        Ok(RepositoryContext {
            workspace_root: self.workspace_root,
            origin: OriginRemote {
                name: ORIGIN_REMOTE_NAME,
                url: origin_url,
                github: origin_github,
            },
            github_remotes,
            token_source: self.token_source,
            config: self.config,
        })
    }
}

/// Repository context shared by established fixed-origin command handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryContext {
    pub workspace_root: PathBuf,
    pub origin: OriginRemote,
    pub github_remotes: Vec<GitHubRemote>,
    pub token_source: TokenSource,
    pub config: WorkflowConfig,
}

impl RepositoryContext {
    /// Discovers the repository context from the provided runtime environment.
    pub fn discover(environment: &RuntimeEnvironment) -> Result<Self, RepositoryError> {
        LocalRepositoryContext::discover(environment)?.into_origin_context()
    }
}

/// Configured GitHub remote used for read-only status reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRemote {
    pub name: String,
    pub url: String,
    pub github: GitHubRepository,
}

/// Fixed `origin` remote details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginRemote {
    pub name: &'static str,
    pub url: String,
    pub github: GitHubRepository,
}

pub(super) fn find_workspace_root(start: &Path) -> Result<PathBuf, RepositoryError> {
    for candidate in start.ancestors() {
        if candidate.join(".jj").is_dir() {
            return Ok(candidate.to_path_buf());
        }
    }

    Err(RepositoryError::WorkspaceNotFound)
}

fn read_remote_urls(workspace_root: &Path) -> Result<Vec<ConfiguredRemote>, RepositoryError> {
    let workspace = JjWorkspace::load(workspace_root.to_path_buf()).map_err(jj_metadata_error)?;
    let remotes = workspace.git_remotes().map_err(jj_metadata_error)?;

    Ok(remotes
        .into_iter()
        .map(|remote| ConfiguredRemote {
            name: remote.name,
            url: remote.url,
        })
        .collect())
}

fn jj_metadata_error(error: crate::jj::JjError) -> RepositoryError {
    RepositoryError::JjMetadata {
        message: error.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredRemote {
    pub name: String,
    pub url: String,
}

fn github_remotes(remotes: Vec<ConfiguredRemote>) -> Vec<GitHubRemote> {
    remotes
        .into_iter()
        .filter_map(|remote| {
            GitHubRepository::parse(&remote.url)
                .ok()
                .map(|github| GitHubRemote {
                    name: remote.name,
                    url: remote.url,
                    github,
                })
        })
        .collect()
}
