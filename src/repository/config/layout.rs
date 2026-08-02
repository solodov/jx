use super::*;

const DEFAULT_LAYOUT_SOURCE: &str = "github";
const DEFAULT_LAYOUT_ROOT: &str = "~/src";
const DEFAULT_LAYOUT_WORKSPACE_DIR: &str = ".work";
const DEFAULT_LAYOUT_PATH_TEMPLATE: &str = "{host}/{owner}/{repo}";

/// Clone destination and remote URL resolved from layout config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClonePlan {
    pub identity: RepositoryIdentity,
    pub remote_url: String,
    pub destination: PathBuf,
}

/// Canonical repository identity used by clone and workspace layout resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryIdentity {
    pub source: String,
    pub host: String,
    pub owner: String,
    pub repo: String,
}

impl RepositoryIdentity {
    /// Returns `source:owner/repo` for diagnostics where host alone may be ambiguous.
    pub fn summary(&self) -> String {
        format!("{}:{}/{}", self.source, self.owner, self.repo)
    }

    /// Converts this identity to the GitHub repository shape when the source is GitHub-like.
    pub fn github_repository(&self) -> GitHubRepository {
        GitHubRepository {
            owner: self.owner.clone(),
            name: self.repo.clone(),
        }
    }
}

/// Opinionated clone and workspace placement loaded from optional config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutConfig {
    pub default_source: String,
    pub default_root: String,
    pub workspace_dir: String,
    pub sources: BTreeMap<String, LayoutSourceConfig>,
    pub default: LayoutDefaultConfig,
    pub rules: Vec<LayoutRuleConfig>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        let github = LayoutSourceConfig {
            name: DEFAULT_LAYOUT_SOURCE.to_owned(),
            provider: LayoutProvider::GitHub,
            host: "github.com".to_owned(),
            clone_url: CloneUrlFormat::Ssh,
        };
        let mut sources = BTreeMap::new();
        sources.insert(github.name.clone(), github);

        Self {
            default_source: DEFAULT_LAYOUT_SOURCE.to_owned(),
            default_root: DEFAULT_LAYOUT_ROOT.to_owned(),
            workspace_dir: DEFAULT_LAYOUT_WORKSPACE_DIR.to_owned(),
            sources,
            default: LayoutDefaultConfig {
                path: DEFAULT_LAYOUT_PATH_TEMPLATE.to_owned(),
            },
            rules: Vec::new(),
        }
    }
}

impl LayoutConfig {
    pub(super) fn apply_layer(&mut self, layer: LayoutConfigLayer) {
        if let Some(default_source) = layer.default_source {
            self.default_source = default_source;
        }
        if let Some(default_root) = layer.default_root {
            self.default_root = default_root;
        }
        if let Some(workspace_dir) = layer.workspace_dir {
            self.workspace_dir = workspace_dir;
        }
        if let Some(path) = layer.default_path {
            self.default.path = path;
        }
        for source in layer.sources {
            self.sources.insert(source.name.clone(), source);
        }
        self.rules.extend(layer.rules);
    }

    pub(super) fn validate(&self) -> Result<(), RepositoryError> {
        if !self.sources.contains_key(&self.default_source) {
            return Err(RepositoryError::InvalidConfig {
                file: "jx config".to_owned(),
                message: format!(
                    "`layout.default_source` references `{}`, but no `[[layout.sources]]` source is configured with that name",
                    self.default_source
                ),
            });
        }
        validate_single_path_segment("layout.workspace_dir", &self.workspace_dir)?;
        validate_layout_template("layout.default.path", &self.default.path)?;

        for (index, rule) in self.rules.iter().enumerate() {
            if !self.sources.contains_key(&rule.source) {
                return Err(RepositoryError::InvalidConfig {
                    file: "jx config".to_owned(),
                    message: format!(
                        "`layout.rules[{index}].source` references `{}`, but no `[[layout.sources]]` source is configured with that name",
                        rule.source
                    ),
                });
            }
            if rule.owner.is_none() && rule.repo.is_none() {
                return Err(RepositoryError::InvalidConfig {
                    file: "jx config".to_owned(),
                    message: format!(
                        "`layout.rules[{index}]` must specify at least one of `owner` or `repo`"
                    ),
                });
            }
            if let Some(path) = &rule.path {
                validate_layout_template(&format!("layout.rules[{index}].path"), path)?;
            }
        }

        Ok(())
    }

