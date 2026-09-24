use super::*;

#[test]
fn refresh_logging_failures_are_reported_even_if_later_writes_recover() {
    for recover in [false, true] {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut action = RunningPrAction {
            child: None,
            title: "Test".to_owned(),
            on_success: PrActionOnSuccess::Refresh,
            log: ActionLog {
                file: File::open(file.path()).unwrap(), // Read-only to simulate a failed append.
                path: file.path().to_path_buf(),
                metadata: serde_json::json!({}),
                started: Instant::now(),
                refresh_started: None,
            },
            refresh_log_error: None,
        };
        action.refresh_started("live");
        if recover {
            action.log.file = OpenOptions::new().append(true).open(file.path()).unwrap();
        }
        let failure = action.refreshed(Ok(())).unwrap_err();
        if recover {
            assert_eq!(failure.log_path.as_deref(), Some(file.path()));
            let log = fs::read_to_string(file.path()).unwrap();
            assert!(log.contains("Could not record refresh start"));
            assert!(log.contains("\"status\":\"refresh_failed\""));
            assert!(!log.contains("\"status\":\"success\""));
        } else {
            assert!(failure.log_path.is_none());
            assert!(failure.message.contains("Could not record refresh start"));
            assert!(failure.message.contains("logging failed"));
        }
    }
}
