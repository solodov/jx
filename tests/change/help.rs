//! Exercise rendered help as the public model-facing contract.

use super::*;

#[test]
fn every_help_page_is_self_contained_and_available_outside_a_repository() {
    for subcommand in [
        None,
        Some("new"),
        Some("read"),
        Some("search"),
        Some("apply"),
        Some("run"),
    ] {
        let help = help_text(subcommand);
        for required in [
            "--json",
            "Shared JSON contract:",
            "change_id:",
            "commit_id:",
            "operation_id:",
            "Operation errors:",
            "expected",
            "actual",
            "workspaces",
            "Exit statuses:",
            "130:",
            "plain stderr",
            "not bare jj",
        ] {
            assert!(
                help.contains(required),
                "{subcommand:?} help is missing {required:?}"
            );
        }
        for code in [
            "stale_change",
            "checked_out_changes",
            "divergent_change",
            "abandoned_change",
            "concurrent_operations",
            "repository_busy",
            "interrupted",
            "invalid_request",
            "jj_failed",
            "repository_error",
        ] {
            assert!(
                help.contains(&format!("{code}:")),
                "{subcommand:?} help omits {code}"
            );
        }
        assert!(
            !json_examples(&help).is_empty(),
            "help needs valid JSON examples"
        );
    }
}

#[test]
fn the_group_workflow_executes_using_only_the_documented_inputs() {
    let repo = example_repo();
    let help = help_text(None);
    let steps = command_examples(&help);
    assert_eq!(steps.len(), 4);
    let batch = json_examples(&help)
        .into_iter()
        .find(|example| example.get("operations").is_some())
        .unwrap();
    let mut change = String::new();
    let mut read_hash = String::new();
    let mut applied_hash = String::new();
    for (index, step) in steps.iter().enumerate() {
        let result = run_example(
            &repo,
            step,
            &[
                ("<CHANGE_ID>", &change),
                ("<READ_COMMIT_HASH>", &read_hash),
                ("<APPLIED_COMMIT_HASH>", &applied_hash),
            ],
            (index == 2).then_some(&batch),
        );
        match index {
            0 => change = change_id(&result).to_owned(),
            1 => {
                assert_eq!(result["content"], "before\n");
                read_hash = commit_id(&result).to_owned();
            }
            2 => {
                assert_eq!(result["changed"], true);
                assert!(result["conflicted_changes"].as_array().unwrap().is_empty());
                applied_hash = commit_id(&result).to_owned();
                assert_ne!(applied_hash, read_hash);
            }
            3 => {
                assert_eq!(result["exit_code"], 0);
                assert_eq!(commit_id(&result), applied_hash);
            }
            _ => unreachable!(),
        }
    }
    assert!(!repo.root.join("agent-note.txt").exists());
}

#[test]
fn subcommand_examples_match_their_json_response_schemas() {
    let repo = example_repo();
    let new_help = help_text(Some("new"));
    let created = run_example(&repo, &command_examples(&new_help)[0], &[], None);
    assert_result_schema(&new_help, &created);
    assert_eq!(created["changed"], true);

    let replacements = [
        ("<CHANGE_ID>", change_id(&created)),
        ("<COMMIT_HASH>", commit_id(&created)),
    ];
    let read_help = help_text(Some("read"));
    let read = run_example(&repo, &command_examples(&read_help)[0], &replacements, None);
    assert_result_schema(&read_help, &read);
    assert_eq!(read["content"], "before\n");
    assert_eq!(read["path"], "README.md");

    let search_help = help_text(Some("search"));
    for command in command_examples(&search_help) {
        let result = run_example(&repo, &command, &replacements, None);
        assert_result_schema(&search_help, &result);
        assert!(!result["output"].as_str().unwrap().is_empty());
    }

    let apply_help = help_text(Some("apply"));
    let batch = json_examples(&apply_help)
        .into_iter()
        .find(|example| example.get("operations").is_some())
        .unwrap();
    let applied = run_example(
        &repo,
        &command_examples(&apply_help)[0],
        &replacements,
        Some(&batch),
    );
    assert_result_schema(&apply_help, &applied);
    assert_eq!(repo.read(&applied, "README.md")["content"], "after\n");
    assert_eq!(repo.read(&applied, "agent-note.txt")["content"], "hello\n");
    assert_eq!(
        repo.read(&applied, "renamed.txt")["content"],
        "old content\n"
    );
    assert!(!repo
        .jj(&[
            "--ignore-working-copy",
            "file",
            "list",
            "-r",
            commit_id(&applied)
        ])
        .contains("obsolete.txt"));

    let run_help = help_text(Some("run"));
    for command in command_examples(&run_help) {
        let result = run_example(
            &repo,
            &command,
            &[("<COMMIT_HASH>", commit_id(&applied))],
            None,
        );
        assert_result_schema(&run_help, &result);
        assert_eq!(result["exit_code"], 0);
        if command.contains("printf") {
            assert_eq!(result["stdout"], "ok\n");
        }
    }

    let failed = repo.apply_output(&created, batch["operations"].clone());
    assert_eq!(failed.status.code(), Some(1));
    let error = json_output(failed);
    assert_eq!(error["error"]["code"], "stale_change");
    let example = json_examples(&apply_help)
        .into_iter()
        .find(|example| example.get("error").is_some())
        .unwrap();
    assert_schema(&example, &error);
}

