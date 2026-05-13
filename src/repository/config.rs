use super::*;

mod diff;
mod layout;
mod parse;
mod repo_policy;
mod shell;

pub use diff::*;
pub use layout::*;
use parse::{config_file_label, parse_workflow_config_layer, WorkflowConfigLayer};
pub use repo_policy::*;
pub use shell::*;

const GLOBAL_CONFIG_RELATIVE_PATH: [&str; 2] = [".config", "jx"];
const PROJECT_CONFIG_FILE: &str = ".jx.toml";

/// Optional workflow config composed from global files and the project file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowConfig {
    pub paths: Vec<PathBuf>,
    pub layout: LayoutConfig,
    pub repo: RepoConfig,
    pub diff: DiffConfig,
    pub auth: AuthConfig,
    pub shell: ShellConfig,
}

impl WorkflowConfig {
    /// Discovers config for commands that do not need fixed-origin GitHub context.
    pub fn discover(environment: &RuntimeEnvironment) -> Result<Self, RepositoryError> {
        let workspace_root = find_workspace_root(environment.current_dir())?;
        Self::discover_for_workspace(&workspace_root, environment)
    }

    /// Discovers only global config for commands that operate across configured layout roots.
    pub fn discover_global(environment: &RuntimeEnvironment) -> Result<Self, RepositoryError> {
        let mut config = Self::default();

        if let Some(path) = global_config_dir(environment) {
            config.apply_optional_global_configs(path)?;
        }
        config.validate()?;

        Ok(config)
    }

    /// Discovers global layout config, plus project config when run inside a jj workspace.
    pub fn discover_for_clone(environment: &RuntimeEnvironment) -> Result<Self, RepositoryError> {
        let mut config = Self::default();

        if let Some(path) = global_config_dir(environment) {
            config.apply_optional_global_configs(path)?;
        }
        if let Ok(workspace_root) = find_workspace_root(environment.current_dir()) {
            config.apply_optional_project_config(workspace_root.join(PROJECT_CONFIG_FILE))?;
        }
        config.validate()?;

        Ok(config)
    }

    /// Discovers config for a layout-resolved directory before a jj workspace exists.
    pub fn discover_for_uninitialized(
        environment: &RuntimeEnvironment,
    ) -> Result<Self, RepositoryError> {
        let mut config = Self::default();

        if let Some(path) = global_config_dir(environment) {
            config.apply_optional_global_configs(path)?;
        }
        config
            .apply_optional_project_config(environment.current_dir().join(PROJECT_CONFIG_FILE))?;
        config.validate()?;

        Ok(config)
    }

    pub(super) fn discover_for_workspace(
        workspace_root: &Path,
        environment: &RuntimeEnvironment,
    ) -> Result<Self, RepositoryError> {
        let mut config = Self::default();

        if let Some(path) = global_config_dir(environment) {
            config.apply_optional_global_configs(path)?;
        }
        config.apply_optional_project_config(workspace_root.join(PROJECT_CONFIG_FILE))?;
        config.validate()?;

        Ok(config)
    }

    fn apply_optional_global_configs(&mut self, dir: PathBuf) -> Result<(), RepositoryError> {
        for path in optional_global_config_files(dir)? {
            self.apply_config_file(path)?;
        }
        Ok(())
    }

    fn apply_optional_project_config(&mut self, path: PathBuf) -> Result<(), RepositoryError> {
        if path.is_file() {
            self.apply_config_file(path)?;
        }
        Ok(())
    }

    fn apply_config_file(&mut self, path: PathBuf) -> Result<(), RepositoryError> {
        let file = config_file_label(&path);
        let contents = fs::read_to_string(&path)
            .map_err(|source| RepositoryError::ConfigRead { file, source })?;
        let layer = parse_workflow_config_layer(path, &contents)?;

        self.apply_layer(layer);
        Ok(())
    }

    fn apply_layer(&mut self, layer: WorkflowConfigLayer) {
        // List-valued sections compose across files; scalar sections use the
        // last configured value so later global/project files can refine defaults.
        self.paths.push(layer.path);
        if let Some(layout) = layer.layout {
            self.layout.apply_layer(layout);
        }
        if let Some(repo) = layer.repo {
            self.repo.apply_layer(repo);
        }
        if let Some(diff) = layer.diff {
            self.diff.apply_layer(diff);
        }
        if let Some(auth) = layer.auth {
            self.auth.apply_layer(auth);
        }
        if let Some(shell) = layer.shell {
            self.shell.apply_layer(shell);
        }
    }

    fn validate(&self) -> Result<(), RepositoryError> {
        self.layout.validate()?;
        self.shell.validate()?;

        if let Some(default_tool) = &self.diff.default_tool {
            if !self.diff.tools.contains_key(default_tool) {
                return Err(RepositoryError::InvalidConfig {
                    file: "jx config".to_owned(),
                    message: format!(
                        "`diff.default_tool` references `{default_tool}`, but no `[diff.tools.{default_tool}]` tool is configured"
                    ),
                });
            }
        }

        Ok(())
    }

    /// Human-readable config status that avoids printing local paths.
    pub fn summary(&self) -> &'static str {
        if self.paths.is_empty() {
            "defaults"
        } else {
            "present"
        }
    }

    /// Human-readable reviewer candidate status.
    pub fn reviewer_summary(&self, repository: &GitHubRepository) -> String {
        self.repo.reviewer_summary_for(repository)
    }
}

