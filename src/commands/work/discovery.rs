use super::*;
use crate::repository::LayoutDiscoveryPath;

/// Discovers only paths allowed by layout templates, then applies identity and placement rules.
pub(super) fn discovered_work_locations(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
    scope: WorkDiscoveryScope,
) -> Result<Vec<DiscoveredWorkLocation>, RepositoryError> {
    let mut span = PerfLog::from_environment(environment).start(
        "work.discover_locations",
        [perf_attr("scope", scope.label())],
    );
    let result = discover_work_locations_traced(config, environment, scope, &mut span);
    if let Err(error) = &result {
        span.record_error(error);
    }
    result
}

#[derive(Debug, Clone, Copy)]
pub(super) enum WorkDiscoveryScope {
    All,
    PrimaryOnly,
}

impl WorkDiscoveryScope {
    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::PrimaryOnly => "primary",
        }
    }
}

fn discover_work_locations_traced(
    config: &WorkflowConfig,
    environment: &RuntimeEnvironment,
    scope: WorkDiscoveryScope,
    span: &mut PerfSpan,
) -> Result<Vec<DiscoveredWorkLocation>, RepositoryError> {
    let paths = span.measure("plan_layout_discovery", Vec::new(), || {
        config.layout.discovery_paths(environment)
    })?;
    let mut paths_by_root = BTreeMap::<_, BTreeSet<_>>::new();
    for LayoutDiscoveryPath { root, components } in paths {
        let patterns = paths_by_root.entry(root).or_default();
        if matches!(scope, WorkDiscoveryScope::All) {
            let mut managed = vec![Some(config.layout.workspace_dir.clone())];
            managed.extend(components.clone());
            managed.push(None);
            patterns.insert(managed);
        }
        patterns.insert(components);
    }

    let mut workspace_roots = Vec::new();
    for (root, patterns) in paths_by_root {
        let timer = span.start_step(
            "scan_layout_root",
            [
                perf_attr("layout_root", root.display().to_string()),
                perf_attr("pattern_count", patterns.len()),
            ],
        );
        let mut stats = DiscoveryStats::default();
        let previous_count = workspace_roots.len();
        for components in patterns {
            collect_layout_workspace_roots(&root, &components, &mut workspace_roots, &mut stats);
        }
        span.finish_step(
            timer,
            [
                perf_attr("directory_count", stats.directory_count),
                perf_attr("read_dir_count", stats.read_dir_count),
                perf_attr("entry_count", stats.entry_count),
                perf_attr(
                    "workspace_root_count",
                    workspace_roots.len() - previous_count,
                ),
            ],
            None::<&RepositoryError>,
        );
    }
    workspace_roots.sort();
    workspace_roots.dedup();
    span.set([perf_attr("workspace_root_count", workspace_roots.len())]);

    span.measure("match_layout_locations", Vec::new(), || {
        workspace_roots
            .into_iter()
            .filter_map(|root| {
                discovered_work_location(&config.layout, &root, environment).transpose()
            })
            .collect()
    })
}

#[derive(Default)]
struct DiscoveryStats {
    directory_count: usize,
    read_dir_count: usize,
    entry_count: usize,
}

/// Follows literal components directly and enumerates only single-directory placeholders.
fn collect_layout_workspace_roots(
    path: &Path,
    components: &[Option<String>],
    roots: &mut Vec<PathBuf>,
    stats: &mut DiscoveryStats,
) {
    stats.directory_count += 1;
    if path.join(".jj").is_dir() {
        if components.is_empty() {
            roots.push(path.to_path_buf());
        }
        return;
    }
    let Some((component, remaining)) = components.split_first() else {
        return;
    };

    if let Some(name) = component {
        if name == ".jj" {
            return;
        }
        let child = path.join(name);
        // Match read_dir's file_type behavior: do not follow child-directory symlinks.
        if fs::symlink_metadata(&child).is_ok_and(|metadata| metadata.is_dir()) {
            collect_layout_workspace_roots(&child, remaining, roots, stats);
        }
        return;
    }

    stats.read_dir_count += 1;
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        stats.entry_count += 1;
        if entry.file_name() == ".jj" || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        collect_layout_workspace_roots(&entry.path(), remaining, roots, stats);
    }
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
