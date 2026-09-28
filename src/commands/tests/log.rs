use super::*;

#[test]
fn explicit_and_default_logs_record_annotations_and_rendering_without_changing_output() {
    for args in [vec!["jx", "log"], vec!["jx"]] {
        let workspace = TestWorkspace::new();
        let log_path = workspace.home.join("log-perf.jsonl");
        let environment = log_environment(&workspace, &log_path);
        let services = FakeServices::default();

        let result = run_with_args_and_services(args, &environment, &services).unwrap();

        assert_eq!(result.stdout, "workspace log\n");
        let events = read_log_perf_events(&log_path);
        let log = event(&events, "log.run");
        assert_eq!(log["status"], "ok");
        assert_eq!(log["current_dir"], workspace.path().display().to_string());
        assert_eq!(log["annotation_count"], 0);
        assert_eq!(log["steps"][0]["name"], "load_annotations");
        assert_eq!(log["steps"][1]["name"], "workspace_log");
        assert_eq!(log["steps"][1]["output_bytes"], result.stdout.len());
        assert_eq!(
            event(&events, "command.run")["current_dir"],
            workspace.path().display().to_string()
        );
    }
}

#[test]
fn production_log_records_jj_phases_and_workspace_context() {
    if !ProcessCommand::new("jj")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        eprintln!("skipping production log timing test because jj CLI is unavailable");
        return;
    }
    let workspace = TestWorkspace::new();
    let log_path = workspace.home.join("log-perf.jsonl");
    let environment = log_environment(&workspace, &log_path);
    let services = ProductionServices::new(&environment).unwrap();

    let result = run_with_args_and_services(["jx", "log"], &environment, &services).unwrap();

    assert!(!result.stdout.is_empty());
    let events = read_log_perf_events(&log_path);
    let log = event(&events, "log.run");
    assert_eq!(log["status"], "ok");
    assert_eq!(
        log["workspace_root"],
        workspace.path().display().to_string()
    );
    assert!(log["immutable_commit_count"]
        .as_u64()
        .is_some_and(|count| count > 0));
    let names = log["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| {
            assert!(step["duration_us"].as_u64().is_some());
            assert!(step.get("err").is_none());
            step["name"].as_str().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "load_annotations",
            "workspace_log",
            "find_workspace",
            "snapshot_working_copy",
            "load_repository",
            "evaluate_log_revset",
            "immutable_history",
            "prepare_templates",
            "render_graph"
        ]
    );
}

#[test]
fn log_errors_keep_partial_phase_timings_and_original_error() {
    let workspace = TestWorkspace::new_uninitialized_under("");
    let log_path = workspace.home.join("log-perf.jsonl");
    let environment = log_environment(&workspace, &log_path);
    let services = ProductionServices::new(&environment).unwrap();

    let error = run_with_args_and_services(["jx", "log"], &environment, &services).unwrap_err();

    let events = read_log_perf_events(&log_path);
    let log = event(&events, "log.run");
    assert_eq!(log["status"], "error");
    assert_eq!(log["err"], error.to_string());
    assert_eq!(log["steps"][0]["name"], "load_annotations");
    assert_eq!(log["steps"][2]["name"], "find_workspace");
    assert_eq!(log["steps"][2]["err"], log["steps"][1]["err"]);
    assert!(log["steps"][2]["err"]
        .as_str()
        .unwrap()
        .contains("No jj workspace found"));
    assert_eq!(event(&events, "command.run")["status"], "error");
}

#[test]
fn disabled_or_unwritable_tracing_does_not_change_log_output() {
    let workspace = TestWorkspace::new();
    for destination in ["off".to_owned(), workspace.home.display().to_string()] {
        let environment = RuntimeEnvironment::new(
            workspace.path(),
            [
                ("HOME".to_owned(), workspace.home.display().to_string()),
                ("JX_PERF_LOG".to_owned(), destination),
            ],
        );
        let result =
            run_with_args_and_services(["jx", "log"], &environment, &FakeServices::default())
                .unwrap();
        assert_eq!(result.stdout, "workspace log\n");
    }
}

fn log_environment(workspace: &TestWorkspace, log_path: &Path) -> RuntimeEnvironment {
    RuntimeEnvironment::new(
        workspace.path(),
        [
            ("HOME".to_owned(), workspace.home.display().to_string()),
            ("JX_PERF_LOG".to_owned(), log_path.display().to_string()),
        ],
    )
}

fn read_log_perf_events(path: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn event<'a>(events: &'a [serde_json::Value], op: &str) -> &'a serde_json::Value {
    events.iter().find(|event| event["op"] == op).unwrap()
}
