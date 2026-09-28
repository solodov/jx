use super::*;
use crate::{domain::PreparedPrAction, repository::PrActionOnSuccess};

/// Coordinates one command, one remote fetch, and one short cached/UI reload independently.
pub(super) struct DashboardActivity<'a> {
    pub(super) actions: DashboardActions,
    pub(super) status: DashboardStatus,
    pub(super) view: DashboardView,
    remote: Option<DashboardRefresh>,
    remote_ready: Option<Result<DashboardFrameSnapshot, String>>,
    reload: Option<(DashboardRefreshKind, DashboardRefresh)>,
    pending_kind: Option<DashboardRefreshKind>,
    schedule: DashboardRefreshSchedule,
    loader: DashboardFrameLoader,
    environment: &'a RuntimeEnvironment,
    action_set: pr_actions::PrActionSet,
    refresh_seconds: u64,
}

impl<'a> DashboardActivity<'a> {
    pub(super) fn new(
        loader: DashboardFrameLoader,
        environment: &'a RuntimeEnvironment,
        action_set: pr_actions::PrActionSet,
        refresh_seconds: u64,
    ) -> Self {
        Self {
            actions: DashboardActions::default(),
            status: DashboardStatus::default(),
            view: DashboardView::default(),
            remote: None,
            remote_ready: None,
            reload: None,
            pending_kind: None,
            schedule: DashboardRefreshSchedule::default(),
            loader,
            environment,
            action_set,
            refresh_seconds,
        }
    }

    /// Polls independent workers without dropping results on action completion or cancellation.
    pub(super) fn tick(&mut self, frozen: bool, size: DashboardTerminalSize) {
        if let Some(completion) = self.actions.poll() {
            if let Some(policy) =
                self.status
                    .action_completed(completion, self.environment, Instant::now())
            {
                self.schedule.after_action(policy);
            }
        }
        self.status.tick(Instant::now());
        self.poll_remote();
        self.poll_reload();
        if !frozen
            && !self.actions.is_running()
            && self.reload.is_none()
            && self.view.pending.is_none()
        {
            let next = self.schedule.next(Local::now(), self.live_busy());
            if next == Some(DashboardRefreshKind::Local) {
                self.refresh_started(DashboardRefreshKind::Local);
                self.start_reload(DashboardRefreshKind::Local);
            } else if let Some(result) = self.remote_ready.take() {
                if self.action_set == pr_actions::PrActionSet::Review && result.is_ok() {
                    // Remote loading updates the cache. Build rows from current local actions,
                    // not the snapshot that may have been prepared before a dismissal.
                    self.start_reload(DashboardRefreshKind::Live);
                } else {
                    self.queue_result(DashboardRefreshKind::Live, result);
                }
            } else if next == Some(DashboardRefreshKind::Live) {
                self.refresh_started(DashboardRefreshKind::Live);
                self.remote = Some(DashboardRefresh::start(
                    Arc::clone(&self.loader),
                    DashboardLoadKind::Live,
                ));
            }
        }
        self.update_view(frozen, size);
    }

    /// Only remote-refresh actions must wait for an in-flight live refresh.
    pub(super) fn can_start(&self, policy: PrActionOnSuccess) -> bool {
        self.actions.can_start(policy)
            && self.reload.is_none()
            && (policy != PrActionOnSuccess::Refresh || !self.live_busy())
    }

    /// Applies any queued frame before attributing subsequent work to the selected invocation.
    pub(super) fn start_action(&mut self, action: PreparedPrAction, size: DashboardTerminalSize) {
        if !self.can_start(action.on_success) {
            return;
        }
        self.update_view(false, size);
        self.actions
            .start(action, self.environment, self.action_set);
    }

    pub(super) fn request_refresh(&mut self) {
        self.status.request_refresh();
        if !self.live_busy() {
            self.schedule.request_live();
        }
    }

    pub(super) fn is_busy(&self) -> bool {
        self.actions.is_busy()
            || self.has_workers()
            || self.view.pending.is_some()
            || self.remote_ready.is_some()
    }

    pub(super) fn has_workers(&self) -> bool {
        self.remote.is_some() || self.reload.is_some()
    }

    fn live_busy(&self) -> bool {
        self.remote.is_some()
            || self.remote_ready.is_some()
            || self
                .reload
                .as_ref()
                .is_some_and(|(kind, _)| *kind == DashboardRefreshKind::Live)
            || self.pending_kind == Some(DashboardRefreshKind::Live)
    }

    fn refresh_started(&mut self, kind: DashboardRefreshKind) {
        self.actions.refresh_started(kind);
        self.status
            .refresh_started(kind, self.view.frame.is_none(), Instant::now());
    }

    fn start_reload(&mut self, origin: DashboardRefreshKind) {
        self.reload = Some((
            origin,
            DashboardRefresh::start(
                Arc::clone(&self.loader),
                match origin {
                    DashboardRefreshKind::Local => DashboardLoadKind::Local,
                    DashboardRefreshKind::Live => DashboardLoadKind::AfterLive,
                },
            ),
        ));
    }

    fn poll_remote(&mut self) {
        let Some(loading) = &mut self.remote else {
            return;
        };
        if let Some(result) = loading.poll() {
            self.remote_ready = Some(result);
            self.remote = None;
            self.schedule.loaded(
                DashboardRefreshKind::Live,
                Local::now(),
                self.refresh_seconds,
            );
        } else if !loading.timed_out && dashboard_refresh_timed_out(loading.started.elapsed()) {
            loading.timed_out = true;
            self.report_timeout(DashboardRefreshKind::Live);
        }
    }

    fn poll_reload(&mut self) {
        let Some((kind, loading)) = &mut self.reload else {
            return;
        };
        let kind = *kind;
        if let Some(result) = loading.poll() {
            self.reload = None;
            self.queue_result(kind, result);
        } else if !loading.timed_out && dashboard_refresh_timed_out(loading.started.elapsed()) {
            loading.timed_out = true;
            self.report_timeout(kind);
        }
    }

    fn report_timeout(&mut self, kind: DashboardRefreshKind) {
        let failure = self.actions.refresh_timed_out(
            kind,
            dashboard_refresh_timeout_error(),
            self.environment,
            self.action_set,
        );
        self.status
            .refresh_timed_out(kind, failure, self.environment, Instant::now());
        // Keep each worker until it finishes; timeouts must not spawn duplicate cache writers.
    }

    fn queue_result(
        &mut self,
        kind: DashboardRefreshKind,
        result: Result<DashboardFrameSnapshot, String>,
    ) {
        debug_assert!(self.view.pending.is_none());
        self.pending_kind = Some(kind);
        self.view.pending = Some(result);
    }

    fn update_view(&mut self, frozen: bool, size: DashboardTerminalSize) {
        if let Some(update) = self.view.update(frozen, size) {
            match update {
                DashboardViewUpdate::Loaded(result) => {
                    let kind = self
                        .pending_kind
                        .take()
                        .expect("queued refresh has an origin");
                    self.status.refreshed(
                        kind,
                        self.actions
                            .refreshed(kind, result, self.environment, self.action_set),
                        self.environment,
                        Instant::now(),
                    );
                }
                DashboardViewUpdate::Reflowed(result) => {
                    self.status.reflowed(result, Instant::now())
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/activity.rs"]
mod tests;
