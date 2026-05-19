use super::*;
use globset::Glob;
use std::fs;

/// One globally navigable work location discovered from the configured layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkLocation {
    pub(super) key: String,
    pub(super) root: PathBuf,
}

/// One primary repository checkout discovered from the configured layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkRepository {
    pub(super) key: String,
    pub(super) root: PathBuf,
}

/// Complete command plan for adding a managed jj workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkAddPlan {
    pub(super) identity: RepositoryIdentity,
    pub(super) primary_checkout_root: PathBuf,
    pub(super) destination: PathBuf,
    pub(super) workspace_name: String,
    pub(super) revision: Option<String>,
    pub(super) task_id: Option<String>,
    pub(super) shared_paths: PlannedSharedWorkspacePaths,
}

impl WorkAddPlan {
    pub(super) fn workspace_options(&self) -> WorkspaceAddOptions {
        WorkspaceAddOptions {
            name: self.workspace_name.clone(),
            destination: self.destination.clone(),
            revision: self.revision.clone(),
            shared_paths: self
                .shared_paths
                .link_candidates
                .iter()
                .map(|candidate| candidate.relative_path.clone())
                .collect(),
        }
    }
}

/// Post-create setup failure for an already-created managed workspace.
#[derive(Debug, Error)]
#[error("{source}")]
pub(super) struct WorkAddSetupError {
    workspace: String,
    destination: PathBuf,
    #[source]
    source: Box<WorkAddSetupErrorSource>,
}

impl WorkAddSetupError {
    fn new(plan: &WorkAddPlan, source: impl Into<WorkAddSetupErrorSource>) -> Self {
        Self {
            workspace: plan.workspace_name.clone(),
            destination: plan.destination.clone(),
            source: Box::new(source.into()),
        }
    }

    pub(super) fn workspace(&self) -> &str {
        &self.workspace
    }

    pub(super) fn destination(&self) -> &Path {
        &self.destination
    }
}

#[derive(Debug, Error)]
enum WorkAddSetupErrorSource {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error("could not {action} shared workspace path `{relative_path}` at {path}: {source}")]
    SharedPathIo {
        action: &'static str,
        relative_path: String,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Effective shared-path policy split by source existence in the primary checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlannedSharedWorkspacePaths {
    pub(super) effective_paths: Vec<String>,
    pub(super) link_candidates: Vec<SharedWorkspacePathCandidate>,
    pub(super) missing_sources: Vec<MissingSharedWorkspacePath>,
}

/// One configured shared path that exists in the primary checkout and can be linked later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SharedWorkspacePathCandidate {
    pub(super) relative_path: String,
    pub(super) source: PathBuf,
    pub(super) destination: PathBuf,
}

/// One configured shared path skipped because the primary checkout has no source entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MissingSharedWorkspacePath {
    pub(super) relative_path: String,
    pub(super) source: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoveredWorkLocation {
    identity: RepositoryIdentity,
    workspace: Option<String>,
    root: PathBuf,
}

/// Builds the full work-add plan before crossing jj or filesystem mutation boundaries.
pub(super) fn plan_work_add(
    request: &WorkAddRequest,
    context: &LocalRepositoryContext,
    environment: &RuntimeEnvironment,
) -> Result<WorkAddPlan, CommandError> {
    let task_id = domain::normalize_task_id(request.task_id.as_deref())?;
    let workspace_name = workspace_name_for_task(&request.name, task_id.as_deref());
    validate_workspace_name(&workspace_name)?;
    let identity = workspace_identity(context, environment)?;
    let primary_checkout_root = context
        .config
        .layout
        .project_destination(&identity, environment)?;
    let destination =
        context
            .config
            .layout
            .workspace_destination(&identity, &workspace_name, environment)?;
    let effective_paths = context
        .config
        .repo
        .workspace_shared_paths_for(&identity.github_repository())?;
    let shared_paths =
        plan_shared_workspace_paths(&primary_checkout_root, &destination, effective_paths);

    Ok(WorkAddPlan {
        identity,
        primary_checkout_root,
        destination,
        workspace_name,
        revision: request.revision.clone(),
        task_id,
        shared_paths,
    })
}

/// Applies setup steps that happen only after jj has created the workspace.
pub(super) fn apply_work_add_setup(plan: &WorkAddPlan) -> Result<(), WorkAddSetupError> {
    write_work_add_metadata(plan)?;
    apply_shared_workspace_paths(plan)?;
    Ok(())
}

