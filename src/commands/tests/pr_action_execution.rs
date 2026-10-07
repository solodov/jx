use super::*;
use crate::commands::dashboard::test_support::context;
use crate::repository::{PrActionConfigScope, PrActionSource};
use std::fs;

fn invocation(command: &[&str], cwd: &Path) -> PreparedPrAction {
    PreparedPrAction {
        id: "test".to_owned(),
        title: "Test".to_owned(),
        target: context(12, "owner/repo").key(),
        command: command.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: cwd.to_path_buf(),
        on_success: crate::repository::PrActionOnSuccess::default(),
        source: PrActionSource {
            path: cwd.join("config.toml"),
            scope: PrActionConfigScope::Global,
        },
    }
}

fn action_environment(root: &Path) -> RuntimeEnvironment {
    RuntimeEnvironment::new(root, [("HOME".to_owned(), root.display().to_string())])
}

#[cfg(unix)]
fn wait_for_action(running: &mut RunningPrAction) -> Result<(), PrActionFailure> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = running.poll() {
            return result;
        }
        assert!(std::time::Instant::now() < deadline, "action timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn quiet_execution_logs_both_streams_argv_and_status_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let action = invocation(
        &["sh", "-c", "! test -t 0 && ! test -t 1 && ! test -t 2 || exit 9; read value && exit 8; printf '%s\\n' \"$1\" \"$2\"; printf 'stderr marker' >&2; printf 'cwd marker' > cwd-marker; exit 7", "action", "$(touch injected) ; {title}", "two words"],
        temp.path(),
    );
    let environment = action_environment(temp.path());
    let mut running =
        RunningPrAction::start(action, &environment, PrActionSet::StackStatus).unwrap();
    let failure = wait_for_action(&mut running).unwrap_err();
    assert_eq!(failure.message, "Test failed");
    let path = failure.log_path.unwrap();
    assert_eq!(path, temp.path().join(".local/state/jx/jx-actions.log"));
    let log = fs::read_to_string(&path).unwrap();
    assert!(log.contains("$(touch injected) ; {title}\ntwo words\n"));
    assert!(log.contains("stderr marker"));
    assert!(log.contains(&temp.path().display().to_string()));
    assert!(log.contains("exit status: 7"));
    assert!(log.contains("\"dashboard\":\"stack-status\""));
    assert!(log.contains("\"repo\":\"owner/repo\""));
    assert!(!temp.path().join("injected").exists());
    assert_eq!(
        fs::read_to_string(temp.path().join("cwd-marker")).unwrap(),
        "cwd marker"
    );
    let mut success = RunningPrAction::start(
        invocation(&["sh", "-c", "echo success marker"], temp.path()),
        &environment,
        PrActionSet::Review,
    )
    .unwrap();
    wait_for_action(&mut success).unwrap();
    let log = fs::read_to_string(&path).unwrap();
    assert!(log.contains("success marker"));
    assert!(log.contains("\"status\":\"success\""));
    assert!(
        log.contains("stderr marker"),
        "the log must append rather than truncate"
    );
}