    /// Resolves a clone input to its remote URL and deterministic destination path.
    pub fn clone_plan(
        &self,
        repository: &str,
        explicit_destination: Option<&Path>,
        environment: &RuntimeEnvironment,
    ) -> Result<ClonePlan, RepositoryError> {
        let parsed = self.parse_clone_repository(repository, environment)?;
        let destination = match explicit_destination {
            Some(destination) => resolve_operator_path(destination, environment)?,
            None => self.project_destination(&parsed.identity, environment)?,
        };

        Ok(ClonePlan {
            identity: parsed.identity,
            remote_url: parsed.remote_url,
            destination,
        })
    }

    /// Resolves the layout primary checkout path and requires an existing jj clone.
    pub fn locate_clone(
        &self,
        repository: &str,
        environment: &RuntimeEnvironment,
    ) -> Result<ClonePlan, RepositoryError> {
        let plan = self.clone_plan(repository, None, environment)?;
        if plan.destination.join(".jj").is_dir() {
            return Ok(plan);
        }

        Err(RepositoryError::LayoutCloneNotFound {
            repository: repository.to_owned(),
            path: plan.destination,
        })
    }

    /// Resolves the visible project checkout path for a repository identity.
    pub fn project_destination(
        &self,
        identity: &RepositoryIdentity,
        environment: &RuntimeEnvironment,
    ) -> Result<PathBuf, RepositoryError> {
        let (root, relative) = self.layout_root_and_relative(identity, environment)?;
        Ok(root.join(relative))
    }

    /// Resolves the hidden workspace storage root for the selected layout root.
    pub fn workspace_storage_root(
        &self,
        identity: &RepositoryIdentity,
        environment: &RuntimeEnvironment,
    ) -> Result<PathBuf, RepositoryError> {
        let (root, _) = self.layout_root_and_relative(identity, environment)?;
        Ok(root.join(&self.workspace_dir))
    }

    /// Resolves the hidden parent directory that contains managed workspaces for a repository.
    pub fn workspace_collection_root(
        &self,
        identity: &RepositoryIdentity,
        environment: &RuntimeEnvironment,
    ) -> Result<PathBuf, RepositoryError> {
        let (_, relative) = self.layout_root_and_relative(identity, environment)?;
        Ok(self
            .workspace_storage_root(identity, environment)?
            .join(relative))
    }

    /// Resolves a managed workspace destination under the configured hidden workspace layout.
    pub fn workspace_destination(
        &self,
        identity: &RepositoryIdentity,
        workspace_name: &str,
        environment: &RuntimeEnvironment,
    ) -> Result<PathBuf, RepositoryError> {
        validate_workspace_name(workspace_name)?;
        Ok(self
            .workspace_collection_root(identity, environment)?
            .join(workspace_name))
    }

    /// Returns every configured layout root that can contain repositories or managed workspaces.
    pub fn configured_roots(
        &self,
        environment: &RuntimeEnvironment,
    ) -> Result<Vec<PathBuf>, RepositoryError> {
        let mut roots = vec![resolve_config_root(&self.default_root, environment)?];
        for rule in &self.rules {
            if let Some(root) = &rule.root {
                roots.push(resolve_config_root(root, environment)?);
            }
        }
        roots.sort();
        roots.dedup();
        Ok(roots)
    }

    /// Resolves repository identity from an explicit remote URL using configured layout sources.
    pub fn identity_for_remote_url(
        &self,
        remote_url: &str,
    ) -> Result<RepositoryIdentity, RepositoryError> {
        let Some(parsed) = parse_explicit_clone_url(remote_url)? else {
            return Err(RepositoryError::InvalidCloneRepository {
                repository: remote_url.to_owned(),
                message: "remote URL must be an explicit Git URL".to_owned(),
            });
        };

        self.repository_from_explicit_url(parsed, remote_url)
            .map(|repository| repository.identity)
    }

