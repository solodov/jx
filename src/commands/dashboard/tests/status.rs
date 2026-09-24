use super::*;
use crate::commands::dashboard::test_support::context;

#[test]
fn configured_reloads_keep_one_action_timer_until_the_list_is_rendered() {
    let now = Instant::now();
    for (policy, kind) in [
        (PrActionOnSuccess::RefreshLocal, DashboardRefreshKind::Local),
        (PrActionOnSuccess::Refresh, DashboardRefreshKind::Live),
    ] {
        let action = action(now);
        let mut status = DashboardStatus::default();
        let later = now + Duration::from_secs(10);
        let line = status.line(Some(&action), later, 120).unwrap();
        assert!(line.contains("Running dismiss/fix tests… 10s"));
        assert!(line.contains("Esc cancel"));
        assert!(!line.contains("owner/repo"));
        assert_eq!(
            status.action_completed(
                DashboardActionCompletion {
                    action,
                    outcome: DashboardActionOutcome::Succeeded(policy),
                },
                &environment(),
                later,
            ),
            Some(policy)
        );
        assert!(status
            .line(None, later, 120)
            .unwrap()
            .contains("Running dismiss/fix tests… 10s"));
        status.refresh_started(kind, false, later);
        status.clear_notice();
        let later = later + Duration::from_secs(10);
        status.tick(later);
        let line = status.line(None, later, 120).unwrap();
        assert!(line.contains("Running dismiss/fix tests… 20s"));
        assert!(!line.contains("Refreshing"));
        assert!(!line.contains("Esc cancel"));
        assert!(!line.contains("completed"));
        status.refreshed(Ok(()), &environment(), later);
        assert!(status
            .line(None, later, 120)
            .unwrap()
            .contains("\"dismiss/fix tests\" completed"));
        status.tick(later + NOTICE_DURATION);
        assert!(status.line(None, later, 120).is_none());
    }
}

#[test]
fn default_action_success_is_immediate_and_not_repeated_by_the_next_periodic_refresh() {
    let now = Instant::now();
    let mut status = DashboardStatus::default();
    assert_eq!(
        complete(
            &mut status,
            DashboardActionOutcome::Succeeded(PrActionOnSuccess::default()),
            now
        ),
        None
    );
    assert!(status.updating_action.is_none());
    assert!(status.refresh_started.is_none());
    let line = status.line(None, now, 120).unwrap();
    assert!(line.contains("\"dismiss/fix tests\" completed"));
    assert!(!line.contains("Refreshing"));

    let later = now + NOTICE_DURATION;
    status.tick(later);
    assert!(status.line(None, later, 120).is_none());
    status.refresh_started(DashboardRefreshKind::Live, false, later);
    assert!(status.line(None, later, 120).is_none());
    status.refreshed(Ok(()), &environment(), later);
    assert!(status.line(None, later, 120).is_none());
}

#[test]
fn no_refresh_success_clears_its_own_previous_error() {
    let now = Instant::now();
    let mut status = DashboardStatus::default();
    complete(
        &mut status,
        DashboardActionOutcome::Failed(PrActionFailure {
            message: "failed".to_owned(),
            log_path: None,
        }),
        now,
    );
    assert!(status.has_error());
    assert_eq!(
        complete(
            &mut status,
            DashboardActionOutcome::Succeeded(PrActionOnSuccess::None),
            now
        ),
        None
    );
    assert!(!status.has_error());
    assert!(status.line(None, now, 120).unwrap().contains("completed"));
}

#[test]
fn initial_and_manual_live_refreshes_show_elapsed_time() {
    let now = Instant::now();
    for initial in [true, false] {
        let mut status = DashboardStatus::default();
        if !initial {
            status.request_refresh();
        }
        status.refresh_started(DashboardRefreshKind::Live, initial, now);
        status.clear_notice();
        let line = status
            .line(None, now + Duration::from_secs(2), 120)
            .unwrap();
        assert!(line.contains("Refreshing pull requests… 2s"));
        assert!(!line.contains("dismiss/fix tests"));
    }
}

