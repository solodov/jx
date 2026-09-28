use super::*;
use crate::commands::{dashboard::test_support::context, pr_actions::PrActionSet};
#[cfg(unix)]
use crate::repository::{PrActionConfigScope, PrActionSource};
use std::sync::{mpsc, Mutex};

#[cfg(unix)]
#[test]
fn actions_and_local_reloads_finish_while_remote_loading_is_held_open() {
    for policy in [PrActionOnSuccess::None, PrActionOnSuccess::RefreshLocal] {
        for changed_remote_state in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let environment = environment(temp.path());
            let (loader, started, release) = controlled_loader(temp.path());
            let mut activity =
                DashboardActivity::new(loader, &environment, PrActionSet::Review, 300);
            seed_frame(&mut activity);
            activity.tick(false, size());
            assert_eq!(
                started.recv_timeout(Duration::from_secs(5)).unwrap(),
                DashboardRefreshKind::Live
            );
            assert!(activity.can_start(PrActionOnSuccess::None));
            assert!(activity.can_start(PrActionOnSuccess::RefreshLocal));
            assert!(!activity.can_start(PrActionOnSuccess::Refresh));
            activity.start_action(
                action(
                    temp.path(),
                    PrActionOnSuccess::Refresh,
                    "printf blocked > blocked",
                ),
                size(),
            );
            assert!(!activity.actions.is_running());
            assert!(!temp.path().join("blocked").exists());

            let command = if policy == PrActionOnSuccess::RefreshLocal {
                "printf old > dismissal"
            } else {
                "printf done > command"
            };
            activity.start_action(action(temp.path(), policy, command), size());
            assert!(activity.actions.is_running());
            assert!(!activity.can_start(PrActionOnSuccess::None));
            drive_until(&mut activity, |activity| !activity.actions.is_busy());
            assert!(
                activity.remote.is_some(),
                "the action must not wait for remote completion"
            );
            if policy == PrActionOnSuccess::RefreshLocal {
                assert_eq!(
                    started.recv_timeout(Duration::from_secs(5)).unwrap(),
                    DashboardRefreshKind::Local
                );
                assert!(activity.view.frame.as_ref().unwrap().rows.is_empty());
            } else {
                assert!(temp.path().join("command").exists());
                assert!(
                    started.try_recv().is_err(),
                    "none must not request a reload"
                );
            }
            activity.request_refresh(); // Coalesces with the active live fetch.
            release
                .send(Ok(if changed_remote_state { "new" } else { "old" }))
                .unwrap();
            drive_until(&mut activity, |activity| !activity.is_busy());
            assert_eq!(
                started.recv_timeout(Duration::from_secs(5)).unwrap(),
                DashboardRefreshKind::Local
            );
            assert!(started.try_recv().is_err(), "no duplicate remote fetch");
            let visible = policy == PrActionOnSuccess::None || changed_remote_state;
            assert_eq!(
                activity.view.frame.as_ref().unwrap().rows.len(),
                usize::from(visible)
            );
            assert!(activity.can_start(PrActionOnSuccess::Refresh));
        }
    }
}

#[cfg(unix)]
#[test]
fn local_actions_can_run_during_another_actions_live_followup() {
    let temp = tempfile::tempdir().unwrap();
    let environment = environment(temp.path());
    let (loader, started, release) = controlled_loader(temp.path());
    let mut activity = DashboardActivity::new(loader, &environment, PrActionSet::Review, 300);
    seed_frame(&mut activity);
    activity.start_action(
        action(temp.path(), PrActionOnSuccess::Refresh, "exit 0"),
        size(),
    );
    drive_until(&mut activity, |activity| activity.remote.is_some());
    assert_eq!(
        started.recv_timeout(Duration::from_secs(5)).unwrap(),
        DashboardRefreshKind::Live
    );
    assert!(activity.actions.is_busy());
    assert!(activity.can_start(PrActionOnSuccess::None));
    assert!(activity.can_start(PrActionOnSuccess::RefreshLocal));
    activity.start_action(
        action(
            temp.path(),
            PrActionOnSuccess::RefreshLocal,
            "printf old > dismissal",
        ),
        size(),
    );
    drive_until(&mut activity, |activity| {
        activity.can_start(PrActionOnSuccess::None)
    });
    assert!(activity.view.frame.as_ref().unwrap().rows.is_empty());
    assert!(
        activity.actions.is_busy(),
        "local completion must not finish the live followup"
    );
    assert_eq!(
        started.recv_timeout(Duration::from_secs(5)).unwrap(),
        DashboardRefreshKind::Local
    );
    release.send(Ok("old")).unwrap();
    drive_until(&mut activity, |activity| !activity.is_busy());
    assert!(activity.view.frame.as_ref().unwrap().rows.is_empty());
}