    fn parse_clone_repository(
        &self,
        repository: &str,
        environment: &RuntimeEnvironment,
    ) -> Result<ParsedCloneRepository, RepositoryError> {
        let repository = repository.trim();
        if repository.is_empty() {
            return Err(RepositoryError::InvalidCloneRepository {
                repository: repository.to_owned(),
                message: "repository must not be empty".to_owned(),
            });
        }

        if let Some(parsed) = parse_explicit_clone_url(repository)? {
            return self.repository_from_explicit_url(parsed, repository);
        }

        if let Some((source, slug)) = parse_explicit_source_slug(repository) {
            return self.repository_from_source_slug(source, slug);
        }

        let parts = repository.split('/').collect::<Vec<_>>();
        match parts.as_slice() {
            [repo] => self.repository_from_current_layout_prefix(repo, environment),
            [owner, repo] => self.repository_from_source_slug(&self.default_source, (*owner, *repo)),
            [host, owner, repo] => self.repository_from_host_slug(host, owner, repo),
            _ => Err(RepositoryError::InvalidCloneRepository {
                repository: repository.to_owned(),
                message: "use `repo` from a configured layout prefix, `owner/repo`, `host/owner/repo`, `source:owner/repo`, or an explicit Git URL".to_owned(),
            }),
        }
    }

    fn repository_from_current_layout_prefix(
        &self,
        repo: &str,
        environment: &RuntimeEnvironment,
    ) -> Result<ParsedCloneRepository, RepositoryError> {
        let repo = normalize_repo_name(repo)?;
        let mut candidates = Vec::new();

        for source in self.sources.values() {
            if let Some(candidate) = self.current_layout_prefix_candidate(
                source,
                &self.default_root,
                &self.default.path,
                (None, None),
                &repo,
                environment,
            )? {
                candidates.push(candidate);
            }

            for rule in self.rules.iter().filter(|rule| rule.source == source.name) {
                if let Some(candidate) = self.current_layout_prefix_candidate(
                    source,
                    rule.root.as_deref().unwrap_or(&self.default_root),
                    rule.path.as_deref().unwrap_or(&self.default.path),
                    (rule.owner.as_deref(), rule.repo.as_deref()),
                    &repo,
                    environment,
                )? {
                    candidates.push(candidate);
                }
            }
        }

        candidates.sort_by(|left, right| {
            parsed_clone_repository_signature(left).cmp(&parsed_clone_repository_signature(right))
        });
        candidates.dedup_by(|left, right| {
            parsed_clone_repository_signature(left) == parsed_clone_repository_signature(right)
        });

        match candidates.as_slice() {
            [candidate] => Ok(candidate.clone()),
            [] => Err(RepositoryError::InvalidCloneRepository {
                repository: repo,
                message: "repo-only shorthands require running from a configured layout prefix; use `owner/repo`, `host/owner/repo`, `source:owner/repo`, or an explicit Git URL".to_owned(),
            }),
            _ => Err(RepositoryError::InvalidCloneRepository {
                repository: repo,
                message: format!(
                    "repo-only shorthand from `{}` matches multiple layout identities: {:?}; use `owner/repo`, `host/owner/repo`, `source:owner/repo`, or an explicit Git URL",
                    environment.current_dir().display(),
                    candidates
                        .iter()
                        .map(|candidate| candidate.identity.summary())
                        .collect::<Vec<_>>()
                ),
            }),
        }
    }

    fn current_layout_prefix_candidate(
        &self,
        source: &LayoutSourceConfig,
        root: &str,
        path_template: &str,
        rule_identity: (Option<&str>, Option<&str>),
        repo: &str,
        environment: &RuntimeEnvironment,
    ) -> Result<Option<ParsedCloneRepository>, RepositoryError> {
        let Some(identity) = identity_from_current_layout_prefix(
            source,
            root,
            path_template,
            rule_identity,
            repo,
            environment,
        )?
        else {
            return Ok(None);
        };

        let destination = self.project_destination(&identity, environment)?;
        if destination.parent() != Some(environment.current_dir()) {
            return Ok(None);
        }

        Ok(Some(ParsedCloneRepository {
            remote_url: source.clone_url.remote_url(&identity),
            identity,
        }))
    }

