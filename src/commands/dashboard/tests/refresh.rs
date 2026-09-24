use super::*;

#[test]
fn local_reload_preserves_the_live_deadline_and_manual_refresh_still_fetches() {
    let now = Local::now();
    let mut schedule = DashboardRefreshSchedule::default();
    assert_eq!(schedule.next(now), Some(DashboardRefreshKind::Live));
    schedule.loaded(DashboardRefreshKind::Live, now, 300);
    let deadline = schedule.next_live_at.unwrap();
    assert_eq!(schedule.next(now), None);

    schedule.after_action(PrActionOnSuccess::RefreshLocal);
    assert_eq!(schedule.next(now), Some(DashboardRefreshKind::Local));
    schedule.loaded(DashboardRefreshKind::Local, now, 300);
    assert_eq!(schedule.next_live_at, Some(deadline));
    assert_eq!(schedule.next(now), None);
    assert_eq!(schedule.next(deadline), Some(DashboardRefreshKind::Live));

    schedule.request_live();
    assert_eq!(schedule.next(now), Some(DashboardRefreshKind::Live));
}

#[test]
fn local_reload_precedes_an_overdue_live_refresh_without_cancelling_it() {
    let now = Local::now();
    let mut schedule = DashboardRefreshSchedule::default();
    schedule.after_action(PrActionOnSuccess::RefreshLocal);
    assert_eq!(schedule.next(now), Some(DashboardRefreshKind::Local));
    schedule.loaded(DashboardRefreshKind::Local, now, 300);
    assert_eq!(schedule.next(now), Some(DashboardRefreshKind::Live));
}

#[test]
fn default_actions_leave_the_periodic_deadline_and_pending_requests_unchanged() {
    let now = Local::now();
    let mut schedule = DashboardRefreshSchedule::default();
    schedule.loaded(DashboardRefreshKind::Live, now, 300);
    let deadline = schedule.next_live_at.unwrap();
    schedule.after_action(PrActionOnSuccess::default());
    assert_eq!(schedule.next_live_at, Some(deadline));
    assert_eq!(schedule.next(now), None);
    assert_eq!(schedule.next(deadline), Some(DashboardRefreshKind::Live));

    schedule.after_action(PrActionOnSuccess::RefreshLocal);
    schedule.after_action(PrActionOnSuccess::None);
    assert_eq!(schedule.next(now), Some(DashboardRefreshKind::Local));
    assert_eq!(schedule.next_live_at, Some(deadline));

    schedule.request_live();
    schedule.after_action(PrActionOnSuccess::None);
    assert_eq!(schedule.next(now), Some(DashboardRefreshKind::Live));
}

#[test]
fn explicit_refresh_actions_request_a_live_reload() {
    let now = Local::now();
    let mut schedule = DashboardRefreshSchedule::default();
    schedule.loaded(DashboardRefreshKind::Live, now, 300);
    schedule.after_action(PrActionOnSuccess::Refresh);
    assert_eq!(schedule.next(now), Some(DashboardRefreshKind::Live));
}

#[test]
fn worker_passes_the_requested_load_kind_without_falling_back_on_failure() {
    let (sender, receiver) = mpsc::channel();
    let loader: DashboardFrameLoader = Arc::new(move |kind| {
        sender.send(kind).unwrap();
        Err("local storage unavailable".to_owned())
    });
    let refresh = DashboardRefresh::start(loader, DashboardRefreshKind::Local);
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        DashboardRefreshKind::Local
    );
    assert_eq!(
        refresh
            .receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .err()
            .as_deref(),
        Some("local storage unavailable")
    );
    assert!(receiver.try_recv().is_err());
}