/// Authentication preferences loaded from optional config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthConfig {
    pub keychain: Option<KeychainConfig>,
}

impl AuthConfig {
    fn apply_layer(&mut self, layer: AuthConfig) {
        if let Some(keychain) = layer.keychain {
            self.keychain = Some(keychain);
        }
    }
}

/// Repository context discovery failures with actionable diagnostics.
#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("No jj workspace found. Run `jx` from inside a jj workspace.")]
    WorkspaceNotFound,
    #[error(
        "The fixed `origin` remote is missing. Add an `origin` GitHub remote before running `jx`."
    )]
    MissingOrigin,
    #[error(
        "The fixed `origin` remote URL `{url}` is not a GitHub repository URL. Set `origin` to a URL such as `https://github.com/example-owner/example-repo.git`."
    )]
    OriginNotGitHub { url: String },
    #[error("Could not read repository metadata for the jj workspace: {source}")]
    MetadataRead { source: io::Error },
    #[error("Could not read jj workspace repository metadata: {message}")]
    JjMetadata { message: String },
    #[error("Could not read workflow config `{file}`: {source}")]
    ConfigRead { file: String, source: io::Error },
    #[error("Could not parse workflow config `{file}`: {source}")]
    ConfigParse {
        file: String,
        source: toml::de::Error,
    },
    #[error(
        "Unsupported workflow config key `{key}` in `{file}`. Config supports `[layout]`, `[repo]`, `[[repo.rules]]`, `[diff]`, `[auth.keychain] service/account`, and `[shell]`; remotes and hooks are not configurable."
    )]
    UnsupportedConfigKey { file: String, key: String },
    #[error("Invalid workflow config `{file}`: {message}")]
    InvalidConfig { file: String, message: String },
    #[error("Invalid repository `{repository}`. {message}.")]
    InvalidCloneRepository { repository: String, message: String },
    #[error("Invalid workspace name `{name}`. {message}.")]
    InvalidWorkspaceName { name: String, message: String },
    #[error("Workspace path already exists: {path}")]
    WorkspacePathExists { path: PathBuf },
    #[error("Workspace `{name}` is not registered in this jj repository")]
    WorkspaceNameNotFound { name: String },
    #[error("Current workspace is not registered in this jj repository")]
    CurrentWorkspaceNotFound,
    #[error("Refusing to delete current workspace `{name}`")]
    RefuseRemoveCurrentWorkspace { name: String },
    #[error("Refusing to delete primary workspace `{name}` at {path}")]
    RefuseRemovePrimaryWorkspace { name: String, path: PathBuf },
    #[error("Refusing to delete workspace `{name}` at {path} because it is outside the configured `{workspace_dir}` layout")]
    RefuseRemoveUnmanagedWorkspace {
        name: String,
        path: PathBuf,
        workspace_dir: String,
    },
    #[error("Work location `{key}` was not found in configured layouts")]
    WorkLocationNotFound { key: String },
    #[error("Work location `{key}` matches multiple paths: {paths:?}")]
    WorkLocationAmbiguous { key: String, paths: Vec<PathBuf> },
    #[error("Invalid repository filter `{pattern}`: {message}")]
    InvalidRepositoryFilter { pattern: String, message: String },
    #[error("No configured repository matched `{pattern}`")]
    RepositoryFilterNotFound { pattern: String },
    #[error("Layout source `{name}` is not configured. Add `[[layout.sources]]` for that source or use an explicit host/URL.")]
    UnknownLayoutSource { name: String },
    #[error("Multiple layout sources use host `{host}`: {sources:?}. Use `source:owner/repo` to disambiguate.")]
    AmbiguousLayoutHost { host: String, sources: Vec<String> },
    #[error("Could not expand layout path `{path}` because HOME is not set")]
    MissingHomeForLayout { path: String },
    #[error("Workspace path `{path}` does not match configured layout roots. Add an origin remote manually or move the repository under the configured layout.")]
    LayoutPathNotMatched { path: PathBuf },
    #[error("Workspace path `{path}` matches multiple layout identities: {identities:?}. Add an origin remote manually or make the layout rules more specific.")]
    AmbiguousLayoutPath {
        path: PathBuf,
        identities: Vec<String>,
    },
    #[error("Could not read workspace metadata `{file}`: {source}")]
    WorkspaceMetadataRead { file: PathBuf, source: io::Error },
    #[error("Could not parse workspace metadata `{file}`: {source}")]
    WorkspaceMetadataParse {
        file: PathBuf,
        source: toml::de::Error,
    },
    #[error("Could not write workspace metadata `{file}`: {source}")]
    WorkspaceMetadataWrite { file: PathBuf, source: io::Error },
}

fn global_config_dir(environment: &RuntimeEnvironment) -> Option<PathBuf> {
    let home = environment
        .variable("HOME")
        .filter(|value| !value.is_empty())?;
    Some(
        GLOBAL_CONFIG_RELATIVE_PATH
            .iter()
            .fold(PathBuf::from(home), |path, component| path.join(component)),
    )
}

fn optional_global_config_files(dir: PathBuf) -> Result<Vec<PathBuf>, RepositoryError> {
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(RepositoryError::ConfigRead {
                file: config_file_label(&dir),
                source,
            });
        }
    };

    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| RepositoryError::ConfigRead {
            file: config_file_label(&dir),
            source,
        })?;
        let path = entry.path();
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "toml")
            && !path.is_dir()
        {
            files.push(path);
        }
    }
    files.sort();

    Ok(files)
}
