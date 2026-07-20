use super::*;
use globset::{Glob, GlobMatcher};
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
    identity: RepositoryIdentity,
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
    pub(super) project: Option<String>,
    pub(super) parent: Option<WorkspaceParentMetadata>,
    pub(super) shared_paths: PlannedSharedWorkspacePaths,
}

/// Render-ready workspace facts enriched with local jx metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkListEntry {
    pub(super) workspace: WorkspaceEntry,
    pub(super) project: Option<String>,
}

/// Render-ready global work location facts enriched with local jx metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkLocationListEntry {
    pub(super) location: WorkLocation,
    pub(super) project: Option<String>,
}

/// Current workspace details exposed as a stable integration boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkInfo {
    pub(super) workspace: WorkspaceEntry,
    pub(super) repository_root: PathBuf,
    pub(super) identity: RepositoryIdentity,
    pub(super) metadata: WorkspaceMetadata,
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

impl WorkRepository {
    pub(super) fn github_repository(&self) -> GitHubRepository {
        self.identity.github_repository()
    }

    pub(super) fn provider_slug(&self) -> String {
        format!(
            "{}/{}/{}",
            self.identity.host, self.identity.owner, self.identity.repo
        )
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentNavigationRepository {
    primary_root: PathBuf,
    workspace_collection_root: PathBuf,
}

/// Builds the full work-add plan before crossing jj or filesystem mutation boundaries.
pub(super) fn plan_work_add(
    request: &WorkAddRequest,
    context: &LocalRepositoryContext,
    environment: &RuntimeEnvironment,
    parent_workspace: Option<&WorkspaceEntry>,
) -> Result<WorkAddPlan, CommandError> {
    let task_id = domain::normalize_task_id(request.task_id.as_deref())?;
    let (project, parent) = work_add_metadata_context(request, context, parent_workspace)?;
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
        project,
        parent,
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
    if plan.task_id.is_none() && plan.project.is_none() && plan.parent.is_none() {
        return Ok(());
    }

    write_workspace_metadata(
        &plan.destination,
        &WorkspaceMetadata {
            task_id: plan.task_id.clone(),
            project: plan.project.clone(),
            parent: plan.parent.clone(),
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

fn work_add_metadata_context(
    request: &WorkAddRequest,
    context: &LocalRepositoryContext,
    parent_workspace: Option<&WorkspaceEntry>,
) -> Result<(Option<String>, Option<WorkspaceParentMetadata>), CommandError> {
    let project = normalize_project_key(request.project.as_deref())?;
    if !request.child {
        return Ok((project, None));
    }

    let Some(parent_workspace) = parent_workspace else {
        return Err(CommandError::Check {
            message: "Child workspaces require a current workspace".to_owned(),
        });
    };
    let parent_metadata = read_workspace_metadata(&context.workspace_root)?;
    let Some(parent_project) = parent_metadata.project.clone() else {
        return Err(CommandError::Check {
            message: "Child workspaces require current workspace project metadata".to_owned(),
        });
    };
    if project
        .as_deref()
        .is_some_and(|project| project != parent_project.as_str())
    {
        return Err(CommandError::Check {
            message: format!(
                "Child workspace project must match current workspace project `{parent_project}`"
            ),
        });
    }

    let parent = WorkspaceParentMetadata {
        workspace_name: parent_workspace.name.clone(),
        task_id: parent_metadata.task_id,
        project: Some(parent_project.clone()),
    };
    Ok((Some(parent_project), Some(parent)))
}

fn normalize_project_key(project: Option<&str>) -> Result<Option<String>, CommandError> {
    let Some(project) = project.map(str::trim).filter(|project| !project.is_empty()) else {
        return Ok(None);
    };
    if !is_project_key(project) {
        return Err(CommandError::Check {
            message: format!(
                "Project key `{project}` may contain only ASCII letters, numbers, `.`, `_`, or `-`, and cannot start or end with `.`"
            ),
        });
    }

    Ok(Some(project.to_owned()))
}

fn is_project_key(project: &str) -> bool {
    !project.starts_with('.')
        && !project.ends_with('.')
        && project
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(super) fn work_list_entries(
    workspaces: Vec<WorkspaceEntry>,
) -> Result<Vec<WorkListEntry>, RepositoryError> {
    workspaces
        .into_iter()
        .map(|workspace| {
            let project = read_workspace_metadata(&workspace.root)?.project;
            Ok(WorkListEntry { workspace, project })
        })
        .collect()
}

pub(super) fn work_location_list_entries(
    locations: Vec<WorkLocation>,
) -> Result<Vec<WorkLocationListEntry>, RepositoryError> {
    locations
        .into_iter()
        .map(|location| {
            let project = read_workspace_metadata(&location.root)?.project;
            Ok(WorkLocationListEntry { location, project })
        })
        .collect()
}

pub(super) fn current_work_info(
    context: &LocalRepositoryContext,
    workspace: WorkspaceEntry,
    environment: &RuntimeEnvironment,
) -> Result<WorkInfo, CommandError> {
    let identity = workspace_identity(context, environment)?;
    let metadata = read_workspace_metadata(&context.workspace_root)?;
    Ok(WorkInfo {
        workspace: WorkspaceEntry {
            root: context.workspace_root.clone(),
            is_current: true,
            ..workspace
        },
        repository_root: context.repository_root.clone(),
        identity,
        metadata,
    })
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

fn global_discovered_work_locations(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
) -> Result<Vec<DiscoveredWorkLocation>, RepositoryError> {
    discovered_work_locations(config, environment, WorkDiscoveryScope::All)
}

fn global_discovered_work_repositories(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
) -> Result<Vec<DiscoveredWorkLocation>, RepositoryError> {
    Ok(
        discovered_work_locations(config, environment, WorkDiscoveryScope::PrimaryOnly)?
            .into_iter()
            .filter(|location| location.workspace.is_none())
            .collect(),
    )
}

fn discovered_work_locations(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
    scope: WorkDiscoveryScope,
) -> Result<Vec<DiscoveredWorkLocation>, RepositoryError> {
    let mut workspace_roots = Vec::new();
    let max_depth = max_work_location_depth(&config.layout);
    for root in config.layout.configured_roots(environment)? {
        collect_jj_workspace_roots(
            &root,
            max_depth,
            scope.skipped_child_name(&config.layout),
            &mut workspace_roots,
        );
    }
    workspace_roots.sort();
    workspace_roots.dedup();

    let mut discovered = Vec::new();
    for root in workspace_roots {
        if let Some(location) = discovered_work_location(&config.layout, &root, environment)? {
            discovered.push(location);
        }
    }

    Ok(discovered)
}

#[derive(Debug, Clone, Copy)]
enum WorkDiscoveryScope {
    All,
    PrimaryOnly,
}

impl WorkDiscoveryScope {
    fn skipped_child_name(self, layout: &LayoutConfig) -> Option<&str> {
        match self {
            Self::All => None,
            Self::PrimaryOnly => Some(&layout.workspace_dir),
        }
    }
}

/// Builds the global work-location index used by shell completion and path resolution.
pub(super) fn global_work_locations(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
) -> Result<Vec<WorkLocation>, RepositoryError> {
    Ok(assign_work_location_keys(global_discovered_work_locations(
        config,
        environment,
    )?))
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

pub(super) fn filter_navigation_work_locations_by_query(
    locations: &[WorkLocation],
    query: &str,
) -> Vec<WorkLocation> {
    locations
        .iter()
        .filter(|location| navigation_match_rank(&location.key, query).is_some())
        .cloned()
        .collect()
}

pub(super) fn navigation_work_locations_from_global(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
    current_workspaces: &[WorkspaceEntry],
    global: Vec<WorkLocation>,
) -> Result<Vec<WorkLocation>, RepositoryError> {
    let global = navigation_global_work_locations(config, environment, global)?;
    let mut locations = current_workspace_name_locations(current_workspaces);

    let Some(current_repository) =
        current_navigation_repository(config, environment, current_workspaces)?
    else {
        locations.extend(global);
        return Ok(deduplicate_work_locations_by_key(locations));
    };

    let (current_global, other_global) =
        partition_navigation_global_locations(&global, &current_repository);
    locations.extend(current_repository_workspace_aliases(
        &current_global,
        &current_repository,
    ));
    locations.push(WorkLocation {
        key: "default".to_owned(),
        root: current_repository.primary_root.clone(),
    });
    locations.push(WorkLocation {
        key: "trunk".to_owned(),
        root: current_repository.primary_root.clone(),
    });
    locations.push(WorkLocation {
        key: "root".to_owned(),
        root: current_repository.primary_root.clone(),
    });
    locations.extend(current_global);
    locations.extend(other_global);

    Ok(deduplicate_work_locations_by_key(locations))
}

fn navigation_global_work_locations(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
    global: Vec<WorkLocation>,
) -> Result<Vec<WorkLocation>, RepositoryError> {
    global
        .into_iter()
        .map(|location| navigation_global_work_location(config, environment, location))
        .collect()
}

fn navigation_global_work_location(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
    location: WorkLocation,
) -> Result<WorkLocation, RepositoryError> {
    let identity = config
        .layout
        .identity_for_workspace_root(&location.root, environment)?;
    if !config.shell.uses_repository_slug(&identity) {
        return Ok(location);
    }

    let mut key = config.shell.repository_label(&identity);
    if let Some((_, workspace)) = location.key.split_once('@') {
        key.push('@');
        key.push_str(workspace);
    }

    Ok(WorkLocation {
        key,
        root: location.root,
    })
}

fn current_repository_workspace_aliases(
    global: &[WorkLocation],
    current_repository: &CurrentNavigationRepository,
) -> Vec<WorkLocation> {
    global
        .iter()
        .filter(|location| {
            location
                .root
                .starts_with(&current_repository.workspace_collection_root)
        })
        .filter_map(|location| {
            location
                .key
                .split_once('@')
                .map(|(_, workspace)| WorkLocation {
                    key: workspace.to_owned(),
                    root: location.root.clone(),
                })
        })
        .collect()
}

/// Builds the global primary-checkout index used by cross-repository commands.
pub(super) fn global_work_repositories(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
) -> Result<Vec<WorkRepository>, RepositoryError> {
    Ok(assign_work_repository_keys(
        global_discovered_work_repositories(config, environment)?,
    ))
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

pub(super) fn resolve_workspace_entry_by_fragment(
    workspaces: &[WorkspaceEntry],
    query: &str,
) -> Result<WorkspaceEntry, RepositoryError> {
    let mut matches = workspaces
        .iter()
        .filter_map(|workspace| {
            navigation_match_rank(&workspace.name, query).map(|rank| (workspace, rank))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| (left.1, &left.0.name).cmp(&(right.1, &right.0.name)));

    let Some(best_rank) = matches.first().map(|(_, rank)| *rank) else {
        return Err(RepositoryError::WorkspaceNameNotFound {
            name: query.to_owned(),
        });
    };
    let best_matches = matches
        .into_iter()
        .filter(|(_, rank)| *rank == best_rank)
        .map(|(workspace, _)| workspace)
        .collect::<Vec<_>>();

    match best_matches.as_slice() {
        [workspace] => Ok((*workspace).clone()),
        _ => {
            let mut names = best_matches
                .into_iter()
                .map(|workspace| workspace.name.clone())
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            Err(RepositoryError::WorkspaceNameAmbiguous {
                name: query.to_owned(),
                matches: names,
            })
        }
    }
}

pub(super) fn deletable_workspace_entries(
    context: &LocalRepositoryContext,
    identity: &RepositoryIdentity,
    workspaces: &[WorkspaceEntry],
    environment: &RuntimeEnvironment,
) -> Result<Vec<WorkspaceEntry>, RepositoryError> {
    let primary = context
        .config
        .layout
        .project_destination(identity, environment)?;
    let mut deletable = Vec::new();
    for workspace in workspaces {
        if workspace.root == primary {
            continue;
        }
        let managed =
            context
                .config
                .layout
                .workspace_destination(identity, &workspace.name, environment)?;
        if workspace.root == managed {
            deletable.push(workspace.clone());
        }
    }

    Ok(deletable)
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

/// Resolves exact current-repository navigation targets without scanning global layout roots.
pub(super) fn resolve_local_navigation_work_location(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
    current_workspaces: &[WorkspaceEntry],
    query: &str,
) -> Result<Option<PathBuf>, RepositoryError> {
    if let Some(path) = resolve_navigation_path(query, environment) {
        return Ok(Some(path));
    }

    let Some(components) = navigation_query_components(query) else {
        return Ok(None);
    };
    if components.len() != 1 {
        return Ok(None);
    }
    let key = &components[0];

    if let Some(workspace) = current_workspaces
        .iter()
        .find(|workspace| workspace.name == *key)
    {
        return Ok(Some(workspace.root.clone()));
    }

    let Some(current_repository) =
        current_navigation_repository(config, environment, current_workspaces)?
    else {
        return Ok(None);
    };
    if matches!(key.as_str(), "default" | "trunk" | "root") {
        return Ok(Some(current_repository.primary_root));
    }

    if validate_workspace_name(key).is_err() {
        return Ok(None);
    }
    let workspace_root = current_repository.workspace_collection_root.join(key);
    Ok(workspace_root.is_dir().then_some(workspace_root))
}

/// Resolves a shell navigation query by explicit path first, then unique key and child-directory fragments.
pub(super) fn resolve_navigation_work_location(
    locations: &[WorkLocation],
    query: &str,
    environment: &RuntimeEnvironment,
) -> Result<PathBuf, RepositoryError> {
    if let Some(path) = resolve_navigation_path(query, environment) {
        return Ok(path);
    }

    let Some(components) = navigation_query_components(query) else {
        return Err(RepositoryError::WorkLocationNotFound {
            key: query.to_owned(),
        });
    };

    let mut candidates = Vec::new();
    for split in 1..=components.len() {
        let location_query = components[..split].join("/");
        let path_queries = &components[split..];
        for (location, location_rank) in matching_navigation_locations(locations, &location_query) {
            if let Some((path, path_ranks)) =
                resolve_navigation_subpath(&location.root, path_queries)
            {
                candidates.push(NavigationResolution {
                    path,
                    score: NavigationScore {
                        location_rank,
                        remaining_segments: path_queries.len(),
                        path_ranks,
                    },
                });
            }
        }
    }

    let best_score = candidates
        .iter()
        .map(|candidate| &candidate.score)
        .min()
        .cloned();
    let Some(best_score) = best_score else {
        return Err(RepositoryError::WorkLocationNotFound {
            key: query.to_owned(),
        });
    };
    let matches = candidates
        .into_iter()
        .filter(|candidate| candidate.score == best_score)
        .collect::<Vec<_>>();
    let mut paths = matches
        .iter()
        .map(|candidate| candidate.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    match paths.as_slice() {
        [path] => Ok(path.clone()),
        _ => Err(RepositoryError::WorkLocationAmbiguous {
            key: query.to_owned(),
            paths,
        }),
    }
}

fn resolve_navigation_path(query: &str, environment: &RuntimeEnvironment) -> Option<PathBuf> {
    let path = Path::new(query);
    if !is_explicit_navigation_path(query, path) {
        return None;
    }

    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        environment.current_dir().join(path)
    };
    if !path.is_dir() {
        return None;
    }

    fs::canonicalize(&path).ok().or(Some(path))
}

fn is_explicit_navigation_path(query: &str, path: &Path) -> bool {
    path.is_absolute()
        || matches!(query, "." | "..")
        || query.starts_with("./")
        || query.starts_with("../")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationResolution {
    path: PathBuf,
    score: NavigationScore,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NavigationScore {
    location_rank: NavigationMatchRank,
    remaining_segments: usize,
    path_ranks: Vec<NavigationMatchRank>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NavigationMatchRank {
    Exact,
    Prefix,
    Contains,
}

fn navigation_query_components(query: &str) -> Option<Vec<String>> {
    let components = Path::new(query)
        .components()
        .map(|component| match component {
            std::path::Component::Normal(value) => value.to_str().map(str::to_owned),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (!components.is_empty()).then_some(components)
}

fn matching_navigation_locations<'a>(
    locations: &'a [WorkLocation],
    query: &str,
) -> Vec<(&'a WorkLocation, NavigationMatchRank)> {
    locations
        .iter()
        .filter_map(|location| {
            navigation_match_rank(&location.key, query).map(|rank| (location, rank))
        })
        .collect()
}

fn resolve_navigation_subpath(
    root: &Path,
    queries: &[String],
) -> Option<(PathBuf, Vec<NavigationMatchRank>)> {
    let mut path = root.to_path_buf();
    let mut ranks = Vec::new();

    for query in queries {
        let (child, rank) = resolve_navigation_child(&path, query)?;
        path = child;
        ranks.push(rank);
    }

    Some((path, ranks))
}

fn resolve_navigation_child(parent: &Path, query: &str) -> Option<(PathBuf, NavigationMatchRank)> {
    let mut matches = fs::read_dir(parent)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            if !entry.path().is_dir() {
                return None;
            }
            navigation_match_rank(&name, query).map(|rank| (entry.path(), rank))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| (left.1, &left.0).cmp(&(right.1, &right.0)));

    let best_rank = matches.first()?.1;
    let best_matches = matches
        .into_iter()
        .filter(|(_, rank)| *rank == best_rank)
        .collect::<Vec<_>>();
    match best_matches.as_slice() {
        [(path, rank)] => Some((path.clone(), *rank)),
        _ => None,
    }
}

fn navigation_match_rank(candidate: &str, query: &str) -> Option<NavigationMatchRank> {
    if candidate == query {
        Some(NavigationMatchRank::Exact)
    } else if candidate.starts_with(query) {
        Some(NavigationMatchRank::Prefix)
    } else if candidate.contains(query) {
        Some(NavigationMatchRank::Contains)
    } else {
        None
    }
}

fn current_workspace_name_locations(current_workspaces: &[WorkspaceEntry]) -> Vec<WorkLocation> {
    current_workspaces
        .iter()
        .map(|workspace| WorkLocation {
            key: workspace.name.clone(),
            root: workspace.root.clone(),
        })
        .collect()
}

fn current_navigation_repository(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
    current_workspaces: &[WorkspaceEntry],
) -> Result<Option<CurrentNavigationRepository>, RepositoryError> {
    let Some(current) = current_workspaces
        .iter()
        .find(|workspace| workspace.is_current)
    else {
        return Ok(None);
    };

    let identity = match config
        .layout
        .identity_for_workspace_root(&current.root, environment)
    {
        Ok(identity) => identity,
        Err(RepositoryError::LayoutPathNotMatched { .. })
        | Err(RepositoryError::AmbiguousLayoutPath { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };

    Ok(Some(CurrentNavigationRepository {
        primary_root: config.layout.project_destination(&identity, environment)?,
        workspace_collection_root: config
            .layout
            .workspace_collection_root(&identity, environment)?,
    }))
}

fn partition_navigation_global_locations(
    global: &[WorkLocation],
    current_repository: &CurrentNavigationRepository,
) -> (Vec<WorkLocation>, Vec<WorkLocation>) {
    global
        .iter()
        .cloned()
        .partition(|location| is_current_repository_location(location, current_repository))
}

fn is_current_repository_location(
    location: &WorkLocation,
    current_repository: &CurrentNavigationRepository,
) -> bool {
    location.root == current_repository.primary_root
        || location
            .root
            .starts_with(&current_repository.workspace_collection_root)
}

fn deduplicate_work_locations_by_key(locations: Vec<WorkLocation>) -> Vec<WorkLocation> {
    let mut seen = BTreeSet::new();
    locations
        .into_iter()
        .filter(|location| seen.insert(location.key.clone()))
        .collect()
}

fn matching_work_repositories<'a>(
    repositories: &'a [WorkRepository],
    pattern: &str,
) -> Result<Vec<&'a WorkRepository>, RepositoryError> {
    let pattern = pattern.trim();
    if contains_glob_meta(pattern) {
        let matchers = repository_filter_matchers(pattern)?;
        Ok(repositories
            .iter()
            .filter(|repository| repository_matches_glob(repository, &matchers))
            .collect())
    } else {
        Ok(repositories
            .iter()
            .filter(|repository| {
                repository_filter_labels(repository).any(|label| label.contains(pattern))
            })
            .collect())
    }
}

fn repository_matches_glob(repository: &WorkRepository, matchers: &[GlobMatcher]) -> bool {
    repository_filter_labels(repository)
        .any(|label| matchers.iter().any(|matcher| matcher.is_match(&label)))
}

fn repository_filter_matchers(pattern: &str) -> Result<Vec<GlobMatcher>, RepositoryError> {
    let mut patterns = vec![pattern.to_owned()];
    if !pattern.starts_with("**/") {
        patterns.push(format!("**/{pattern}"));
    }

    patterns
        .into_iter()
        .map(|candidate| {
            Glob::new(&candidate)
                .map_err(|source| RepositoryError::InvalidRepositoryFilter {
                    pattern: pattern.to_owned(),
                    message: source.to_string(),
                })
                .map(|glob| glob.compile_matcher())
        })
        .collect()
}

fn repository_filter_labels(repository: &WorkRepository) -> impl Iterator<Item = String> + '_ {
    [
        repository.key.clone(),
        format!(
            "{}/{}/{}",
            repository.identity.host, repository.identity.owner, repository.identity.repo
        ),
    ]
    .into_iter()
}

fn contains_glob_meta(pattern: &str) -> bool {
    pattern
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']'))
}

fn collect_jj_workspace_roots(
    root: &Path,
    max_depth: usize,
    skipped_child_name: Option<&str>,
    roots: &mut Vec<PathBuf>,
) {
    collect_jj_workspace_roots_at_depth(root, 0, max_depth, skipped_child_name, roots);
}

fn collect_jj_workspace_roots_at_depth(
    path: &Path,
    depth: usize,
    max_depth: usize,
    skipped_child_name: Option<&str>,
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
        if !file_type.is_dir() || entry.file_name() == ".jj" {
            continue;
        }
        if skipped_child_name.is_some_and(|name| entry.file_name() == name) {
            continue;
        }
        children.push(entry.path());
    }
    children.sort();

    for child in children {
        collect_jj_workspace_roots_at_depth(
            &child,
            depth + 1,
            max_depth,
            skipped_child_name,
            roots,
        );
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
    let (repo_counts, owner_repo_counts) = repo_key_counts(&identities);

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

fn assign_work_repository_keys(locations: Vec<DiscoveredWorkLocation>) -> Vec<WorkRepository> {
    let identities = unique_identities(&locations);
    let (repo_counts, owner_repo_counts) = repo_key_counts(&identities);
    let mut keyed = locations
        .into_iter()
        .filter(|location| location.workspace.is_none())
        .map(|location| WorkRepository {
            key: repo_key(&location.identity, &repo_counts, &owner_repo_counts),
            root: location.root,
            identity: location.identity,
        })
        .collect::<Vec<_>>();

    keyed.sort_by_key(work_repository_sort_key);
    keyed.dedup();
    keyed
}

fn repo_key_counts(
    identities: &[RepositoryIdentity],
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
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
    (repo_counts, owner_repo_counts)
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

fn work_repository_sort_key(repository: &WorkRepository) -> String {
    repository.key.clone()
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
