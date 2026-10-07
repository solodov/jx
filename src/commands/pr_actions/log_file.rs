use super::*;

/// Opens the shared rotating sink for action output and dashboard refresh failures.
pub(in crate::commands) fn open_action_log(
    environment: &RuntimeEnvironment,
) -> Result<LogSink, String> {
    let path = action_log_path(environment)
        .ok_or_else(|| "HOME or XDG_STATE_HOME must be set to store dashboard logs".to_owned())?;
    LogSink::open(path.clone())
        .map_err(|error| format!("cannot open log {}: {error}", path.display()))
}

fn action_log_path(environment: &RuntimeEnvironment) -> Option<PathBuf> {
    if let Some(path) = environment
        .variable("JX_ACTION_LOG")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        return Some(environment.current_dir().join(path));
    }
    environment
        .variable("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| environment.home_dir().map(|home| home.join(".local/state")))
        .map(|root| {
            environment
                .current_dir()
                .join(root)
                .join("jx/jx-actions.log")
        })
}
