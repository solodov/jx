use super::*;

/// Loads transient local aliases only for human stack-status output.
pub(super) fn load_status_aliases(
    services: &dyn CommandServices,
    workspace_root: &Path,
    report: &PullRequestStackStatusReport,
    format: StackStatusFormat,
) -> Result<BTreeMap<String, String>, JjError> {
    if format != StackStatusFormat::Human || report.snapshot.nodes.is_empty() {
        return Ok(BTreeMap::new());
    }
    let bookmarks = report
        .snapshot
        .nodes
        .iter()
        .map(|node| node.branch.clone())
        .collect::<Vec<_>>();
    services.local_bookmark_change_aliases(workspace_root, &bookmarks)
}

/// Attaches aliases using each repository's own jj workspace and preserves scoped failures.
pub(super) fn load_global_status_aliases(
    services: &dyn CommandServices,
    entries: &mut [GlobalStackStatusEntry],
    format: StackStatusFormat,
) {
    for entry in entries {
        let Ok(report) = &entry.result else {
            continue;
        };
        match load_status_aliases(services, &entry.root, report, format) {
            Ok(aliases) => entry.local_aliases = aliases,
            Err(error) => entry.result = Err(error.to_string()),
        }
    }
}
