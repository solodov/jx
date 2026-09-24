use super::*;
use crate::domain::{prepare_pr_action, PrActionContext, PrActionUnavailable, PreparedPrAction};
use crate::repository::ResolvedPrAction;
mod execution;
mod log_file;
pub(super) use execution::{PrActionFailure, RunningPrAction};
pub(super) use log_file::open_action_log;

pub(super) struct AvailablePrAction {
    pub(super) definition: ResolvedPrAction,
    pub(super) prepared: Result<PreparedPrAction, PrActionUnavailable>,
}

/// Loads only the invoking dashboard's action set for the selected repository.
pub(super) fn load_pr_actions(
    mut context: PrActionContext,
    environment: &RuntimeEnvironment,
    action_set: PrActionSet,
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
    let actions = match action_set {
        PrActionSet::Review => &config.review_actions,
        PrActionSet::StackStatus => &config.stack_status_actions,
    };
    Ok(actions
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

/// Independent configuration namespace chosen by the invoking command, not by PR ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrActionSet {
    Review,
    StackStatus,
}

#[cfg(test)]
#[path = "tests/pr_action_execution.rs"]
mod tests;
