use super::*;
use crate::repository::PrActionOnSuccess;
use std::{
    fs::{self, File, OpenOptions},
    process::Child,
    time::Instant,
};

/// A quiet command and its log, retained through a configured reload after the child exits.
/// Dropping a live command cancels and reaps it.
pub(in crate::commands) struct RunningPrAction {
    child: Option<Child>,
    title: String,
    on_success: PrActionOnSuccess,
    log: ActionLog,
    refresh_log_error: Option<PrActionFailure>,
}

impl RunningPrAction {
    /// Opens the log before spawning; child streams never inherit the dashboard's terminal.
    pub(in crate::commands) fn start(
        action: PreparedPrAction,
        environment: &RuntimeEnvironment,
        action_set: PrActionSet,
    ) -> Result<Self, PrActionFailure> {
        let mut log = ActionLog::open(environment, &action, action_set)?;
        log.record("start", None)
            .map_err(|error| log.unavailable("Action not started", error))?;
        let child = (|| {
            let (program, arguments) = action
                .command
                .split_first()
                .ok_or_else(|| io::Error::other("action has no executable"))?;
            let mut command = ProcessCommand::new(program);
            command
                .args(arguments)
                .current_dir(&action.cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::from(log.file.try_clone()?))
                .stderr(Stdio::from(log.file.try_clone()?));
            // Isolate descendants so cancelling an action cannot signal the dashboard.
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            command.spawn()
        })();
        match child {
            Ok(child) => Ok(Self {
                child: Some(child),
                title: action.title,
                on_success: action.on_success,
                log,
                refresh_log_error: None,
            }),
            Err(error) => Err(log.failure(
                &action.title,
                "failed",
                &format!("could not start: {error}"),
            )),
        }
    }

    /// Returns a result once finished, leaving the UI free to handle keys and resizes while running.
    pub(in crate::commands) fn poll(&mut self) -> Option<Result<(), PrActionFailure>> {
        match self.child.as_mut()?.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) => {
                self.child = None;
                let message = status.to_string();
                Some(if status.success() {
                    let status = if self.on_success == PrActionOnSuccess::None {
                        "success"
                    } else {
                        "command_success"
                    };
                    self.log.record(status, Some(&message)).map_err(|error| {
                        self.log
                            .unavailable("Action completed, but logging failed", error)
                    })
                } else {
                    Err(self.log.failure(&self.title, "failed", &message))
                })
            }
            Err(error) => {
                self.stop();
                Some(Err(self.log.failure(
                    &self.title,
                    "failed",
                    &format!("could not wait: {error}"),
                )))
            }
        }
    }

    /// Begins logging the follow-up load; a write failure is reported with its eventual result.
    pub(in crate::commands) fn refresh_started(&mut self, kind: &str) {
        self.log.refresh_started = Some(Instant::now());
        self.log.metadata["refresh_kind"] = kind.into();
        self.refresh_log_error = self.log.record("refresh_start", None).err().map(|error| {
            self.log
                .unavailable("Could not record refresh start", error)
        });
    }

    /// Records the rendered-list outcome and only then marks the overall action successful.
    pub(in crate::commands) fn refreshed(
        &mut self,
        result: Result<(), String>,
    ) -> Result<(), PrActionFailure> {
        let result = match (result, self.refresh_log_error.take()) {
            (Ok(()), Some(error)) => Err(format!("List refreshed, but {}", error.message)),
            (Err(message), Some(error)) => Err(format!("{message}; {}", error.message)),
            (result, None) => result,
        };
        let result = match result {
            Ok(()) => self
                .log
                .record("refresh_success", None)
                .and_then(|()| self.log.record("success", None))
                .map_err(|error| {
                    self.log
                        .unavailable("List refreshed, but logging failed", error)
                }),
            Err(error) => Err(self.log.failure(&self.title, "refresh_failed", &error)),
        };
        self.log.refresh_started = None;
        result
    }

    /// Records a slow reload without discarding its log or preventing a later result.
    pub(in crate::commands) fn refresh_timed_out(&mut self, message: &str) -> PrActionFailure {
        self.log.failure(&self.title, "refresh_timeout", message)
    }

    /// Cancels an in-flight action without leaving a child running behind the dashboard.
    pub(in crate::commands) fn cancel(mut self) -> PrActionFailure {
        self.stop();
        self.log
            .failure(&self.title, "cancelled", "cancelled by operator")
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            // SAFETY: this unreaped child owns the private process group created at spawn.
            unsafe {
                libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for RunningPrAction {
    fn drop(&mut self) {
        if self.child.is_some() && self.poll().is_none() {
            self.stop();
            let _ = self.log.record("cancelled", Some("dashboard closed"));
        } else if self.log.refresh_started.is_some() {
            let _ = self.log.record(
                "refresh_abandoned",
                Some("dashboard closed before the list update"),
            );
        }
    }
}

/// Only logged failures point to a log; logging failures explain the problem directly.
#[derive(Debug)]
pub(in crate::commands) struct PrActionFailure {
    pub(in crate::commands) message: String,
    pub(in crate::commands) log_path: Option<PathBuf>,
}

impl From<String> for PrActionFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            log_path: None,
        }
    }
}

