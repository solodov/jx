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

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoveredWorkLocation {
    identity: RepositoryIdentity,
    workspace: Option<String>,
    root: PathBuf,
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
