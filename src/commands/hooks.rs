use super::*;

const HOOK_LOG_FILE: &str = "jx-hooks.log";

/// Runs configured lifecycle hooks, logs each attempt, and aborts the caller when a hook cannot complete.
pub(super) fn run_repo_hooks(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    repository: &GitHubRepository,
    workspace: &WorkspaceEntry,
    event: RepoHookEvent,
    hooks: Vec<RepoHook>,
) -> Result<Vec<RepoHookEffect>, CommandError> {
    let log = HookLog::from_environment(environment);
    let mut effects = Vec::new();
    for hook in hooks {
        effects.push(run_repo_hook(
            services, repository, workspace, event, &hook, &log,
        )?);
    }
    Ok(effects)
}

/// Appends operator-visible hook effects in the same `Event[...]` style as stack publishing.
pub(super) fn append_repo_hook_effects(
    output: &mut String,
    effects: &[RepoHookEffect],
    color: bool,
) {
    for effect in effects {
        let line = format!("Event[{}]: ran `{}`", effect.hook, effect.command.join(" "));
        output.push_str(&style_log_line(&line, color));
        output.push('\n');
    }
}

fn run_repo_hook(
    services: &dyn CommandServices,
    repository: &GitHubRepository,
    workspace: &WorkspaceEntry,
    event: RepoHookEvent,
    hook: &RepoHook,
    log: &HookLog,
) -> Result<RepoHookEffect, CommandError> {
    let log_context = HookLogContext {
        repository,
        workspace,
        event,
        hook,
    };
    log.append(&log_context, "start", None, None);
    let output = services
        .run_hook_command(&workspace.root, hook)
        .map_err(|source| {
            let message = source.to_string();
            log.append(&log_context, "error", Some(message.as_str()), None);
            CommandError::Hook {
                message: format!(
                    "hook `{}` could not start for {}: `{}`: {source}",
                    hook.id,
                    event.label(),
                    hook.command.join(" ")
                ),
            }
        })?;

    if output.success {
        log.append(&log_context, "success", Some(output.status.as_str()), None);
        return Ok(RepoHookEffect {
            hook: hook.id.clone(),
            command: hook.command.clone(),
        });
    }

    let message = failed_hook_message(hook, event, &output);
    log.append(
        &log_context,
        "error",
        Some(output.status.as_str()),
        non_empty_output(&output.output),
    );
    Err(CommandError::Hook { message })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RepoHookEffect {
    pub(super) hook: String,
    pub(super) command: Vec<String>,
}

fn failed_hook_message(
    hook: &RepoHook,
    event: RepoHookEvent,
    output: &HookCommandOutput,
) -> String {
    let output_text = output.output.trim();
    let mut message = format!(
        "hook `{}` failed for {}{}",
        hook.id,
        event.label(),
        if output_text.is_empty() {
            format!(" ({})", output.status)
        } else {
            String::new()
        }
    );
    append_indented_output(&mut message, output_text);
    message
}

fn non_empty_output(output: &str) -> Option<&str> {
    let output = output.trim();
    (!output.is_empty()).then_some(output)
}

struct HookLogContext<'a> {
    repository: &'a GitHubRepository,
    workspace: &'a WorkspaceEntry,
    event: RepoHookEvent,
    hook: &'a RepoHook,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HookLog {
    path: Option<PathBuf>,
}

impl HookLog {
    fn from_environment(environment: &RuntimeEnvironment) -> Self {
        Self {
            path: hook_log_path(environment),
        }
    }

    fn append(
        &self,
        context: &HookLogContext<'_>,
        status: &str,
        message: Option<&str>,
        output: Option<&str>,
    ) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        else {
            return;
        };
        let mut record = serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "status": status,
            "hook": context.hook.id.as_str(),
            "event": context.event.label(),
            "repo": context.repository.slug(),
            "workspace": context.workspace.name.as_str(),
            "cwd": context.workspace.root.display().to_string(),
            "command": &context.hook.command,
        });
        if let Some(message) = message {
            record["message"] = serde_json::Value::String(message.to_owned());
        }
        if let Some(output) = output {
            record["output"] = serde_json::Value::String(output.to_owned());
        }
        if serde_json::to_writer(&mut file, &record).is_ok() {
            let _ = writeln!(file);
        }
    }
}

fn hook_log_path(environment: &RuntimeEnvironment) -> Option<PathBuf> {
    if let Some(path) = environment
        .variable("JX_HOOK_LOG")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        if matches!(path, "off" | "false" | "0") {
            return None;
        }
        return Some(PathBuf::from(path));
    }

    environment
        .variable("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .home_dir()
                .map(|home| home.join(".local").join("state"))
        })
        .map(|root| root.join("jx").join(HOOK_LOG_FILE))
}

fn append_indented_output(message: &mut String, output: &str) {
    if output.is_empty() {
        return;
    }

    message.push_str("\n\n");
    for (index, line) in output.lines().enumerate() {
        if index > 0 {
            message.push('\n');
        }
        message.push_str("  ");
        message.push_str(line);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HookCommandOutput {
    pub(super) success: bool,
    pub(super) status: String,
    pub(super) output: String,
}

impl HookCommandOutput {
    #[cfg(test)]
    pub(super) fn success() -> Self {
        Self {
            success: true,
            status: "exit code 0".to_owned(),
            output: String::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn failure(status: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            success: false,
            status: status.into(),
            output: output.into(),
        }
    }

    pub(super) fn from_process_status(status: std::process::ExitStatus, output: String) -> Self {
        Self {
            success: status.success(),
            status: process_exit_status_summary(status),
            output,
        }
    }
}

fn process_exit_status_summary(status: std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| status.to_string())
}