#[test]
fn manual_refresh_during_a_cached_reload_only_shows_feedback_when_live_loading_starts() {
    let now = Instant::now();
    let mut status = DashboardStatus::default();
    status.refresh_started(DashboardRefreshKind::Local, false, now);
    status.request_refresh();
    assert!(status.line(None, now, 120).is_none());
    status.refreshed(Ok(()), &environment(), now);
    status.refresh_started(DashboardRefreshKind::Live, false, now);
    assert!(status
        .line(None, now, 120)
        .unwrap()
        .contains("Refreshing pull requests… 0s"));
}

#[test]
fn logged_errors_show_the_actual_log_path_and_expire_after_ten_seconds() {
    let now = Instant::now();
    for (path, display) in [
        (
            "/home/operator/.local/state/jx/jx-actions.log",
            "~/.local/state/jx/jx-actions.log",
        ),
        ("/state/custom-actions.log", "/state/custom-actions.log"),
    ] {
        let mut status = DashboardStatus::default();
        complete(
            &mut status,
            DashboardActionOutcome::Failed(PrActionFailure {
                message: "action failed".to_owned(),
                log_path: Some(PathBuf::from(path)),
            }),
            now,
        );
        status.refreshed(Ok(()), &environment(), now);
        let line = status.line(None, now, 80).unwrap();
        assert!(line.contains(&format!("\"dismiss/fix tests\" failed, see {display}")));
        assert!(!line.contains("owner/repo"));
        assert!(!line.contains("Error:"));
        assert!(!line.contains("Esc"));
        assert!(!line.contains('?'));
        status.tick(now + ERROR_DURATION - Duration::from_secs(1));
        assert!(
            status.has_error(),
            "unrelated success does not hide an error"
        );
        status.tick(now + ERROR_DURATION);
        assert!(status.line(None, now, 80).is_none());
    }
}

#[test]
fn unlogged_errors_show_the_cause_and_interaction_clears_all_notices() {
    let now = Instant::now();
    let mut status = DashboardStatus::default();
    complete(
        &mut status,
        DashboardActionOutcome::Failed(PrActionFailure {
            message: "cannot open log: disk full".to_owned(),
            log_path: None,
        }),
        now,
    );
    let line = status.line(None, now, 120).unwrap();
    assert!(line.contains("cannot open log: disk full"));
    assert!(!line.contains("see "));
    complete(
        &mut status,
        DashboardActionOutcome::Cancelled(PrActionFailure {
            message: "cancelled".to_owned(),
            log_path: Some(PathBuf::from("/logs/actions.log")),
        }),
        now,
    );
    status.clear_notice();
    assert!(status.line(None, now, 120).is_none());
}

#[test]
fn cancellation_notice_expires_after_three_seconds() {
    let now = Instant::now();
    let mut status = DashboardStatus::default();
    complete(
        &mut status,
        DashboardActionOutcome::Cancelled(PrActionFailure {
            message: "cancelled".to_owned(),
            log_path: Some(PathBuf::from("/logs/actions.log")),
        }),
        now,
    );
    assert!(status
        .line(None, now, 120)
        .unwrap()
        .contains("\"dismiss/fix tests\" cancelled"));
    status.tick(now + NOTICE_DURATION);
    assert!(status.line(None, now, 120).is_none());
}

