use super::*;
use crate::{
    commands::pr_actions::{PrActionFailure, PrActionSet, RunningPrAction},
    domain::PreparedPrAction,
    repository::PrActionOnSuccess,
};

/// One running action and a failure notice that survives refreshes until acknowledged.
#[derive(Default)]
pub(super) struct DashboardActions {
    running: Option<RunningPrAction>,
    on_success: PrActionOnSuccess,
    completed: Option<PrActionOnSuccess>,
    pub(super) failure: Option<PrActionFailure>,
}

impl DashboardActions {
    pub(super) fn start(
        &mut self,
        action: PreparedPrAction,
        environment: &RuntimeEnvironment,
        action_set: PrActionSet,
    ) {
        if self.running.is_some() || self.failure.is_some() {
            return;
        }
        self.on_success = action.on_success;
        match RunningPrAction::start(action, environment, action_set) {
            Ok(running) => self.running = Some(running),
            Err(error) => self.complete(Err(error)),
        }
    }

    pub(super) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Returns the reload policy once after success; failures leave the current rows intact.
    pub(super) fn poll(&mut self) -> Option<PrActionOnSuccess> {
        self.poll_running();
        self.completed.take()
    }

    pub(super) fn cancel(&mut self) -> bool {
        if self.poll_running() {
            return true;
        }
        let Some(running) = self.running.take() else {
            return false;
        };
        self.complete(Err(running.cancel()));
        true
    }

    /// Consumes keys while the failure popup is visible; dismissing never runs another action.
    pub(super) fn handle_failure_key(&mut self, key: KeyEvent) -> bool {
        if self.failure.is_none() {
            return false;
        }
        if key.kind == KeyEventKind::Press
            && matches!(
                key.code,
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q' | 'Q')
            )
        {
            self.failure = None;
        }
        true
    }

    fn poll_running(&mut self) -> bool {
        let Some(result) = self.running.as_mut().and_then(RunningPrAction::poll) else {
            return false;
        };
        self.complete(result);
        true
    }

    fn complete(&mut self, result: Result<(), PrActionFailure>) {
        self.running = None;
        self.completed = result.is_ok().then_some(self.on_success);
        self.failure = result.err();
    }
}

#[cfg(test)]
#[path = "tests/actions.rs"]
mod tests;
