use super::*;

#[test]
fn no_args_renders_workspace_scoped_log() {
    // Verifies: No-argument invocation renders the workspace-scoped log.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx"], &environment, &services).expect("log succeeds");

    assert_eq!(result.stdout, "workspace log\n");
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