fn write_work_add_metadata(plan: &WorkAddPlan) -> Result<(), WorkAddSetupError> {
    let Some(task_id) = &plan.task_id else {
        return Ok(());
    };

    write_workspace_metadata(
        &plan.destination,
        &WorkspaceMetadata {
            task_id: Some(task_id.clone()),
        },
    )
    .map_err(|source| WorkAddSetupError::new(plan, source))
}

fn apply_shared_workspace_paths(plan: &WorkAddPlan) -> Result<(), WorkAddSetupError> {
    for candidate in &plan.shared_paths.link_candidates {
        apply_shared_workspace_path(plan, candidate)?;
    }
    Ok(())
}

fn apply_shared_workspace_path(
    plan: &WorkAddPlan,
    candidate: &SharedWorkspacePathCandidate,
) -> Result<(), WorkAddSetupError> {
    if let Some(parent) = candidate.destination.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            shared_path_setup_error(
                plan,
                candidate,
                "create parent directories for",
                parent.to_path_buf(),
                source,
            )
        })?;
    }

    match fs::symlink_metadata(&candidate.destination) {
        Ok(_) => {
            return Err(shared_path_setup_error(
                plan,
                candidate,
                "create symlink for",
                candidate.destination.clone(),
                io::Error::new(io::ErrorKind::AlreadyExists, "destination already exists"),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(shared_path_setup_error(
                plan,
                candidate,
                "inspect destination for",
                candidate.destination.clone(),
                source,
            ));
        }
    }

    symlink_shared_workspace_path(&candidate.source, &candidate.destination).map_err(|source| {
        shared_path_setup_error(
            plan,
            candidate,
            "create symlink for",
            candidate.destination.clone(),
            source,
        )
    })
}

fn shared_path_setup_error(
    plan: &WorkAddPlan,
    candidate: &SharedWorkspacePathCandidate,
    action: &'static str,
    path: PathBuf,
    source: io::Error,
) -> WorkAddSetupError {
    WorkAddSetupError::new(
        plan,
        WorkAddSetupErrorSource::SharedPathIo {
            action,
            relative_path: candidate.relative_path.clone(),
            path,
            source,
        },
    )
}

fn symlink_shared_workspace_path(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, destination)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(source, destination)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (source, destination);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "symlink creation is unsupported on this platform",
        ))
    }
}

fn workspace_name_for_task(name: &str, task_id: Option<&str>) -> String {
    task_id.map_or_else(|| name.to_owned(), |task_id| format!("{task_id}-{name}"))
}

pub(super) fn workspace_identity(
    context: &LocalRepositoryContext,
    environment: &RuntimeEnvironment,
) -> Result<RepositoryIdentity, RepositoryError> {
    if let Some(remote) = context
        .remotes
        .iter()
        .find(|remote| remote.name == crate::repository::ORIGIN_REMOTE_NAME)
    {
        if let Ok(identity) = context.config.layout.identity_for_remote_url(&remote.url) {
            return Ok(identity);
        }
    }

    context
        .config
        .layout
        .identity_for_workspace_root(&context.workspace_root, environment)
}

fn plan_shared_workspace_paths(
    primary_checkout_root: &Path,
    destination_root: &Path,
    effective_paths: Vec<String>,
) -> PlannedSharedWorkspacePaths {
    let mut link_candidates = Vec::new();
    let mut missing_sources = Vec::new();

    for relative_path in &effective_paths {
        let relative = repo_relative_path(relative_path);
        let source = primary_checkout_root.join(&relative);
        if shared_path_source_exists(&source) {
            link_candidates.push(SharedWorkspacePathCandidate {
                relative_path: relative_path.clone(),
                source,
                destination: destination_root.join(relative),
            });
        } else {
            missing_sources.push(MissingSharedWorkspacePath {
                relative_path: relative_path.clone(),
                source,
            });
        }
    }

    PlannedSharedWorkspacePaths {
        effective_paths,
        link_candidates,
        missing_sources,
    }
}

fn shared_path_source_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn repo_relative_path(path: &str) -> PathBuf {
    path.split('/').fold(PathBuf::new(), |mut relative, part| {
        relative.push(part);
        relative
    })
}