#[cfg(unix)]
#[test]
fn configured_refreshes_log_phases_timings_failures_and_timeout_recovery() {
    use crate::repository::PrActionOnSuccess;
    for (policy, kind, fail, timeout) in [
        (PrActionOnSuccess::Refresh, "live", false, false),
        (PrActionOnSuccess::RefreshLocal, "local", true, false),
        (PrActionOnSuccess::Refresh, "live", false, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut action = invocation(&["sh", "-c", "exit 0"], temp.path());
        action.on_success = policy;
        let mut running = RunningPrAction::start(
            action,
            &action_environment(temp.path()),
            PrActionSet::Review,
        )
        .unwrap();
        wait_for_action(&mut running).unwrap();
        let path = temp.path().join(".local/state/jx/jx-actions.log");
        let command_log = fs::read_to_string(&path).unwrap();
        assert!(command_log.contains("\"status\":\"command_success\""));
        assert!(!command_log.contains("\"status\":\"success\""));
        running.refresh_started(kind);
        if timeout {
            let error = running.refresh_timed_out("refresh timed out");
            assert_eq!(error.log_path.as_deref(), Some(path.as_path()));
        }
        let result = running.refreshed(if fail {
            Err("snapshot could not render".to_owned())
        } else {
            Ok(())
        });
        if fail {
            assert_eq!(
                result.unwrap_err().log_path.as_deref(),
                Some(path.as_path())
            );
        } else {
            result.unwrap();
        }
        let log = fs::read_to_string(&path).unwrap();
        let records = log
            .lines()
            .filter_map(|line| line.strip_prefix("[jx-action] "))
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        let mut expected = vec!["start", "command_success", "refresh_start"];
        if timeout {
            expected.push("refresh_timeout");
        }
        if fail {
            expected.push("refresh_failed");
            assert!(log.contains("snapshot could not render"));
        } else {
            expected.extend(["refresh_success", "success"]);
        }
        assert_eq!(
            records
                .iter()
                .map(|record| record["status"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(records
            .windows(2)
            .all(|pair| pair[0]["elapsed_ms"].as_u64().unwrap()
                <= pair[1]["elapsed_ms"].as_u64().unwrap()));
        for record in &records[2..] {
            assert_eq!(record["refresh_kind"], kind);
            assert!(record["refresh_elapsed_ms"].as_u64().is_some());
            assert_eq!(record["on_success"], policy.as_str());
        }
    }
}

#[test]
fn spawn_failures_are_logged_and_unwritable_logs_prevent_execution() {
    let temp = tempfile::tempdir().unwrap();
    let environment = action_environment(temp.path());
    let missing = invocation(&["/definitely/missing/jx-action"], temp.path());
    let error = RunningPrAction::start(missing.clone(), &environment, PrActionSet::Review)
        .err()
        .unwrap();
    let log = fs::read_to_string(error.log_path.unwrap()).unwrap();
    assert!(log.contains("could not start"));
    assert!(log.contains("\"status\":\"failed\""));
    let blocked = RuntimeEnvironment::new(
        temp.path(),
        [(
            "JX_ACTION_LOG".to_owned(),
            temp.path().display().to_string(),
        )],
    );
    let error = RunningPrAction::start(missing, &blocked, PrActionSet::Review)
        .err()
        .unwrap();
    assert!(error.message.contains("cannot open log"));
    assert_eq!(error.log_path, None);
    #[cfg(unix)]
    {
        let action = invocation(&["sh", "-c", "touch must-not-run"], temp.path());
        assert!(RunningPrAction::start(action, &blocked, PrActionSet::Review).is_err());
        assert!(!temp.path().join("must-not-run").exists());
    }
}

#[cfg(unix)]
#[test]
fn pending_actions_can_be_cancelled_and_dropped_without_terminal_prompts() {
    let temp = tempfile::tempdir().unwrap();
    let environment = RuntimeEnvironment::new(
        temp.path(),
        [("JX_ACTION_LOG".to_owned(), "action.log".to_owned())],
    );
    let mut running = RunningPrAction::start(
        invocation(&["sh", "-c", "sleep 30"], temp.path()),
        &environment,
        PrActionSet::Review,
    )
    .unwrap();
    assert!(running.poll().is_none());
    let failure = running.cancel();
    assert_eq!(failure.message, "Test cancelled");
    assert_eq!(
        failure.log_path.as_deref(),
        Some(temp.path().join("action.log").as_path())
    );
    let running = RunningPrAction::start(
        invocation(&["sh", "-c", "sleep 30"], temp.path()),
        &environment,
        PrActionSet::Review,
    )
    .unwrap();
    drop(running);
    let log = fs::read_to_string(temp.path().join("action.log")).unwrap();
    assert!(log.contains("cancelled by operator"));
    assert!(log.contains("dashboard closed"));
}

#[cfg(unix)]
#[test]
fn large_action_output_is_drained_before_success_and_rotates_through_the_sink() {
    let temp = tempfile::tempdir().unwrap();
    let environment = action_environment(temp.path());
    let path = temp.path().join(".local/state/jx/jx-actions.log");
    LogSink::open(&path).unwrap();
    fs::write(&path, vec![b'x'; 20 * 1024 * 1024 - 1024]).unwrap();
    let action = invocation(&[
        "sh", "-c",
        "i=0; while [ \"$i\" -lt 100 ]; do printf '%08000d' 0; printf '%08000d' 1 >&2; i=$((i + 1)); done; printf 'stdout tail'; printf 'stderr tail' >&2",
    ], temp.path());
    let mut running = RunningPrAction::start(action, &environment, PrActionSet::Review).unwrap();
    wait_for_action(&mut running).unwrap();
    let log = fs::read_to_string(&path).unwrap();
    let success = log.find("\"status\":\"success\"").unwrap();
    assert!(log.find("stdout tail").unwrap() < success);
    assert!(log.find("stderr tail").unwrap() < success);
    assert!(path.metadata().unwrap().len() <= 20 * 1024 * 1024);
    let archive = path.with_extension("log.1");
    assert!(archive.metadata().unwrap().len() <= 20 * 1024 * 1024);
}

#[cfg(unix)]
#[test]
fn output_logging_failures_do_not_deadlock_a_child_with_full_pipes() {
    let temp = tempfile::tempdir().unwrap();
    let environment = action_environment(temp.path());
    let action = invocation(&[
        "sh", "-c",
        "while [ ! -f release ]; do sleep 0.01; done; i=0; while [ \"$i\" -lt 100 ]; do printf '%08000d' 0; printf '%08000d' 1 >&2; i=$((i + 1)); done",
    ], temp.path());
    let mut running = RunningPrAction::start(action, &environment, PrActionSet::Review).unwrap();
    let path = temp.path().join(".local/state/jx/jx-actions.log");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(temp.path().join("release"), "").unwrap();
    let failure = wait_for_action(&mut running).unwrap_err();
    assert!(failure.log_path.is_none());
    assert!(failure.message.contains("output capture failed"));
    assert!(failure.message.contains("logging failed"));
}

#[cfg(unix)]
#[test]
fn descendants_retaining_output_pipes_do_not_leave_capture_running_forever() {
    let temp = tempfile::tempdir().unwrap();
    let mut running = RunningPrAction::start(
        invocation(
            &["sh", "-c", "sleep 30 & printf 'background started'; exit 0"],
            temp.path(),
        ),
        &action_environment(temp.path()),
        PrActionSet::Review,
    )
    .unwrap();
    let failure = wait_for_action(&mut running).unwrap_err();
    let log = fs::read_to_string(failure.log_path.unwrap()).unwrap();
    assert!(log.contains("background started"));
    assert!(log.contains("action output did not close"));
    assert!(!log.contains("\"status\":\"success\""));
}

#[test]
fn action_loading_uses_selected_checkout_not_caller_and_external_prs_use_global_config() {
    let temp = tempfile::tempdir().unwrap();
    let caller = temp.path().join("caller");
    let selected = temp.path().join("selected");
    let global = temp.path().join(".config/jx");
    for root in [&caller, &selected] {
        fs::create_dir_all(root.join(".jj")).unwrap();
        fs::create_dir_all(root.join(".jx")).unwrap();
    }
    fs::create_dir_all(&global).unwrap();
    fs::write(
        global.join("actions.toml"),
        "[[repo.review_actions]]\nid='open'\ntitle='Global review'\ncommand=['review','{pr_url}']\ncwd='caller'\n[[repo.stack_status_actions]]\nid='open'\ntitle='Global stack'\ncommand=['stack','{pr_url}']\ncwd='caller'\n",
    )
    .unwrap();
    fs::write(
        caller.join(".jx/config.toml"),
        "[[repo.review_actions]]\nid='open'\ntitle='Wrong caller review'\ncommand=['wrong']\n[[repo.stack_status_actions]]\nid='open'\ntitle='Wrong caller stack'\ncommand=['wrong']\n",
    )
    .unwrap();
    let environment = RuntimeEnvironment::new(
        &caller,
        [("HOME".to_owned(), temp.path().display().to_string())],
    );
    let mut ctx = context(12, "owner/repo");
    for (set, title, program) in [
        (PrActionSet::Review, "Global review", "review"),
        (PrActionSet::StackStatus, "Global stack", "stack"),
    ] {
        let external = load_pr_actions(ctx.clone(), &environment, set).unwrap();
        assert_eq!(external.len(), 1);
        let action = external[0].prepared.as_ref().unwrap();
        assert_eq!(action.title, title);
        assert_eq!(action.command, [program, ctx.pr_url.as_str()]);
        assert_eq!(action.cwd, caller);
        assert!(!action.requires_confirmation());
    }
    ctx.repository_root = Some(selected.clone());
    for (local_set, other_set, key, global_title) in [
        (
            PrActionSet::Review,
            PrActionSet::StackStatus,
            "review_actions",
            "Global stack",
        ),
        (
            PrActionSet::StackStatus,
            PrActionSet::Review,
            "stack_status_actions",
            "Global review",
        ),
    ] {
        fs::write(
            selected.join(".jx/config.toml"),
            format!("[[repo.{key}]]\nid='open'\ntitle='Selected'\ncommand=['selected']\n"),
        )
        .unwrap();
        let local = load_pr_actions(ctx.clone(), &environment, local_set).unwrap();
        let action = local[0].prepared.as_ref().unwrap();
        assert_eq!(action.title, "Selected");
        assert_eq!(action.cwd, selected);
        assert!(action.requires_confirmation());
        let other = load_pr_actions(ctx.clone(), &environment, other_set).unwrap();
        let action = other[0].prepared.as_ref().unwrap();
        assert_eq!(action.title, global_title);
        assert_eq!(action.cwd, caller);
        assert!(!action.requires_confirmation());
    }
}
