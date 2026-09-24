use super::*;

#[test]
fn standalone_failures_in_both_dashboards_are_logged_and_shown_as_one_notice() {
    let temp = tempfile::tempdir().unwrap();
    let environment = RuntimeEnvironment::new(
        temp.path(),
        [("HOME".to_owned(), temp.path().display().to_string())],
    );
    let path = temp.path().join(".local/state/jx/jx-actions.log");
    let mut actions = DashboardActions::default();
    let now = Instant::now();
    let details = "2 repository refreshes failed:\nowner/a: offline\nowner/b: bad checkout";
    for action_set in [PrActionSet::Review, PrActionSet::StackStatus] {
        let failure = actions
            .refreshed(Err(details.to_owned()), &environment, action_set)
            .unwrap_err();
        assert_eq!(failure.log_path.as_deref(), Some(path.as_path()));
        let mut status = DashboardStatus::default();
        status.refreshed(Err(failure), &environment, now);
        let line = status.line(None, now, 120).unwrap();
        assert!(line.contains("Refresh failed, see ~/.local/state/jx/jx-actions.log"));
        assert!(!line.contains("owner/a"));
        assert!(!line.contains("owner/b"));
        assert!(!line.contains('\n'));
        actions.refreshed(Ok(()), &environment, action_set).unwrap();
    }
    let timeout = actions.refresh_timed_out(
        "refresh timed out".to_owned(),
        &environment,
        PrActionSet::Review,
    );
    assert_eq!(timeout.log_path.as_deref(), Some(path.as_path()));
    let log = fs::read_to_string(path).unwrap();
    let records = log
        .lines()
        .filter_map(|line| line.strip_prefix("[jx-dashboard] "))
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["dashboard"], "review");
    assert_eq!(records[1]["dashboard"], "stack-status");
    assert_eq!(records[0]["message"], details);
    assert_eq!(records[1]["message"], details);
    assert_eq!(records[2]["message"], "refresh timed out");
}

#[test]
fn standalone_failure_logging_honors_the_override_and_never_links_an_unwritten_log() {
    let temp = tempfile::tempdir().unwrap();
    let environment = RuntimeEnvironment::new(
        temp.path(),
        [("JX_ACTION_LOG".to_owned(), "custom.log".to_owned())],
    );
    let failure = record_refresh_failure("offline".to_owned(), &environment, PrActionSet::Review);
    assert_eq!(failure.log_path, Some(temp.path().join("custom.log")));
    let blocked = RuntimeEnvironment::new(
        temp.path(),
        [(
            "JX_ACTION_LOG".to_owned(),
            temp.path().display().to_string(),
        )],
    );
    let failure = record_refresh_failure("offline".to_owned(), &blocked, PrActionSet::Review);
    assert!(failure.log_path.is_none());
    assert!(failure.message.contains("offline"));
    assert!(failure.message.contains("logging failed"));
}