struct ActionLog {
    file: File,
    path: PathBuf,
    metadata: serde_json::Value,
    started: Instant,
    refresh_started: Option<Instant>,
}

impl ActionLog {
    fn open(
        environment: &RuntimeEnvironment,
        action: &PreparedPrAction,
        action_set: PrActionSet,
    ) -> Result<Self, PrActionFailure> {
        let started = Instant::now();
        let path = action_log_path(environment).ok_or_else(|| PrActionFailure {
            message: "Action not started: HOME or XDG_STATE_HOME must be set to store action logs"
                .to_owned(),
            log_path: None,
        })?;
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
        .map_err(|error| PrActionFailure {
            message: format!(
                "Action not started: cannot open log {}: {error}",
                path.display()
            ),
            log_path: None,
        })?;
        let metadata = serde_json::json!({
            "pid": std::process::id(),
            "action": action.id,
            "title": action.title,
            "dashboard": match action_set { PrActionSet::Review => "review", PrActionSet::StackStatus => "stack-status" },
            "repo": action.target.repository,
            "pr": action.target.number,
            "command": action.command,
            "cwd": action.cwd.display().to_string(),
            "source": action.source.path.display().to_string(),
            "on_success": action.on_success.as_str(),
        });
        Ok(Self {
            file,
            path,
            metadata,
            started,
            refresh_started: None,
        })
    }

    /// Appends an event with total elapsed time and, while reloading, refresh elapsed time.
    fn record(&mut self, status: &str, message: Option<&str>) -> io::Result<()> {
        self.metadata["at"] = chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            .into();
        self.metadata["status"] = status.into();
        self.metadata["message"] = message.into();
        let now = Instant::now();
        self.metadata["elapsed_ms"] =
            serde_json::json!(now.duration_since(self.started).as_millis());
        if let Some(started) = self.refresh_started {
            self.metadata["refresh_elapsed_ms"] =
                serde_json::json!(now.duration_since(started).as_millis());
        }
        // Delimit metadata from raw child output, including output without a final newline.
        let record = format!("\n[jx-action] {}\n", self.metadata);
        self.file.write_all(record.as_bytes())
    }

    fn failure(&mut self, title: &str, status: &str, detail: &str) -> PrActionFailure {
        match self.record(status, Some(detail)) {
            Ok(()) => PrActionFailure {
                message: format!("{title} {status}"),
                log_path: Some(self.path.clone()),
            },
            Err(error) => self.unavailable(
                &format!("{title} {status}: {detail}; logging failed"),
                error,
            ),
        }
    }

    fn unavailable(&self, message: &str, error: io::Error) -> PrActionFailure {
        PrActionFailure {
            message: format!("{message}: {}: {error}", self.path.display()),
            log_path: None,
        }
    }
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

#[cfg(test)]
#[path = "tests/execution.rs"]
mod tests;
