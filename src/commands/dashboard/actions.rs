use super::diagnostics::record_refresh_failure;
use super::*;
use crate::{
    commands::pr_actions::{PrActionFailure, PrActionSet, RunningPrAction},
    domain::{PrActionKey, PreparedPrAction},
    repository::PrActionOnSuccess,
};
use std::time::Instant;

/// Runs one command at a time and retains its log until the configured list update finishes.
#[derive(Default)]
pub(super) struct DashboardActions {
    running: Option<RunningPrAction>,
    refreshing: BTreeMap<DashboardRefreshKind, RunningPrAction>,
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
        if !self.can_start(action.on_success) {
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

    /// Only the subprocess blocks starting its own follow-up refresh.
    pub(super) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Keeps executable restarts from interrupting an action's command or follow-up reload.
    pub(super) fn is_busy(&self) -> bool {
        self.running.is_some() || !self.refreshing.is_empty()
    }

    /// Local actions may overlap a live follow-up, but commands and local reloads stay serial.
    pub(super) fn can_start(&self, policy: PrActionOnSuccess) -> bool {
        !self.is_running()
            && !self.refreshing.contains_key(&DashboardRefreshKind::Local)
            && (policy != PrActionOnSuccess::Refresh || self.refreshing.is_empty())
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

    /// Attaches reload details to the completed command's original log.
    pub(super) fn refresh_started(&mut self, kind: DashboardRefreshKind) {
        if let Some(action) = self.refreshing.get_mut(&kind) {
            action.refresh_started(match kind {
                DashboardRefreshKind::Live => "live",
                DashboardRefreshKind::Local => "local",
            });
        }
    }

    /// Finishes an action's reload or records a standalone refresh failure.
    pub(super) fn refreshed(
        &mut self,
        kind: DashboardRefreshKind,
        result: Result<(), String>,
        environment: &RuntimeEnvironment,
        action_set: PrActionSet,
    ) -> Result<(), PrActionFailure> {
        match self.refreshing.remove(&kind) {
            Some(mut action) => action.refreshed(result),
            None => {
                result.map_err(|message| record_refresh_failure(message, environment, action_set))
            }
        }
    }

    /// Records a timeout, retaining any action log for a late refresh result.
    pub(super) fn refresh_timed_out(
        &mut self,
        kind: DashboardRefreshKind,
        message: String,
        environment: &RuntimeEnvironment,
        action_set: PrActionSet,
    ) -> PrActionFailure {
        match self.refreshing.get_mut(&kind) {
            Some(action) => action.refresh_timed_out(&message),
            None => record_refresh_failure(message, environment, action_set),
        }
    }

    fn poll_running(&mut self) -> bool {
        let Some(result) = self.running.as_mut().and_then(RunningPrAction::poll) else {
            return false;
        };
        let outcome = match result {
            Ok(()) => {
                if let Some(kind) = DashboardRefreshKind::for_action(self.on_success) {
                    self.refreshing.insert(kind, self.running.take().unwrap());
                }
                DashboardActionOutcome::Succeeded(self.on_success)
            }
            Err(error) => DashboardActionOutcome::Failed(error),
        };
        self.complete(outcome);
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
