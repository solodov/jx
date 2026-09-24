use super::*;
use crate::commands::pr_actions::{open_action_log, PrActionFailure, PrActionSet};
use std::io::Write;

/// Records standalone refresh failures before giving the status line a log link.
pub(super) fn record_refresh_failure(
    message: String,
    environment: &RuntimeEnvironment,
    action_set: PrActionSet,
) -> PrActionFailure {
    let logged = (|| {
        let (mut file, path) = open_action_log(environment)?;
        let record = serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "pid": std::process::id(),
            "dashboard": match action_set {
                PrActionSet::Review => "review",
                PrActionSet::StackStatus => "stack-status",
            },
            "cwd": environment.current_dir().display().to_string(),
            "status": "refresh_failed",
            "message": message,
        });
        writeln!(file, "\n[jx-dashboard] {record}")
            .map_err(|error| format!("cannot write log {}: {error}", path.display()))?;
        Ok::<_, String>(path)
    })();
    match logged {
        Ok(path) => PrActionFailure {
            message,
            log_path: Some(path),
        },
        Err(error) => PrActionFailure {
            message: format!("{message}; logging failed: {error}"),
            log_path: None,
        },
    }
}

#[cfg(test)]
#[path = "tests/diagnostics.rs"]
mod tests;
