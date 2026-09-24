use super::*;

/// Loads an all-repository dashboard only when every repository succeeded.
pub(super) fn load(
    request: &StackStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    span: &mut PerfSpan,
) -> Result<DashboardFrameSnapshot, CommandError> {
    let loaded = load_global_stack_status_view(request, environment, services, progress, span)?;
    snapshot(loaded, environment.current_dir())
}

/// Rejects repository failures before the dashboard can replace its last good snapshot.
fn snapshot(
    loaded: LoadedGlobalStackStatusView,
    current_dir: &Path,
) -> Result<DashboardFrameSnapshot, CommandError> {
    let failures = loaded
        .entries
        .iter()
        .filter_map(|entry| {
            entry.result.as_ref().err().map(|error| {
                let repository = entry
                    .repository
                    .as_ref()
                    .map(GitHubRepository::slug)
                    .or_else(|| entry.key.clone())
                    .unwrap_or_else(|| "repository".to_owned());
                format!("{repository} ({}): {error}", entry.display_root)
            })
        })
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        return Err(CommandError::Check {
            message: format!(
                "{} repository refreshes failed:\n{}",
                failures.len(),
                failures.join("\n")
            ),
        });
    }
    let current_dir = current_dir.to_path_buf();
    Ok(DashboardFrameSnapshot::new(move |options| {
        Ok(render_global_stack_status(
            &loaded.entries,
            loaded.total_repositories,
            &current_dir,
            options.color,
            options.terminal_width,
            PullRequestTableLayout::FitTerminal,
            &loaded.display_names,
        ))
    }))
}

#[cfg(test)]
#[path = "tests/dashboard_snapshot.rs"]
mod tests;
