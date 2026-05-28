use super::*;

/// Runs configured check-only commands before lifecycle mutations and rejects file changes.
pub(super) fn run_repo_checks(
    context: &RepositoryContext,
    services: &dyn CommandServices,
    before: RepoCheckTrigger,
    changed_files: &[String],
) -> Result<(), CommandError> {
    let checks = context
        .config
        .repo
        .checks_for(&context.origin.github, before, changed_files);

    for check in checks {
        run_repo_check(context, services, before, &check)?;
    }

    Ok(())
}

/// Runs sync checks against files sync may publish plus the current work sync may rewrite.
pub(super) fn run_sync_repo_checks(
    context: &RepositoryContext,
    services: &dyn CommandServices,
    operation_changed_files: impl FnOnce() -> Result<Vec<String>, JjError>,
) -> Result<(), CommandError> {
    if !context
        .config
        .repo
        .has_checks_for_trigger(&context.origin.github, RepoCheckTrigger::Sync)
    {
        return Ok(());
    }

    let mut changed_files = operation_changed_files()?;
    changed_files.extend(services.workspace_facts(context, None)?.changed_files);
    changed_files.sort();
    changed_files.dedup();

    run_repo_checks(context, services, RepoCheckTrigger::Sync, &changed_files)
}

fn run_repo_check(
    context: &RepositoryContext,
    services: &dyn CommandServices,
    before: RepoCheckTrigger,
    check: &RepoCheckConfig,
) -> Result<(), CommandError> {
    let snapshot_before = services.working_copy_snapshot(context)?;
    let output = services
        .run_check_command(context, check)
        .map_err(|source| CommandError::Check {
            message: format!(
                "check `{}` could not start before {}: `{}`: {source}",
                check.id,
                trigger_display_name(before),
                check.command.join(" ")
            ),
        })?;
    let snapshot_after = services.working_copy_snapshot(context)?;
    let mutated = snapshot_before != snapshot_after;

    if !output.success {
        return Err(CommandError::Check {
            message: failed_check_message(check, before, &output, mutated),
        });
    }
    if mutated {
        return Err(CommandError::Check {
            message: mutated_check_message(check, before),
        });
    }

    Ok(())
}

fn failed_check_message(
    check: &RepoCheckConfig,
    before: RepoCheckTrigger,
    output: &CheckCommandOutput,
    mutated: bool,
) -> String {
    let output_text = output.output.trim();
    let mut message = format!(
        "check `{}` failed before {}{}",
        check.id,
        trigger_display_name(before),
        if output_text.is_empty() {
            format!(" ({})", output.status)
        } else {
            String::new()
        }
    );
    if mutated {
        message.push_str(" and modified the working copy");
    }
    append_indented_output(&mut message, output_text);
    if mutated {
        message.push_str(
            "\n\nChecks must leave jj state unchanged. Review or revert the changes before retrying.",
        );
    }
    message
}

fn mutated_check_message(check: &RepoCheckConfig, before: RepoCheckTrigger) -> String {
    format!(
        "check `{}` modified the working copy before {}\n\nChecks must leave jj state unchanged. Review or revert the changes before retrying.",
        check.id,
        trigger_display_name(before)
    )
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

fn trigger_display_name(trigger: RepoCheckTrigger) -> &'static str {
    match trigger {
        RepoCheckTrigger::PullRequest => "pull request",
        RepoCheckTrigger::Push => "push",
        RepoCheckTrigger::Sync => "sync",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CheckCommandOutput {
    pub(super) success: bool,
    pub(super) status: String,
    pub(super) output: String,
}

impl CheckCommandOutput {
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
