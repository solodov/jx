use super::*;

#[test]
fn log_subcommand_renders_workspace_log() {
    // Verifies: Integrations can call the compact log renderer explicitly.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "log"], &environment, &services).expect("log succeeds");

    assert_eq!(result.stdout, "workspace log\n");
}

#[test]
fn no_args_uses_default_log_command() {
    // Verifies: No-argument invocation keeps log as the built-in default command.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx"], &environment, &services).expect("log succeeds");

    assert_eq!(result.stdout, "workspace log\n");
}

#[test]
fn no_args_uses_configured_default_command() {
    // Verifies: ui.default_command controls bare `jx` without changing explicit commands.
    let workspace = TestWorkspace::new();
    workspace.write_file(".jx/config.toml", "[ui]\ndefault_command = \"status\"\n");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx"], &environment, &services)
        .expect("configured default succeeds");

    assert_eq!(result.stdout, expected_workspace_status());
    assert!(services.workspace_log_annotations.borrow().is_empty());
}

#[test]
fn no_args_reports_invalid_configured_default_command() {
    // Verifies: Bad ui.default_command values point operators back to config.
    let workspace = TestWorkspace::new();
    workspace.write_file(".jx/config.toml", "[ui]\ndefault_command = \"missing\"\n");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx"], &environment, &services)
        .expect_err("configured default is rejected");

    assert!(matches!(
        error,
        CommandError::DefaultCommand { ref command, ref message }
            if command == "missing" && message.contains("missing")
    ));
}

#[test]
fn no_args_passes_stack_pull_request_annotations_to_workspace_log() {
    // Verifies: Default log rendering can link local PR bookmarks without contacting GitHub.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let root = workspace.path();
    write_stack_metadata(
        &root,
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![StackMetadataNode {
                branch: "topic/current".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(42),
                parent_pull_request: None,
                title: "Current".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(root, []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx"], &environment, &services).expect("log succeeds");

    assert_eq!(result.stdout, "workspace log\n");
    assert_eq!(
        services.workspace_log_annotations.borrow().as_slice(),
        [vec![LogBookmarkAnnotation {
            bookmark: "topic/current".to_owned(),
            label: "#42".to_owned(),
            url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
        }]]
    );
}

#[test]
fn status_renders_shared_commit_status_without_github_context() {
    // Verifies: Status shows jj's commit summary, description, and file summary without origin.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "status"], &environment, &services)
        .expect("status succeeds");

    assert_eq!(result.stdout, expected_workspace_status());
}

#[test]
fn st_alias_runs_status() {
    // Verifies: The short status alias uses the same shared renderer.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "st"], &environment, &services)
        .expect("status alias succeeds");

    assert_eq!(result.stdout, expected_workspace_status());
}

#[test]
fn status_revision_passes_selected_revision_to_workspace_status() {
    // Verifies: status -r selects a specific revision without changing rendering ownership.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "status", "--revision", "example-user/topic"],
        &environment,
        &services,
    )
    .expect("status revision succeeds");

    assert_eq!(result.stdout, expected_workspace_status());
    assert_eq!(
        services.workspace_status_requests.borrow().as_slice(),
        [Some("example-user/topic".to_owned())]
    );
}

#[test]
fn command_run_perf_span_records_successful_commands() {
    // Verifies: top-level tracing captures the parsed command and dispatch phases.
    let workspace = TestWorkspace::new();
    let log_path = workspace.home.join("jx-perf.log");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("JX_PERF_LOG".to_owned(), log_path.display().to_string())],
    );
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "status"], &environment, &services)
        .expect("status succeeds");

    assert_eq!(result.stdout, expected_workspace_status());
    let events = read_perf_events(&log_path);
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event["op"], "command.run");
    assert_eq!(event["status"], "ok");
    assert_eq!(event["command"], "status");
    assert_eq!(event["command_path"], "status");
    assert_eq!(event["arg_count"], 1);
    assert_eq!(event["exit_code"], 0);
    assert_eq!(
        perf_step_names(event),
        ["parse_args", "build_request", "handle_request"]
    );
}

#[test]
fn command_run_perf_span_records_command_errors() {
    // Verifies: failed dispatch still emits command identity, exit code, and error text.
    let workspace = TestWorkspace::new();
    let log_path = workspace.home.join("jx-perf.log");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("JX_PERF_LOG".to_owned(), log_path.display().to_string())],
    );
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx", "fetch"], &environment, &services)
        .expect_err("fetch needs an origin remote");

    assert!(matches!(error, CommandError::Repository(_)));
    let events = read_perf_events(&log_path);
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event["op"], "command.run");
    assert_eq!(event["status"], "error");
    assert_eq!(event["command"], "fetch");
    assert_eq!(event["command_path"], "fetch");
    assert_eq!(event["exit_code"], 1);
    assert!(event["err"]
        .as_str()
        .expect("error text is recorded")
        .contains("fixed `origin` remote is missing"));
}

fn read_perf_events(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .expect("perf log is written")
        .lines()
        .map(|line| serde_json::from_str(line).expect("perf event is json"))
        .collect()
}

fn perf_step_names(event: &serde_json::Value) -> Vec<&str> {
    event["steps"]
        .as_array()
        .expect("steps are recorded")
        .iter()
        .map(|step| step["name"].as_str().expect("step name is a string"))
        .collect()
}

#[test]
fn prev_commit_renders_navigation_graph() {
    // Verifies: Commit navigation replaces jj's edit/status output with the focused graph.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "prev-commit"], &environment, &services)
        .expect("previous commit navigation succeeds");

    assert_eq!(result.stdout, "previous commit graph\n");
}

#[test]
fn next_alias_renders_navigation_graph() {
    // Verifies: The short next alias reaches the same focused graph renderer.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "next"], &environment, &services)
        .expect("next commit navigation succeeds");

    assert_eq!(result.stdout, "next commit graph\n");
}

#[test]
fn check_loads_context_and_renders_readiness_summary() {
    // Verifies: Check loads repository context and renders the readiness summary.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "check"], &environment, &services)
        .expect("check succeeds");

    assert_eq!(
            result.stdout,
            format!(
                "ready to publish\nrepo: example-owner/example-repo\nchange: a1b2c3d4, non-empty\nbookmark: {}, will create\ngithub: example-user, can push\nreviewers: none\n",
                example_bookmark_link("example-user/02-zzzzzzzz")
            )
        );
}

#[test]
fn task_id_is_rejected_for_commands_that_do_not_use_bookmarks() {
    // Verifies: Task ID is rejected for commands that do not use bookmarks.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let error = run_with_args(["jx", "check", "--task-id", "ABC-123"], &environment)
        .expect_err("check has no task id option");

    assert!(matches!(error, CommandError::Usage(_)));
}

#[cfg(unix)]
#[test]
fn production_services_builds_octocrab_inside_tokio_runtime() {
    // Verifies: Production services initialize Octocrab inside a Tokio runtime.
    let environment = RuntimeEnvironment::new(
        "/workspace",
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = ProductionServices::new(&environment).expect("runtime builds");

    let _client = services
        .github_runtime
        .block_on(async {
            OctocrabGitHubClient::from_token_source(
                &crate::repository::TokenSource::Environment("GH_TOKEN"),
                &environment,
            )
        })
        .expect("client builds inside runtime");
}