    fn repository_from_explicit_url(
        &self,
        parsed: ParsedCloneUrl,
        remote_url: &str,
    ) -> Result<ParsedCloneRepository, RepositoryError> {
        let source = self.source_name_for_host(&parsed.host)?;
        let source = source.unwrap_or_else(|| parsed.host.clone());
        let identity = RepositoryIdentity {
            source,
            host: parsed.host,
            owner: parsed.owner,
            repo: parsed.repo,
        };

        Ok(ParsedCloneRepository {
            identity,
            remote_url: remote_url.to_owned(),
        })
    }

    fn repository_from_source_slug(
        &self,
        source: &str,
        slug: (&str, &str),
    ) -> Result<ParsedCloneRepository, RepositoryError> {
        let source = source.trim();
        let Some(source_config) = self.sources.get(source) else {
            return Err(RepositoryError::UnknownLayoutSource {
                name: source.to_owned(),
            });
        };
        let identity = RepositoryIdentity {
            source: source.to_owned(),
            host: source_config.host.clone(),
            owner: normalize_repo_component(slug.0, "owner")?,
            repo: normalize_repo_name(slug.1)?,
        };
        let remote_url = source_config.clone_url.remote_url(&identity);

        Ok(ParsedCloneRepository {
            identity,
            remote_url,
        })
    }

    fn repository_from_host_slug(
        &self,
        host: &str,
        owner: &str,
        repo: &str,
    ) -> Result<ParsedCloneRepository, RepositoryError> {
        let host = normalize_host(host)?;
        let source = self.source_name_for_host(&host)?;
        let source_config = source.and_then(|name| self.sources.get(&name));
        let identity = RepositoryIdentity {
            source: source_config
                .map(|source| source.name.clone())
                .unwrap_or_else(|| host.clone()),
            host,
            owner: normalize_repo_component(owner, "owner")?,
            repo: normalize_repo_name(repo)?,
        };
        let remote_url = source_config
            .map(|source| source.clone_url.remote_url(&identity))
            .unwrap_or_else(|| CloneUrlFormat::Https.remote_url(&identity));

        Ok(ParsedCloneRepository {
            identity,
            remote_url,
        })
    }

    fn source_name_for_host(&self, host: &str) -> Result<Option<String>, RepositoryError> {
        let matches = self
            .sources
            .values()
            .filter(|source| source.host == host)
            .map(|source| source.name.clone())
            .collect::<Vec<_>>();

        match matches.as_slice() {
            [] => Ok(None),
            [source] => Ok(Some(source.clone())),
            _ => Err(RepositoryError::AmbiguousLayoutHost {
                host: host.to_owned(),
                sources: matches,
            }),
        }
    }

    fn layout_root_and_relative(
        &self,
        identity: &RepositoryIdentity,
        environment: &RuntimeEnvironment,
    ) -> Result<(PathBuf, PathBuf), RepositoryError> {
        let (root, path) = self.selected_layout(identity);
        Ok((
            resolve_config_root(root, environment)?,
            render_layout_path(path, identity)?,
        ))
    }

    fn selected_layout(&self, identity: &RepositoryIdentity) -> (&str, &str) {
        let mut root = self.default_root.as_str();
        let mut path = self.default.path.as_str();

        for rule in self.rules.iter().filter(|rule| rule.matches(identity)) {
            if let Some(rule_root) = &rule.root {
                root = rule_root;
            }
            if let Some(rule_path) = &rule.path {
                path = rule_path;
            }
        }

        (root, path)
    }

