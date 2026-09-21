use super::*;
use crate::repository::PrActionOnSuccess;
use std::{sync::mpsc, thread, time::Instant};

/// Selects GitHub-backed loading or a local-only rebuild of the review inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::commands) enum DashboardRefreshKind {
    Live,
    Local,
}

/// Local action reloads take priority without postponing the next scheduled live refresh.
#[derive(Default)]
pub(super) struct DashboardRefreshSchedule {
    next_live_at: Option<DateTime<Local>>,
    local_requested: bool,
}

impl DashboardRefreshSchedule {
    pub(super) fn after_action(&mut self, policy: PrActionOnSuccess) {
        match policy {
            PrActionOnSuccess::Refresh => self.request_live(),
            PrActionOnSuccess::RefreshLocal => self.local_requested = true,
        }
    }

    pub(super) fn request_live(&mut self) {
        self.next_live_at = None;
    }

    /// Called only when the dashboard can start a load without racing an action.
    pub(super) fn next(&mut self, now: DateTime<Local>) -> Option<DashboardRefreshKind> {
        if std::mem::take(&mut self.local_requested) {
            Some(DashboardRefreshKind::Local)
        } else if dashboard_wait_duration(now, self.next_live_at).is_none() {
            Some(DashboardRefreshKind::Live)
        } else {
            None
        }
    }

    pub(super) fn loaded(
        &mut self,
        kind: DashboardRefreshKind,
        now: DateTime<Local>,
        refresh_seconds: u64,
    ) {
        if kind == DashboardRefreshKind::Live {
            self.next_live_at = next_dashboard_refresh_time(now, refresh_seconds);
        }
    }
}

pub(super) struct DashboardRefresh {
    receiver: mpsc::Receiver<Result<DashboardFrameSnapshot, String>>,
    pub(super) kind: DashboardRefreshKind,
    pub(super) started: Instant,
    pub(super) timed_out: bool,
}

impl DashboardRefresh {
    pub(super) fn start(loader: DashboardFrameLoader, kind: DashboardRefreshKind) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(loader(kind));
        });
        Self {
            receiver,
            kind,
            started: Instant::now(),
            timed_out: false,
        }
    }

    pub(super) fn poll(&self) -> Option<Result<DashboardFrameSnapshot, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "dashboard refresh worker stopped unexpectedly".to_owned(),
            )),
        }
    }
}

#[cfg(test)]
#[path = "tests/refresh.rs"]
mod tests;
