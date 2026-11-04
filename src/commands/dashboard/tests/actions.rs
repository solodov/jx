use super::*;

#[cfg(unix)]
#[test]
fn first_cancel_keeps_polling_until_child_cleanup_finishes() {
    let temp = tempfile::tempdir().unwrap();
    let mut actions = start_cancellable_action(temp.path(),
        "trap 'touch interrupted; while test ! -e release; do sleep 0.01; done; rm held.lock; exit 0' INT; touch held.lock ready; while :; do sleep 0.05; done");
    wait_for_action(|| temp.path().join("ready").exists());
    assert!(actions.cancel());
    wait_for_action(|| temp.path().join("interrupted").exists());
    assert!(actions.is_running());
    assert!(actions.running_info().unwrap().cancelling);
    assert!(!actions.can_start(PrActionOnSuccess::None));
    assert!(temp.path().join("held.lock").exists());
    std::fs::write(temp.path().join("release"), b"").unwrap();
    wait_for_action(|| {
        actions.poll_running();
        !actions.is_running()
    });
    assert!(!temp.path().join("held.lock").exists());
    assert!(matches!(
        actions.poll().unwrap().outcome,
        DashboardActionOutcome::Cancelled(_)
    ));
    assert!(!actions.is_busy());
}

#[cfg(unix)]
#[test]
fn second_cancel_forces_an_unresponsive_child() {
    let temp = tempfile::tempdir().unwrap();
    let mut actions =
        start_cancellable_action(temp.path(), "trap '' INT; touch ready; exec sleep 30");
    wait_for_action(|| temp.path().join("ready").exists());
    assert!(actions.cancel());
    assert!(actions.is_running());
    assert!(actions.cancel());
    assert!(!actions.is_running());
    assert!(matches!(
        actions.poll().unwrap().outcome,
        DashboardActionOutcome::Cancelled(_)
    ));
}

#[cfg(unix)]
fn start_cancellable_action(root: &Path, command: &str) -> DashboardActions {
    use crate::commands::dashboard::test_support::context;
    use crate::repository::{PrActionConfigScope, PrActionSource};
    let environment =
        RuntimeEnvironment::new(root, [("HOME".to_owned(), root.display().to_string())]);
    let mut actions = DashboardActions::default();
    actions.start(
        PreparedPrAction {
            id: "test".to_owned(),
            title: "Test".to_owned(),
            target: context(12, "owner/repo").key(),
            command: vec!["sh".to_owned(), "-c".to_owned(), command.to_owned()],
            cwd: root.to_path_buf(),
            source: PrActionSource {
                path: root.join("config.toml"),
                scope: PrActionConfigScope::Global,
            },
            on_success: PrActionOnSuccess::RefreshLocal,
        },
        &environment,
        PrActionSet::Review,
    );
    actions
}

#[cfg(unix)]
fn wait_for_action(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "action timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
#[test]
fn actions_report_success_failure_and_cancellation_once_without_blocking_the_next_action() {
    use crate::commands::dashboard::test_support::context;
    use crate::repository::{PrActionConfigScope, PrActionSource};
    let temp = tempfile::tempdir().unwrap();
    let environment = RuntimeEnvironment::new(
        temp.path(),
        [("HOME".to_owned(), temp.path().display().to_string())],
    );
    let mut actions = DashboardActions::default();
    for (command, policy, cancel) in [
        ("exit 1", PrActionOnSuccess::RefreshLocal, false),
        ("exit 0", PrActionOnSuccess::RefreshLocal, false),
        ("exit 0", PrActionOnSuccess::Refresh, false),
        ("exit 0", PrActionOnSuccess::None, false),
        ("exit 1", PrActionOnSuccess::None, false),
        ("sleep 30", PrActionOnSuccess::None, true),
        ("sleep 30", PrActionOnSuccess::RefreshLocal, true),
    ] {
        actions.start(
            PreparedPrAction {
                id: "dismiss".to_owned(),
                title: "Dismiss".to_owned(),
                target: context(12, "owner/repo").key(),
                command: vec!["sh".to_owned(), "-c".to_owned(), command.to_owned()],
                cwd: temp.path().to_path_buf(),
                source: PrActionSource {
                    path: temp.path().join("config.toml"),
                    scope: PrActionConfigScope::Global,
                },
                on_success: policy,
            },
            &environment,
            PrActionSet::Review,
        );
        assert!(actions.is_running());
        assert_eq!(actions.running_info().unwrap().title, "Dismiss");
        assert_eq!(
            actions.running_info().unwrap().target,
            context(12, "owner/repo").key()
        );
        if cancel {
            assert!(actions.cancel());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while actions.is_running() {
            actions.poll_running();
            assert!(Instant::now() < deadline, "action timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        let completion = actions.poll().unwrap();
        assert_eq!(completion.action.target.number, 12);
        match completion.outcome {
            DashboardActionOutcome::Succeeded(actual) => {
                assert_eq!(command, "exit 0");
                assert_eq!(actual, policy);
                if policy != PrActionOnSuccess::None {
                    assert!(actions.is_busy());
                    assert!(
                        !actions.cancel(),
                        "the completed command is no longer cancellable"
                    );
                    let kind = DashboardRefreshKind::for_action(policy).unwrap();
                    actions.refresh_started(kind);
                    actions
                        .refreshed(kind, Ok(()), &environment, PrActionSet::Review)
                        .unwrap();
                }
            }
            DashboardActionOutcome::Cancelled(_) => assert!(cancel),
            DashboardActionOutcome::Failed(_) => assert_eq!(command, "exit 1"),
        }
        assert!(actions.poll().is_none());
        assert!(actions.running_info().is_none());
        assert!(!actions.is_busy());
    }
}