#[cfg(unix)]
#[test]
fn remote_completion_waits_for_an_in_flight_local_reload_without_losing_either_result() {
    let temp = tempfile::tempdir().unwrap();
    let environment = environment(temp.path());
    let (loader, started, release_remote) = controlled_loader(temp.path());
    let (release_local, wait_local) = mpsc::channel();
    let wait_local = Mutex::new(wait_local);
    let loader: DashboardFrameLoader = Arc::new(move |kind| {
        if kind == DashboardLoadKind::Local {
            wait_local
                .lock()
                .unwrap()
                .recv()
                .map_err(|error| error.to_string())?;
        }
        loader(kind)
    });
    let mut activity = DashboardActivity::new(loader, &environment, PrActionSet::Review, 300);
    seed_frame(&mut activity);
    activity.start_action(
        action(temp.path(), PrActionOnSuccess::Refresh, "exit 0"),
        size(),
    );
    drive_until(&mut activity, |activity| activity.remote.is_some());
    assert_eq!(
        started.recv_timeout(Duration::from_secs(5)).unwrap(),
        DashboardRefreshKind::Live
    );
    activity.start_action(
        action(
            temp.path(),
            PrActionOnSuccess::RefreshLocal,
            "printf old > dismissal",
        ),
        size(),
    );
    drive_until(&mut activity, |activity| activity.reload.is_some());
    release_remote.send(Ok("old")).unwrap();
    drive_until(&mut activity, |activity| activity.remote_ready.is_some());
    assert!(activity.reload.is_some());
    assert!(activity.actions.is_busy());
    assert!(activity.view.frame.as_ref().unwrap().text.contains("old"));
    release_local.send(()).unwrap();
    drive_until(&mut activity, |activity| !activity.is_busy());
    assert!(activity.view.frame.as_ref().unwrap().rows.is_empty());
    for _ in 0..2 {
        assert_eq!(
            started.recv_timeout(Duration::from_secs(5)).unwrap(),
            DashboardRefreshKind::Local
        );
    }
    assert!(started.try_recv().is_err());
}

#[cfg(unix)]
#[test]
fn failure_and_cancellation_do_not_discard_remote_results_or_request_local_reloads() {
    for cancel in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let environment = environment(temp.path());
        let (loader, started, release) = controlled_loader(temp.path());
        let mut activity = DashboardActivity::new(loader, &environment, PrActionSet::Review, 300);
        seed_frame(&mut activity);
        activity.tick(false, size());
        assert_eq!(
            started.recv_timeout(Duration::from_secs(5)).unwrap(),
            DashboardRefreshKind::Live
        );
        activity.start_action(
            action(
                temp.path(),
                PrActionOnSuccess::RefreshLocal,
                if cancel { "sleep 30" } else { "exit 7" },
            ),
            size(),
        );
        if cancel {
            assert!(activity.actions.cancel());
        }
        drive_until(&mut activity, |activity| !activity.actions.is_busy());
        assert!(activity.remote.is_some());
        assert!(started.try_recv().is_err());
        release.send(Ok("new")).unwrap();
        drive_until(&mut activity, |activity| !activity.is_busy());
        assert!(activity.view.frame.as_ref().unwrap().text.contains("new"));
    }
}