#[test]
fn documented_pattern_kinds_no_match_and_native_error_behavior_are_accurate() {
    let repo = example_repo();
    let target = repo.new_change("@");
    for pattern in [
        "before",
        "regex:^before$",
        "substring:before",
        "exact:before",
        "glob:*fore",
        "regex-i:^BEFORE$",
        "substring-i:BEFORE",
        "exact-i:BEFORE",
        "glob-i:*FORE",
    ] {
        let result = repo.json(&[
            "change",
            "search",
            commit_id(&target),
            "--pattern",
            pattern,
            "--json",
        ]);
        assert!(
            result["output"]
                .as_str()
                .unwrap()
                .contains("README.md:1:before"),
            "{pattern}"
        );
    }
    let missing = repo.json(&[
        "change",
        "search",
        commit_id(&target),
        "--pattern",
        "substring:no-such-text",
        "--json",
    ]);
    assert_eq!(missing["output"], "");
    let failed = repo
        .command(env!("CARGO_BIN_EXE_jx"), &repo.root)
        .args([
            "change",
            "search",
            commit_id(&target),
            "--pattern",
            "invalid-kind:before",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert_eq!(json_output(failed)["error"]["code"], "jj_failed");
}

#[test]
fn documented_usage_and_failed_run_exit_statuses_match_the_cli() {
    let empty = tempfile::tempdir().unwrap();
    let usage = Command::new(env!("CARGO_BIN_EXE_jx"))
        .current_dir(empty.path())
        .args(["change", "apply", "--json"])
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(2));
    assert!(usage.stdout.is_empty());
    assert!(!usage.stderr.is_empty());

    let repo = example_repo();
    let target = repo.new_change("@");
    let failed = repo
        .command(env!("CARGO_BIN_EXE_jx"), &repo.root)
        .args([
            "change",
            "run",
            commit_id(&target),
            "--json",
            "--",
            "sh",
            "-c",
            "exit 23",
        ])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    let result = json_output(failed);
    assert_eq!(result["exit_code"], 1);
    assert!(result.get("error").is_none());
    assert_result_schema(&help_text(Some("run")), &result);
}

fn help_text(subcommand: Option<&str>) -> String {
    let empty = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_jx"));
    command
        .current_dir(empty.path())
        .env("HOME", empty.path())
        .env("XDG_STATE_HOME", empty.path().join("state"))
        .env("XDG_CONFIG_HOME", empty.path().join("config"))
        .arg("change");
    if let Some(subcommand) = subcommand {
        command.arg(subcommand);
    }
    let output = command.arg("--help").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout).unwrap()
}

fn example_repo() -> TestRepo {
    let repo = TestRepo::new();
    for (path, content) in [
        ("README.md", "before\n"),
        ("old.txt", "old content\n"),
        ("obsolete.txt", "obsolete\n"),
        ("checks.txt", "TODO: validate_input\n"),
    ] {
        fs::write(repo.root.join(path), content).unwrap();
    }
    repo.jj(&["describe", "-m", "help example base"]);
    repo
}

fn command_examples(help: &str) -> Vec<String> {
    help.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("jx change "))
        .map(str::to_owned)
        .collect()
}

fn json_examples(help: &str) -> Vec<Value> {
    help.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('{'))
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("invalid JSON help example {line:?}: {error}"))
        })
        .collect()
}

fn run_example(
    repo: &TestRepo,
    line: &str,
    replacements: &[(&str, &str)],
    input: Option<&Value>,
) -> Value {
    let mut script = line.strip_prefix("jx ").unwrap().to_owned();
    for (placeholder, value) in replacements {
        if script.contains(placeholder) {
            assert!(!value.is_empty(), "missing {placeholder}");
        }
        script = script.replace(placeholder, value);
    }
    assert!(!script.contains('<'), "unsubstituted example: {script}");
    let executable = env!("CARGO_BIN_EXE_jx").replace('\'', "'\\''");
    let mut child = repo
        .command("sh", &repo.root)
        .args(["-c", &format!("exec '{executable}' {script}")])
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        serde_json::to_writer(child.stdin.take().unwrap(), input).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "example failed: {line}\n{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    json_output(output)
}

fn assert_result_schema(help: &str, result: &Value) {
    let example = json_examples(help)
        .into_iter()
        .find(|example| example.get("snapshot").is_some())
        .unwrap();
    assert_schema(&example, result);
}

fn assert_schema(example: &Value, actual: &Value) {
    match example {
        Value::Object(fields) => {
            let object = actual.as_object().expect("expected JSON object");
            assert_eq!(
                fields.len(),
                object.len(),
                "response fields differ from help: {actual}"
            );
            for (key, value) in fields {
                assert_schema(
                    value,
                    object
                        .get(key)
                        .unwrap_or_else(|| panic!("missing {key}: {actual}")),
                );
            }
        }
        Value::Array(_) => assert!(actual.is_array()),
        Value::String(_) => assert!(actual.is_string()),
        Value::Bool(_) => assert!(actual.is_boolean()),
        Value::Number(_) => assert!(actual.is_number()),
        Value::Null => assert!(actual.is_null()),
    }
}
