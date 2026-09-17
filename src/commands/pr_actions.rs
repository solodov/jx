use super::*;
use crate::domain::{prepare_pr_action, PrActionContext, PrActionUnavailable, PreparedPrAction};
use crate::repository::ResolvedPrAction;
use std::process::ExitStatus;

pub(super) struct AvailablePrAction {
    pub(super) definition: ResolvedPrAction,
    pub(super) prepared: Result<PreparedPrAction, PrActionUnavailable>,
}

/// Loads policy for the selected repository, never the caller's unrelated checkout.
pub(super) fn load_pr_actions(
    mut context: PrActionContext,
    environment: &RuntimeEnvironment,
) -> Result<Vec<AvailablePrAction>, CommandError> {
    let config = if let Some(root) = &context.repository_root {
        WorkflowConfig::discover_for_uninitialized(&environment.with_current_dir(root))?
    } else {
        WorkflowConfig::discover_global(environment)?
    };
    context.repository_root = context
        .repository_root
        .filter(|root| root.join(".jj").is_dir() || root.join(".git").exists());
    context.local_commit_id = None;
    context.local_change_id = None;
    if let (Some(root), Some(head)) = (&context.repository_root, &context.head_oid) {
        if let Ok(workspace) = JjWorkspace::load(root) {
            if let Some(revision) = workspace.pr_action_revision(head) {
                context.local_commit_id = Some(revision.commit_id);
                context.local_change_id = revision.change_id;
            }
        }
    }
    Ok(config
        .actions
        .for_repository(&context.repository)
        .into_iter()
        .map(|definition| {
            let prepared = prepare_pr_action(&definition, &context, environment.current_dir());
            AvailablePrAction {
                definition,
                prepared,
            }
        })
        .collect())
}

/// Runs precisely the prepared argv in the foreground, inheriting the normal terminal streams.
/// The dashboard owns suspension, confirmation, acknowledgement, and refresh around this boundary.
pub(super) fn execute_pr_action(action: &PreparedPrAction) -> io::Result<ExitStatus> {
    let (program, arguments) = action
        .command
        .split_first()
        .ok_or_else(|| io::Error::other("action has no executable"))?;
    ProcessCommand::new(program)
        .args(arguments)
        .current_dir(&action.cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
}

#[cfg(test)]
#[path = "tests/pr_action_execution.rs"]
mod tests;