    /// Resolves the expected repository identity for an existing workspace path.
    pub fn identity_for_workspace_root(
        &self,
        workspace_root: &Path,
        environment: &RuntimeEnvironment,
    ) -> Result<RepositoryIdentity, RepositoryError> {
        let mut identities = Vec::new();

        for source in self.sources.values() {
            if let Some(identity) = self.inverse_layout_candidate(
                source,
                &self.default_root,
                &self.default.path,
                (None, None),
                workspace_root,
                environment,
            )? {
                identities.push(identity);
            }

            for rule in self.rules.iter().filter(|rule| rule.source == source.name) {
                if let Some(identity) = self.inverse_layout_candidate(
                    source,
                    rule.root.as_deref().unwrap_or(&self.default_root),
                    rule.path.as_deref().unwrap_or(&self.default.path),
                    (rule.owner.as_deref(), rule.repo.as_deref()),
                    workspace_root,
                    environment,
                )? {
                    identities.push(identity);
                }
            }
        }

        identities.sort_by(|left, right| {
            (&left.source, &left.owner, &left.repo).cmp(&(&right.source, &right.owner, &right.repo))
        });
        identities.dedup();

        match identities.as_slice() {
            [identity] => Ok(identity.clone()),
            [] => Err(RepositoryError::LayoutPathNotMatched {
                path: workspace_root.to_path_buf(),
            }),
            _ => Err(RepositoryError::AmbiguousLayoutPath {
                path: workspace_root.to_path_buf(),
                identities: identities
                    .into_iter()
                    .map(|identity| identity.summary())
                    .collect(),
            }),
        }
    }

    /// Builds the configured remote URL for a normalized repository identity.
    pub fn remote_url_for_identity(
        &self,
        identity: &RepositoryIdentity,
    ) -> Result<String, RepositoryError> {
        let Some(source) = self.sources.get(&identity.source) else {
            return Err(RepositoryError::UnknownLayoutSource {
                name: identity.source.clone(),
            });
        };
        Ok(source.clone_url.remote_url(identity))
    }

    fn inverse_layout_candidate(
        &self,
        source: &LayoutSourceConfig,
        root: &str,
        path_template: &str,
        rule_identity: (Option<&str>, Option<&str>),
        workspace_root: &Path,
        environment: &RuntimeEnvironment,
    ) -> Result<Option<RepositoryIdentity>, RepositoryError> {
        let root = resolve_config_root(root, environment)?;
        let Ok(relative) = workspace_root.strip_prefix(&root) else {
            return Ok(None);
        };

        if let Some(identity) = self.inverse_project_layout_candidate(
            source,
            path_template,
            rule_identity,
            relative,
            workspace_root,
            environment,
        )? {
            return Ok(Some(identity));
        }

        let Some((workspace_name, managed_relative)) = self.managed_workspace_relative(relative)
        else {
            return Ok(None);
        };
        let Some(identity) =
            self.inverse_identity(source, path_template, rule_identity, &managed_relative)
        else {
            return Ok(None);
        };
        if self.workspace_destination(&identity, &workspace_name, environment)? == workspace_root {
            Ok(Some(identity))
        } else {
            Ok(None)
        }
    }

    fn inverse_project_layout_candidate(
        &self,
        source: &LayoutSourceConfig,
        path_template: &str,
        rule_identity: (Option<&str>, Option<&str>),
        relative: &Path,
        workspace_root: &Path,
        environment: &RuntimeEnvironment,
    ) -> Result<Option<RepositoryIdentity>, RepositoryError> {
        let Some(identity) = self.inverse_identity(source, path_template, rule_identity, relative)
        else {
            return Ok(None);
        };
        if self.project_destination(&identity, environment)? == workspace_root {
            Ok(Some(identity))
        } else {
            Ok(None)
        }
    }

    fn inverse_identity(
        &self,
        source: &LayoutSourceConfig,
        path_template: &str,
        rule_identity: (Option<&str>, Option<&str>),
        relative: &Path,
    ) -> Option<RepositoryIdentity> {
        let mut identity = match_layout_path_template(source, path_template, relative)?;

        if let Some(owner) = rule_identity.0 {
            identity.owner = owner.to_owned();
        }
        if let Some(repo) = rule_identity.1 {
            identity.repo = repo.to_owned();
        }
        (!identity.owner.is_empty() && !identity.repo.is_empty()).then_some(identity)
    }

    fn managed_workspace_relative(&self, relative: &Path) -> Option<(String, PathBuf)> {
        let components = path_components(relative)?;
        if components.len() < 3 || components.first()? != &self.workspace_dir {
            return None;
        }
        let workspace_name = components.last()?;
        if !is_valid_layout_name(workspace_name) {
            return None;
        }
        let mut managed_relative = PathBuf::new();
        for component in &components[1..components.len() - 1] {
            managed_relative.push(component);
        }
        Some((workspace_name.clone(), managed_relative))
    }
}

