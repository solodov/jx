use super::*;

#[test]
fn success_is_silent_and_failure_requires_acknowledgement() {
    let mut actions = DashboardActions::default();
    actions.complete(Ok(()));
    assert!(!actions.is_running());
    assert!(actions.failure.is_none());
    actions.complete(Err(PrActionFailure {
        message: "open failed".to_owned(),
        log_path: Some(PathBuf::from("/logs/jx-actions.log")),
    }));
    assert!(!actions.poll());
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
