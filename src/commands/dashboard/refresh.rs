use super::*;
use crate::repository::PrActionOnSuccess;
use std::{sync::mpsc, thread, time::Instant};

/// Selects GitHub-backed loading or a local-only rebuild of the review inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::commands) enum DashboardRefreshKind {
    Live,
    Local,
}

/// Separates remote/cache I/O from the refresh whose completion owns the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::commands) enum DashboardLoadKind {
    /// Fetch remote facts and update the cache without writing visibility actions.
    Live,
    /// Read the cache without fetching or recording automatic visibility changes.
    Local,
    /// Rebuild from cache and clean up dismissals after remote fetching has finished.
    AfterLive,
}

impl DashboardRefreshKind {
    pub(super) fn for_action(policy: PrActionOnSuccess) -> Option<Self> {
        match policy {
            PrActionOnSuccess::None => None,
            PrActionOnSuccess::Refresh => Some(Self::Live),
            PrActionOnSuccess::RefreshLocal => Some(Self::Local),
        }
    }
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
            PrActionOnSuccess::None => {}
        }
    }

    pub(super) fn request_live(&mut self) {
        self.next_live_at = None;
    }

    /// Local reloads can proceed while a live refresh is already in flight.
    pub(super) fn next(
        &mut self,
        now: DateTime<Local>,
        live_busy: bool,
    ) -> Option<DashboardRefreshKind> {
        if std::mem::take(&mut self.local_requested) {
            Some(DashboardRefreshKind::Local)
        } else if !live_busy && dashboard_wait_duration(now, self.next_live_at).is_none() {
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
    pub(super) started: Instant,
    pub(super) timed_out: bool,
}

impl DashboardRefresh {
    pub(super) fn start(loader: DashboardFrameLoader, kind: DashboardLoadKind) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(loader(kind));
        });
        Self {
            receiver,
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
