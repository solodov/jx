use super::*;
use std::{collections::BTreeSet, process::Command, thread};

#[test]
fn rotation_keeps_complete_records_and_bounded_archives() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.log");
    let sink = LogSink::with_limits(path.clone(), 8, 2).unwrap();
    for record in [b"first\n".as_slice(), b"second\n", b"third\n", b"fourth\n"] {
        sink.append(record).unwrap();
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), "fourth\n");
    assert_eq!(
        fs::read_to_string(path.with_extension("log.1")).unwrap(),
        "third\n"
    );
    assert_eq!(
        fs::read_to_string(path.with_extension("log.2")).unwrap(),
        "second\n"
    );
    assert!(!path.with_extension("log.3").exists());
    assert!(sink.append(b"too large\n").is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "fourth\n");
}

#[test]
fn independent_sinks_reopen_after_another_writer_rotates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.log");
    let first = LogSink::with_limits(path.clone(), 8, 2).unwrap();
    let second = LogSink::with_limits(path.clone(), 8, 2).unwrap();
    first.append(b"first\n").unwrap();
    second.append(b"second\n").unwrap();
    first.append(b"third\n").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "third\n");
    assert_eq!(
        fs::read_to_string(path.with_extension("log.1")).unwrap(),
        "second\n"
    );
    assert_eq!(
        fs::read_to_string(path.with_extension("log.2")).unwrap(),
        "first\n"
    );
}

#[test]
fn failures_are_fallible_and_do_not_claim_records_were_written() {
    let directory = tempfile::tempdir().unwrap();
    assert!(LogSink::open(directory.path()).is_err());
    let path = directory.path().join("test.log");
    let sink = LogSink::open(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(sink
        .append_json(&serde_json::json!({"status": "test"}))
        .is_err());
}

#[cfg(unix)]
#[test]
fn active_archived_and_lock_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.log");
    let sink = LogSink::with_limits(path.clone(), 8, 2).unwrap();
    sink.append(b"first\n").unwrap();
    sink.append(b"second\n").unwrap();
    for entry in fs::read_dir(directory.path()).unwrap() {
        assert_eq!(
            entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn multiple_threads_and_processes_append_intact_json_records_while_rotating() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.log");
    let mut children = (0..3)
        .map(|writer| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "commands::log_sink::tests::subprocess_writer",
                    "--nocapture",
                ])
                .env("JX_TEST_LOG_SINK", &path)
                .env("JX_TEST_LOG_WRITER", format!("process-{writer}"))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let threads = (0..3)
        .map(|writer| {
            let path = path.clone();
            thread::spawn(move || write_records(&path, &format!("thread-{writer}")))
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    for child in children.drain(..) {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut records = BTreeSet::new();
    for entry in fs::read_dir(directory.path()).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_some_and(|extension| extension == "lock")
        {
            continue;
        }
        assert!(path.metadata().unwrap().len() <= 4096);
        for line in fs::read_to_string(path).unwrap().lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["payload"].as_str().unwrap().len(), 300);
            assert!(records.insert((
                record["writer"].as_str().unwrap().to_owned(),
                record["index"].as_u64().unwrap()
            )));
        }
    }
    assert_eq!(records.len(), 6 * 20);
}

#[test]
fn subprocess_writer() {
    let Ok(path) = std::env::var("JX_TEST_LOG_SINK") else {
        return;
    };
    write_records(
        Path::new(&path),
        &std::env::var("JX_TEST_LOG_WRITER").unwrap(),
    );
}

fn write_records(path: &Path, writer: &str) {
    let sink = LogSink::with_limits(path.to_path_buf(), 4096, 32).unwrap();
    for index in 0..20 {
        sink.append_json(
            &serde_json::json!({"writer": writer, "index": index, "payload": "x".repeat(300)}),
        )
        .unwrap();
    }
}
