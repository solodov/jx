use super::*;
use std::fs;

#[test]
fn refresh_logging_failures_are_reported_even_if_later_writes_recover() {
    for recover in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("actions.log");
        let sink = LogSink::open(&path).unwrap();
        // Replace the active file with a directory to fail appends without exposing sink internals.
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let mut action = RunningPrAction {
            process: None,
            title: "Test".to_owned(),
            on_success: PrActionOnSuccess::Refresh,
            log: ActionLog {
                sink,
                metadata: serde_json::json!({}),
                started: Instant::now(),
                refresh_started: None,
            },
            refresh_log_error: None,
        };
        action.refresh_started("live");
        if recover {
            fs::remove_dir(&path).unwrap();
        }
        let failure = action.refreshed(Ok(())).unwrap_err();
        if recover {
            assert_eq!(failure.log_path.as_deref(), Some(path.as_path()));
            let log = fs::read_to_string(&path).unwrap();
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