/// Builds the global work-location index used by shell completion and path resolution.
pub(super) fn global_work_locations(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
) -> Result<Vec<WorkLocation>, RepositoryError> {
    let mut workspace_roots = Vec::new();
    let max_depth = max_work_location_depth(&config.layout);
    for root in config.layout.configured_roots(environment)? {
        collect_jj_workspace_roots(&root, max_depth, &mut workspace_roots);
    }
    workspace_roots.sort();
    workspace_roots.dedup();

    let mut discovered = Vec::new();
    for root in workspace_roots {
        if let Some(location) = discovered_work_location(&config.layout, &root, environment)? {
            discovered.push(location);
        }
    }

    Ok(assign_work_location_keys(discovered))
}

pub(super) fn filter_work_locations_by_prefix(
    locations: &[WorkLocation],
    prefix: &str,
) -> Vec<WorkLocation> {
    locations
        .iter()
        .filter(|location| location.key.starts_with(prefix))
        .cloned()
        .collect()
}

/// Builds the global primary-checkout index used by cross-repository commands.
pub(super) fn global_work_repositories(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
) -> Result<Vec<WorkRepository>, RepositoryError> {
    Ok(global_work_locations(config, environment)?
        .into_iter()
        .filter(|location| !location.key.contains('@'))
        .map(|location| WorkRepository {
            key: location.key,
            root: location.root,
        })
        .collect())
}

pub(super) fn filter_work_repositories_by_prefix(
    repositories: &[WorkRepository],
    prefix: &str,
) -> Vec<WorkRepository> {
    repositories
        .iter()
        .filter(|repository| repository.key.starts_with(prefix))
        .cloned()
        .collect()
}

pub(super) fn filter_workspace_entries_by_prefix(
    workspaces: &[WorkspaceEntry],
    prefix: &str,
) -> Vec<WorkspaceEntry> {
    workspaces
        .iter()
        .filter(|workspace| workspace.name.starts_with(prefix))
        .cloned()
        .collect()
}

pub(super) fn resolve_work_repository(
    repositories: &[WorkRepository],
    key: &str,
) -> Result<WorkRepository, RepositoryError> {
    repositories
        .iter()
        .find(|repository| repository.key == key)
        .cloned()
        .ok_or_else(|| RepositoryError::RepositoryFilterNotFound {
            pattern: key.to_owned(),
        })
}

pub(super) fn filter_work_repositories(
    repositories: &[WorkRepository],
    patterns: &[String],
) -> Result<Vec<WorkRepository>, RepositoryError> {
    if patterns.is_empty() {
        return Ok(repositories.to_vec());
    }

    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for pattern in patterns {
        let matches = matching_work_repositories(repositories, pattern)?;
        if matches.is_empty() {
            return Err(RepositoryError::RepositoryFilterNotFound {
                pattern: pattern.clone(),
            });
        }
        for repository in matches {
            if seen.insert(repository.key.clone()) {
                selected.push(repository.clone());
            }
        }
    }

    Ok(selected)
}

pub(super) fn resolve_work_location(
    locations: &[WorkLocation],
    key: &str,
) -> Result<PathBuf, RepositoryError> {
    let matches = locations
        .iter()
        .filter(|location| location.key == key)
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [location] => Ok(location.root.clone()),
        [] => Err(RepositoryError::WorkLocationNotFound {
            key: key.to_owned(),
        }),
        _ => Err(RepositoryError::WorkLocationAmbiguous {
            key: key.to_owned(),
            paths: matches
                .into_iter()
                .map(|location| location.root.clone())
                .collect(),
        }),
    }
}

fn matching_work_repositories<'a>(
    repositories: &'a [WorkRepository],
    pattern: &str,
) -> Result<Vec<&'a WorkRepository>, RepositoryError> {
    if contains_glob_meta(pattern) {
        let matcher = Glob::new(pattern)
            .map_err(|source| RepositoryError::InvalidRepositoryFilter {
                pattern: pattern.to_owned(),
                message: source.to_string(),
            })?
            .compile_matcher();
        Ok(repositories
            .iter()
            .filter(|repository| matcher.is_match(&repository.key))
            .collect())
    } else {
        Ok(repositories
            .iter()
            .filter(|repository| repository.key.starts_with(pattern))
            .collect())
    }
}

fn contains_glob_meta(pattern: &str) -> bool {
    pattern
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']'))
}

fn collect_jj_workspace_roots(root: &Path, max_depth: usize, roots: &mut Vec<PathBuf>) {
    collect_jj_workspace_roots_at_depth(root, 0, max_depth, roots);
}

