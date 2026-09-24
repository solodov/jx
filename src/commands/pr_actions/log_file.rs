use super::*;
use std::fs::{self, File, OpenOptions};

/// Opens the shared append-only log for action output and dashboard refresh failures.
pub(in crate::commands) fn open_action_log(
    environment: &RuntimeEnvironment,
) -> Result<(File, PathBuf), String> {
    let path = action_log_path(environment)
        .ok_or_else(|| "HOME or XDG_STATE_HOME must be set to store dashboard logs".to_owned())?;
    let file = (|| {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)
    })()
    .map_err(|error| format!("cannot open log {}: {error}", path.display()))?;
    Ok((file, path))
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
