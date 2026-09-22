use super::*;

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
            }
            DashboardActionOutcome::Cancelled(_) => assert!(cancel),
            DashboardActionOutcome::Failed(_) => assert_eq!(command, "exit 1"),
        }
        assert!(actions.poll().is_none());
        assert!(actions.running_info().is_none());
    }
}