fn collect_jj_workspace_roots_at_depth(
    path: &Path,
    depth: usize,
    max_depth: usize,
    roots: &mut Vec<PathBuf>,
) {
    if is_jj_workspace_root(path) {
        roots.push(path.to_path_buf());
        return;
    }
    if depth >= max_depth {
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    let mut children = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() && entry.file_name() != ".jj" {
            children.push(entry.path());
        }
    }
    children.sort();

    for child in children {
        collect_jj_workspace_roots_at_depth(&child, depth + 1, max_depth, roots);
    }
}

fn is_jj_workspace_root(path: &Path) -> bool {
    path.join(".jj").is_dir()
}

fn discovered_work_location(
    layout: &LayoutConfig,
    root: &Path,
    environment: &RuntimeEnvironment,
) -> Result<Option<DiscoveredWorkLocation>, RepositoryError> {
    let Ok(identity) = layout.identity_for_workspace_root(root, environment) else {
        return Ok(None);
    };

    if layout.project_destination(&identity, environment)? == root {
        return Ok(Some(DiscoveredWorkLocation {
            identity,
            workspace: None,
            root: root.to_path_buf(),
        }));
    }

    let Some(workspace) = root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
    else {
        return Ok(None);
    };
    if layout.workspace_destination(&identity, &workspace, environment)? != root {
        return Ok(None);
    }

    Ok(Some(DiscoveredWorkLocation {
        identity,
        workspace: Some(workspace),
        root: root.to_path_buf(),
    }))
}

fn assign_work_location_keys(locations: Vec<DiscoveredWorkLocation>) -> Vec<WorkLocation> {
    let identities = unique_identities(&locations);
    let repo_counts = identities
        .iter()
        .fold(BTreeMap::new(), |mut counts, identity| {
            *counts.entry(identity.repo.clone()).or_insert(0) += 1;
            counts
        });
    let owner_repo_counts = identities
        .iter()
        .fold(BTreeMap::new(), |mut counts, identity| {
            *counts
                .entry(format!("{}/{}", identity.owner, identity.repo))
                .or_insert(0) += 1;
            counts
        });

    let mut keyed = locations
        .into_iter()
        .map(|location| {
            let repo_key = repo_key(&location.identity, &repo_counts, &owner_repo_counts);
            let key = match location.workspace {
                Some(workspace) => format!("{repo_key}@{workspace}"),
                None => repo_key,
            };
            WorkLocation {
                key,
                root: location.root,
            }
        })
        .collect::<Vec<_>>();

    keyed.sort_by_key(work_location_sort_key);
    keyed.dedup();
    keyed
}

fn unique_identities(locations: &[DiscoveredWorkLocation]) -> Vec<RepositoryIdentity> {
    let mut identities = locations
        .iter()
        .map(|location| location.identity.clone())
        .collect::<Vec<_>>();
    identities.sort_by(|left, right| identity_signature(left).cmp(&identity_signature(right)));
    identities.dedup_by(|left, right| identity_signature(left) == identity_signature(right));
    identities
}

fn repo_key(
    identity: &RepositoryIdentity,
    repo_counts: &BTreeMap<String, usize>,
    owner_repo_counts: &BTreeMap<String, usize>,
) -> String {
    if repo_counts.get(&identity.repo).copied().unwrap_or_default() == 1 {
        return identity.repo.clone();
    }

    let owner_repo = format!("{}/{}", identity.owner, identity.repo);
    if owner_repo_counts
        .get(&owner_repo)
        .copied()
        .unwrap_or_default()
        == 1
    {
        owner_repo
    } else {
        format!("{}:{owner_repo}", identity.source)
    }
}

fn identity_signature(identity: &RepositoryIdentity) -> (&str, &str, &str, &str) {
    (
        &identity.source,
        &identity.host,
        &identity.owner,
        &identity.repo,
    )
}

fn work_location_sort_key(location: &WorkLocation) -> (String, u8, String) {
    let (repo, workspace) = location
        .key
        .split_once('@')
        .map_or((location.key.as_str(), None), |(repo, workspace)| {
            (repo, Some(workspace))
        });
    (
        repo.to_owned(),
        u8::from(workspace.is_some()),
        location.key.clone(),
    )
}

fn max_work_location_depth(layout: &LayoutConfig) -> usize {
    layout
        .rules
        .iter()
        .map(|rule| rule.path.as_deref().unwrap_or(&layout.default.path))
        .chain(std::iter::once(layout.default.path.as_str()))
        .map(path_component_count)
        .max()
        .unwrap_or(0)
        + 2
}

fn path_component_count(path: &str) -> usize {
    Path::new(path)
        .components()
        .filter(|component| matches!(component, std::path::Component::Normal(_)))
        .count()
}