/// Remote family used to expand repository shorthands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutSourceConfig {
    pub name: String,
    pub provider: LayoutProvider,
    pub host: String,
    pub clone_url: CloneUrlFormat,
}

/// Supported source provider semantics for shorthand parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutProvider {
    GitHub,
}

/// Generated clone URL shape for shorthand repository inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneUrlFormat {
    Ssh,
    Https,
}

impl CloneUrlFormat {
    fn remote_url(self, identity: &RepositoryIdentity) -> String {
        match self {
            Self::Ssh => format!(
                "git@{}:{}/{}.git",
                identity.host, identity.owner, identity.repo
            ),
            Self::Https => format!(
                "https://{}/{}/{}.git",
                identity.host, identity.owner, identity.repo
            ),
        }
    }
}

/// Default path template used when no layout rule matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDefaultConfig {
    pub path: String,
}

/// Repo placement override for a selected source, owner, and optional repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutRuleConfig {
    pub source: String,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub root: Option<String>,
    pub path: Option<String>,
}

impl LayoutRuleConfig {
    fn matches(&self, identity: &RepositoryIdentity) -> bool {
        self.source == identity.source
            && self
                .owner
                .as_ref()
                .is_none_or(|owner| owner == &identity.owner)
            && self.repo.as_ref().is_none_or(|repo| repo == &identity.repo)
    }
}

#[derive(Debug, Default)]
pub(super) struct LayoutConfigLayer {
    pub(super) default_source: Option<String>,
    pub(super) default_root: Option<String>,
    pub(super) workspace_dir: Option<String>,
    pub(super) sources: Vec<LayoutSourceConfig>,
    pub(super) default_path: Option<String>,
    pub(super) rules: Vec<LayoutRuleConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCloneRepository {
    identity: RepositoryIdentity,
    remote_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCloneUrl {
    host: String,
    owner: String,
    repo: String,
}

fn parsed_clone_repository_signature(
    repository: &ParsedCloneRepository,
) -> (&str, &str, &str, &str, &str) {
    (
        &repository.identity.source,
        &repository.identity.host,
        &repository.identity.owner,
        &repository.identity.repo,
        &repository.remote_url,
    )
}

fn identity_from_current_layout_prefix(
    source: &LayoutSourceConfig,
    root: &str,
    path_template: &str,
    rule_identity: (Option<&str>, Option<&str>),
    repo: &str,
    environment: &RuntimeEnvironment,
) -> Result<Option<RepositoryIdentity>, RepositoryError> {
    if rule_identity.1.is_some_and(|rule_repo| rule_repo != repo) {
        return Ok(None);
    }

    let root = resolve_config_root(root, environment)?;
    let Ok(relative) = environment.current_dir().strip_prefix(&root) else {
        return Ok(None);
    };
    let Some(template_components) = template_path_components(path_template) else {
        return Ok(None);
    };
    let Some((last_template_component, prefix_template_components)) =
        template_components.split_last()
    else {
        return Ok(None);
    };
    if last_template_component != "{repo}" {
        return Ok(None);
    }

    let Some(relative_components) = path_components(relative) else {
        return Ok(None);
    };
    if relative_components.len() != prefix_template_components.len() {
        return Ok(None);
    }

    let mut owner = None;
    for (template, value) in prefix_template_components.iter().zip(relative_components) {
        match template.as_str() {
            "{source}" if value == source.name => {}
            "{source}" => return Ok(None),
            "{host}" if value == source.host => {}
            "{host}" => return Ok(None),
            "{owner}" if is_valid_repo_component(&value) => {
                if owner
                    .as_deref()
                    .is_some_and(|owner| owner != value.as_str())
                {
                    return Ok(None);
                }
                owner = Some(value);
            }
            "{owner}" => return Ok(None),
            "{repo}" => return Ok(None),
            literal if literal == value.as_str() => {}
            _ => return Ok(None),
        }
    }

    if let Some(rule_owner) = rule_identity.0 {
        if owner.as_deref().is_some_and(|owner| owner != rule_owner) {
            return Ok(None);
        }
        owner = Some(rule_owner.to_owned());
    }
    let Some(owner) = owner else {
        return Ok(None);
    };

    Ok(Some(RepositoryIdentity {
        source: source.name.clone(),
        host: source.host.clone(),
        owner,
        repo: repo.to_owned(),
    }))
}

fn match_layout_path_template(
    source: &LayoutSourceConfig,
    template: &str,
    relative: &Path,
) -> Option<RepositoryIdentity> {
    let template_components = template_path_components(template)?;
    let relative_components = path_components(relative)?;
    if template_components.len() != relative_components.len() {
        return None;
    }

    let mut source_name = source.name.clone();
    let mut host = source.host.clone();
    let mut owner = String::new();
    let mut repo = String::new();

    for (template, value) in template_components.iter().zip(relative_components) {
        match template.as_str() {
            "{source}" if value == source.name => source_name = value,
            "{source}" => return None,
            "{host}" if value == source.host => host = value,
            "{host}" => return None,
            "{owner}" if is_valid_repo_component(&value) => owner = value,
            "{owner}" => return None,
            "{repo}" if is_valid_repo_component(&value) => repo = value,
            "{repo}" => return None,
            literal if literal == value => {}
            _ => return None,
        }
    }

    Some(RepositoryIdentity {
        source: source_name,
        host,
        owner,
        repo,
    })
}

fn template_path_components(template: &str) -> Option<Vec<String>> {
    Path::new(template)
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str().map(str::to_owned),
            _ => None,
        })
        .collect()
}

