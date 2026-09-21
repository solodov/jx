use super::*;

#[test]
fn local_action_completion_is_delivered_once_and_failure_does_not_reload() {
    let mut actions = DashboardActions {
        on_success: PrActionOnSuccess::RefreshLocal,
        ..DashboardActions::default()
    };
    actions.complete(Ok(()));
    assert_eq!(actions.poll(), Some(PrActionOnSuccess::RefreshLocal));
    assert_eq!(actions.poll(), None);
    actions.complete(Err(PrActionFailure {
        message: "dismiss failed".to_owned(),
        log_path: None,
    }));
    assert_eq!(actions.poll(), None);
    assert!(actions.failure.is_some());
}

#[cfg(unix)]
#[test]
fn running_actions_retain_their_reload_policy_and_failed_or_cancelled_actions_do_not_reload() {
    use crate::commands::dashboard::test_support::context;
    use crate::repository::{PrActionConfigScope, PrActionSource};
    let temp = tempfile::tempdir().unwrap();
    let environment = RuntimeEnvironment::new(
        temp.path(),
        [("HOME".to_owned(), temp.path().display().to_string())],
    );
    for (command, policy, cancel) in [
        ("exit 0", PrActionOnSuccess::RefreshLocal, false),
        ("exit 0", PrActionOnSuccess::Refresh, false),
        ("exit 1", PrActionOnSuccess::RefreshLocal, false),
        ("sleep 30", PrActionOnSuccess::RefreshLocal, true),
    ] {
        let mut actions = DashboardActions::default();
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
        if cancel {
            assert!(actions.cancel());
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while actions.is_running() {
            actions.poll_running();
            assert!(std::time::Instant::now() < deadline, "action timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        let succeeded = command == "exit 0";
        assert_eq!(actions.poll(), succeeded.then_some(policy));
        assert_eq!(actions.poll(), None);
        assert_eq!(actions.failure.is_none(), succeeded);
    }
}

#[test]
fn success_is_silent_and_failure_requires_acknowledgement() {
    let mut actions = DashboardActions::default();
    actions.complete(Ok(()));
    assert!(!actions.is_running());
    assert!(actions.failure.is_none());
    assert_eq!(actions.poll(), Some(PrActionOnSuccess::Refresh));
    assert_eq!(actions.poll(), None);
    actions.complete(Err(PrActionFailure {
        message: "open failed".to_owned(),
        log_path: Some(PathBuf::from("/logs/jx-actions.log")),
    }));
    assert_eq!(actions.poll(), None);
    assert!(
        actions.failure.is_some(),
        "refresh polling must not clear the failure"
    );
    assert!(actions.handle_failure_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
    assert!(actions.failure.is_some());
    let mut repeat = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    repeat.kind = KeyEventKind::Repeat;
    assert!(actions.handle_failure_key(repeat));
    assert!(actions.failure.is_some());
    assert!(actions.handle_failure_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    assert!(actions.failure.is_none());
    assert!(!actions.handle_failure_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
}
