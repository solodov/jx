use super::*;
use crate::{
    commands::pr_actions::{PrActionFailure, PrActionSet, RunningPrAction},
    domain::{PrActionKey, PreparedPrAction},
    repository::PrActionOnSuccess,
};
use std::time::Instant;

/// Runs one action and delivers its outcome once; notifications never block the next action.
#[derive(Default)]
pub(super) struct DashboardActions {
    running: Option<RunningPrAction>,
    info: Option<DashboardActionInfo>,
    on_success: PrActionOnSuccess,
    completed: Option<DashboardActionCompletion>,
}

impl DashboardActions {
    /// Freezes the action identity for feedback even if the selected PR later moves or disappears.
    pub(super) fn start(
        &mut self,
        action: PreparedPrAction,
        environment: &RuntimeEnvironment,
        action_set: PrActionSet,
    ) {
        if self.running.is_some() {
            return;
        }
        self.info = Some(DashboardActionInfo {
            id: action.id.clone(),
            title: action.title.clone(),
            target: action.target.clone(),
            started: Instant::now(),
        });
        self.on_success = action.on_success;
        match RunningPrAction::start(action, environment, action_set) {
            Ok(running) => self.running = Some(running),
            Err(error) => self.complete(DashboardActionOutcome::Failed(error)),
        }
    }

    pub(super) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub(super) fn running_info(&self) -> Option<&DashboardActionInfo> {
        self.info.as_ref()
    }

    /// Delivers completion after polling, including failures that happened during spawn.
    pub(super) fn poll(&mut self) -> Option<DashboardActionCompletion> {
        self.poll_running();
        self.completed.take()
    }

    /// Cancels active work without losing a success that raced with the cancel key.
    pub(super) fn cancel(&mut self) -> bool {
        if self.poll_running() {
            return true;
        }
        let Some(running) = self.running.take() else {
            return false;
        };
        self.complete(DashboardActionOutcome::Cancelled(running.cancel()));
        true
    }

    fn poll_running(&mut self) -> bool {
        let Some(result) = self.running.as_mut().and_then(RunningPrAction::poll) else {
            return false;
        };
        self.complete(match result {
            Ok(()) => DashboardActionOutcome::Succeeded(self.on_success),
            Err(error) => DashboardActionOutcome::Failed(error),
        });
        true
    }

    fn complete(&mut self, outcome: DashboardActionOutcome) {
        self.running = None;
        self.completed = self
            .info
            .take()
            .map(|action| DashboardActionCompletion { action, outcome });
    }
}

#[derive(Debug, Clone)]
pub(super) struct DashboardActionInfo {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) target: PrActionKey,
    pub(super) started: Instant,
}

pub(super) struct DashboardActionCompletion {
    pub(super) action: DashboardActionInfo,
    pub(super) outcome: DashboardActionOutcome,
}

pub(super) enum DashboardActionOutcome {
    Succeeded(PrActionOnSuccess),
    Failed(PrActionFailure),
    Cancelled(PrActionFailure),
}

#[cfg(test)]
#[path = "tests/actions.rs"]
mod tests;