fn path_components(path: &Path) -> Option<Vec<String>> {
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str().map(str::to_owned),
            _ => None,
        })
        .collect()
}

fn parse_explicit_clone_url(repository: &str) -> Result<Option<ParsedCloneUrl>, RepositoryError> {
    let Some((host, path)) = explicit_url_host_and_path(repository) else {
        return Ok(None);
    };
    let host = normalize_host(host)?;
    let (owner, repo) = parse_owner_repo_path(repository, path)?;

    Ok(Some(ParsedCloneUrl { host, owner, repo }))
}

fn explicit_url_host_and_path(repository: &str) -> Option<(&str, &str)> {
    if let Some(rest) = repository.strip_prefix("https://") {
        return rest.split_once('/');
    }
    if let Some(rest) = repository.strip_prefix("ssh://") {
        let (host, path) = rest.split_once('/')?;
        return Some((host.rsplit_once('@').map_or(host, |(_, host)| host), path));
    }
    let (user_and_host, path) = repository.split_once(':')?;
    let (_, host) = user_and_host.split_once('@')?;
    Some((host, path))
}

fn parse_explicit_source_slug(repository: &str) -> Option<(&str, (&str, &str))> {
    let (source, slug) = repository.split_once(':')?;
    if source.contains('/') || source.is_empty() || repository.contains("://") {
        return None;
    }
    let (owner, repo) = slug.split_once('/')?;
    if owner.contains('/') || repo.contains('/') {
        return None;
    }
    Some((source, (owner, repo)))
}

fn parse_owner_repo_path(
    repository: &str,
    path: &str,
) -> Result<(String, String), RepositoryError> {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut components = path.split('/');
    let owner = components.next().unwrap_or_default();
    let repo = components.next().unwrap_or_default();

    if components.next().is_some() {
        return Err(RepositoryError::InvalidCloneRepository {
            repository: repository.to_owned(),
            message: "repository URLs must contain exactly `owner/repo` after the host".to_owned(),
        });
    }

    Ok((
        normalize_repo_component(owner, "owner")?,
        normalize_repo_name(repo)?,
    ))
}

pub(super) fn normalize_host(host: &str) -> Result<String, RepositoryError> {
    let host = host.trim().trim_end_matches('.');
    if !host.is_empty()
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        && host.contains('.')
    {
        Ok(host.to_ascii_lowercase())
    } else {
        Err(RepositoryError::InvalidCloneRepository {
            repository: host.to_owned(),
            message: "host must be a DNS-like name such as `github.com`".to_owned(),
        })
    }
}

pub(super) fn normalize_repo_name(repo: &str) -> Result<String, RepositoryError> {
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    normalize_repo_component(repo, "repo")
}