#[test]
fn completed_remote_results_wait_for_menus_and_stack_status_never_loads_local() {
    let temp = tempfile::tempdir().unwrap();
    let environment = environment(temp.path());
    let (loader, started, release) = controlled_loader(temp.path());
    let mut activity = DashboardActivity::new(loader, &environment, PrActionSet::StackStatus, 300);
    seed_frame(&mut activity);
    activity.tick(false, size());
    assert_eq!(
        started.recv_timeout(Duration::from_secs(5)).unwrap(),
        DashboardRefreshKind::Live
    );
    release.send(Ok("new")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while activity.remote_ready.is_none() {
        activity.tick(true, size());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(activity.view.frame.as_ref().unwrap().text.contains("old"));
    assert!(!activity.can_start(PrActionOnSuccess::Refresh));
    activity.tick(false, size());
    assert!(!activity.is_busy());
    assert!(activity
        .view
        .frame
        .as_ref()
        .unwrap()
        .text
        .contains("remote snapshot"));
    assert!(started.try_recv().is_err());
}

#[cfg(unix)]
#[test]
fn remote_timeout_and_failure_do_not_block_or_finish_a_local_action() {
    let temp = tempfile::tempdir().unwrap();
    let environment = environment(temp.path());
    let (loader, started, release) = controlled_loader(temp.path());
    let mut activity = DashboardActivity::new(loader, &environment, PrActionSet::Review, 300);
    seed_frame(&mut activity);
    activity.tick(false, size());
    assert_eq!(
        started.recv_timeout(Duration::from_secs(5)).unwrap(),
        DashboardRefreshKind::Live
    );
    activity.remote.as_mut().unwrap().started = Instant::now() - DASHBOARD_REFRESH_TIMEOUT;
    activity.tick(false, size());
    assert!(activity.remote.as_ref().unwrap().timed_out);
    assert!(activity.can_start(PrActionOnSuccess::RefreshLocal));
    activity.start_action(
        action(
            temp.path(),
            PrActionOnSuccess::RefreshLocal,
            "printf old > dismissal",
        ),
        size(),
    );
    release.send(Err("network failed".to_owned())).unwrap();
    drive_until(&mut activity, |activity| !activity.is_busy());
    assert!(activity.view.frame.as_ref().unwrap().rows.is_empty());
    let log = fs::read_to_string(temp.path().join(".local/state/jx/jx-actions.log")).unwrap();
    assert!(log.contains("[jx-dashboard]"));
    assert!(log.contains("network failed"));
    assert_eq!(
        started.recv_timeout(Duration::from_secs(5)).unwrap(),
        DashboardRefreshKind::Local
    );
    assert!(
        started.try_recv().is_err(),
        "failed remote loads must not rebuild the cache view"
    );
}

fn seed_frame(activity: &mut DashboardActivity<'_>) {
    activity.queue_result(DashboardRefreshKind::Local, Ok(snapshot("old", true)));
    activity.tick(false, size());
}

#[cfg(unix)]
fn drive_until(
    activity: &mut DashboardActivity<'_>,
    done: impl Fn(&DashboardActivity<'_>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        activity.tick(false, size());
        if done(activity) {
            return;
        }
        assert!(Instant::now() < deadline, "dashboard work did not complete");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn environment(path: &Path) -> RuntimeEnvironment {
    RuntimeEnvironment::new(path, [("HOME".to_owned(), path.display().to_string())])
}

fn size() -> DashboardTerminalSize {
    DashboardTerminalSize::new(100, 30)
}

#[cfg(unix)]
fn action(path: &Path, policy: PrActionOnSuccess, command: &str) -> PreparedPrAction {
    PreparedPrAction {
        id: format!("{policy:?}"),
        title: format!("{policy:?}"),
        target: context(12, "owner/repo").key(),
        command: vec!["sh".to_owned(), "-c".to_owned(), command.to_owned()],
        cwd: path.to_path_buf(),
        source: PrActionSource {
            path: path.join("config.toml"),
            scope: PrActionConfigScope::Global,
        },
        on_success: policy,
    }
}

fn snapshot(text: &str, visible: bool) -> DashboardFrameSnapshot {
    let text = text.to_owned();
    DashboardFrameSnapshot::new(move |_| {
        let mut frame = PullRequestTableFrame::default();
        if visible {
            frame.push_pr_line(&text, Some(context(12, "owner/repo")));
        }
        Ok(frame)
    })
}

type ControlledLoader = (
    DashboardFrameLoader,
    mpsc::Receiver<DashboardRefreshKind>,
    mpsc::Sender<Result<&'static str, String>>,
);

/// Remote state and dismissal baseline are separate, as in the real cache/action store.
fn controlled_loader(path: &Path) -> ControlledLoader {
    let path = path.to_path_buf();
    fs::write(path.join("cache"), "old").unwrap();
    let (started, events) = mpsc::channel();
    let (release, wait) = mpsc::channel::<Result<&'static str, String>>();
    let wait = Mutex::new(wait);
    let loader: DashboardFrameLoader = Arc::new(move |kind| {
        started
            .send(if kind == DashboardLoadKind::Live {
                DashboardRefreshKind::Live
            } else {
                DashboardRefreshKind::Local
            })
            .unwrap();
        if kind == DashboardLoadKind::Live {
            let state = wait
                .lock()
                .unwrap()
                .recv()
                .map_err(|error| error.to_string())??;
            fs::write(path.join("cache"), state).unwrap();
            Ok(snapshot("remote snapshot", true))
        } else {
            let state = fs::read_to_string(path.join("cache")).unwrap();
            let dismissed = fs::read_to_string(path.join("dismissal")).ok();
            Ok(snapshot(&state, dismissed.as_deref() != Some(&state)))
        }
    });
    (loader, events, release)
}