#[test]
fn refresh_timeout_expires_without_resuming_busy_feedback_and_can_recover() {
    let now = Instant::now();
    let mut status = DashboardStatus::default();
    complete(
        &mut status,
        DashboardActionOutcome::Succeeded(PrActionOnSuccess::RefreshLocal),
        now,
    );
    status.refresh_started(DashboardRefreshKind::Local, false, now);
    status.refresh_timed_out(
        dashboard_refresh_timeout_error().into(),
        &environment(),
        now,
    );
    let line = status.line(None, now, 200).unwrap();
    assert!(line.contains("\"dismiss/fix tests\" completed; refresh failed"));
    assert!(line.contains(&dashboard_refresh_timeout_error()));
    let later = now + ERROR_DURATION;
    status.tick(later);
    assert!(status.line(None, later, 160).is_none());
    status.refreshed(Ok(()), &environment(), later);
    assert!(status
        .line(None, later, 160)
        .unwrap()
        .contains("\"dismiss/fix tests\" completed"));

    let later = later + NOTICE_DURATION;
    status.tick(later);
    status.refresh_started(DashboardRefreshKind::Live, false, later);
    assert!(
        status.line(None, later, 120).is_none(),
        "periodic refresh stays quiet"
    );
    status.request_refresh();
    assert!(status
        .line(None, later, 120)
        .unwrap()
        .contains("Refreshing"));
    status.refreshed(Err("network down".to_owned().into()), &environment(), later);
    assert!(status
        .line(None, later, 120)
        .unwrap()
        .contains("Refresh failed: network down"));
    status.refreshed(Ok(()), &environment(), later);
    assert!(!status.has_error());
}

#[test]
fn action_refresh_failures_distinguish_command_success_and_only_link_recorded_errors() {
    let now = Instant::now();
    for log_path in [Some(PathBuf::from("/home/operator/actions.log")), None] {
        let logged = log_path.is_some();
        let mut status = DashboardStatus::default();
        complete(
            &mut status,
            DashboardActionOutcome::Succeeded(PrActionOnSuccess::Refresh),
            now,
        );
        status.refreshed(
            Err(PrActionFailure {
                message: "offline".to_owned(),
                log_path,
            }),
            &environment(),
            now,
        );
        let line = status.line(None, now, 160).unwrap();
        assert!(line.contains("\"dismiss/fix tests\" completed; refresh failed"));
        assert!(!line.contains("Running"));
        if logged {
            assert!(line.contains("see ~/actions.log"));
        } else {
            assert!(line.contains("refresh failed: offline"));
            assert!(!line.contains("see "));
        }
        status.clear_notice();
        assert!(status.line(None, now, 160).is_none());
    }
}

#[test]
fn redraw_errors_also_show_the_cause_and_expire() {
    let now = Instant::now();
    let mut status = DashboardStatus::default();
    status.reflowed(Err("bad row".to_owned()), now);
    assert!(status
        .line(None, now, 120)
        .unwrap()
        .contains("Cannot redraw list: bad row"));
    status.tick(now + ERROR_DURATION);
    assert!(status.line(None, now, 120).is_none());
}

#[test]
fn status_line_sanitizes_and_clips_text_while_painting_the_full_width() {
    for (kind, foreground) in [
        (StatusKind::Working, "38;2;0;0;0m"),
        (StatusKind::Success, "38;2;0;0;0m"),
        (StatusKind::Warning, "38;2;0;0;0m"),
        (StatusKind::Error, "38;2;192;48;40m"),
    ] {
        let message = StatusMessage::new(kind, "bad\nmessage\x1b[2J 漢字".repeat(5));
        for width in [1, 20, 100] {
            let line = message.render(width);
            assert!(!line.contains('\n'));
            assert!(!line.contains("\x1b[2J"));
            assert_eq!(rendered_visible_width(&line), width);
            assert!(line.contains("48;2;236;233;219m"));
            assert!(line.contains(foreground));
            assert!(line.ends_with(" \x1b[0m"));
        }
    }
}

fn complete(
    status: &mut DashboardStatus,
    outcome: DashboardActionOutcome,
    now: Instant,
) -> Option<PrActionOnSuccess> {
    status.action_completed(
        DashboardActionCompletion {
            action: action(now),
            outcome,
        },
        &environment(),
        now,
    )
}

fn environment() -> RuntimeEnvironment {
    RuntimeEnvironment::new(
        "/caller",
        [("HOME".to_owned(), "/home/operator".to_owned())],
    )
}

fn action(now: Instant) -> DashboardActionInfo {
    DashboardActionInfo {
        id: "fix-tests".to_owned(),
        title: "dismiss/fix tests".to_owned(),
        target: context(12, "owner/repo").key(),
        started: now,
    }
}