pub(super) fn normalize_repo_component(
    component: &str,
    name: &str,
) -> Result<String, RepositoryError> {
    let component = component.trim();
    if is_valid_repo_component(component) {
        Ok(component.to_owned())
    } else {
        Err(RepositoryError::InvalidCloneRepository {
            repository: component.to_owned(),
            message: format!("{name} must be a single repository path segment"),
        })
    }
}

pub(super) fn is_valid_repo_component(component: &str) -> bool {
    !component.is_empty()
        && !component.starts_with('.')
        && !component.ends_with('.')
        && component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub fn validate_workspace_name(name: &str) -> Result<(), RepositoryError> {
    if is_valid_layout_name(name) {
        Ok(())
    } else {
        Err(RepositoryError::InvalidWorkspaceName {
            name: name.to_owned(),
            message: "workspace name must contain only letters, numbers, `_`, or `-`".to_owned(),
        })
    }
}

pub(super) fn is_valid_layout_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(super) fn validate_single_path_segment(
    key: &str,
    segment: &str,
) -> Result<(), RepositoryError> {
    let path = Path::new(segment);
    let mut components = path.components();
    if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
        return Ok(());
    }

    Err(RepositoryError::InvalidConfig {
        file: "jx config".to_owned(),
        message: format!("`{key}` must be a single path segment"),
    })
}

pub(super) fn validate_layout_template(key: &str, template: &str) -> Result<(), RepositoryError> {
    if template.trim().is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: "jx config".to_owned(),
            message: format!("`{key}` must not be empty"),
        });
    }

    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start + 1..].find('}') else {
            return Err(RepositoryError::InvalidConfig {
                file: "jx config".to_owned(),
                message: format!("`{key}` contains an unclosed template placeholder"),
            });
        };
        let placeholder = &rest[start + 1..start + 1 + end];
        if !matches!(placeholder, "source" | "host" | "owner" | "repo") {
            return Err(RepositoryError::InvalidConfig {
                file: "jx config".to_owned(),
                message: format!("`{key}` contains unsupported placeholder `{{{placeholder}}}`"),
            });
        }
        rest = &rest[start + 1 + end + 1..];
    }
    if rest.contains('}') {
        return Err(RepositoryError::InvalidConfig {
            file: "jx config".to_owned(),
            message: format!("`{key}` contains an unopened template placeholder"),
        });
    }

    Ok(())
}

fn render_layout_path(
    template: &str,
    identity: &RepositoryIdentity,
) -> Result<PathBuf, RepositoryError> {
    let rendered = template
        .replace("{source}", &identity.source)
        .replace("{host}", &identity.host)
        .replace("{owner}", &identity.owner)
        .replace("{repo}", &identity.repo);
    let path = PathBuf::from(rendered);
    if path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        Ok(path)
    } else {
        Err(RepositoryError::InvalidConfig {
            file: "jx config".to_owned(),
            message: "layout path templates must render relative paths without `.` or `..`"
                .to_owned(),
        })
    }
}

fn resolve_config_root(
    root: &str,
    environment: &RuntimeEnvironment,
) -> Result<PathBuf, RepositoryError> {
    let path = expand_tilde_path(root, environment)?;
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(RepositoryError::InvalidConfig {
            file: "jx config".to_owned(),
            message: format!("layout root `{root}` must be absolute or start with `~/`"),
        })
    }
}

fn resolve_operator_path(
    path: &Path,
    environment: &RuntimeEnvironment,
) -> Result<PathBuf, RepositoryError> {
    let path = expand_tilde_path(&path.to_string_lossy(), environment)?;
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(environment.current_dir().join(path))
    }
}

fn expand_tilde_path(
    path: &str,
    environment: &RuntimeEnvironment,
) -> Result<PathBuf, RepositoryError> {
    let path = path.trim();
    if path == "~" {
        return home_path(path, environment);
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return Ok(home_path(path, environment)?.join(rest));
    }
    Ok(PathBuf::from(path))
}

fn home_path(path: &str, environment: &RuntimeEnvironment) -> Result<PathBuf, RepositoryError> {
    environment
        .variable("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| RepositoryError::MissingHomeForLayout {
            path: path.to_owned(),
        })
}
