use super::*;

#[test]
fn pull_request_runs_matching_repo_check_before_publish() {
    // Verifies: PR publishing runs matching check commands before mutating bookmarks or GitHub state.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_remote_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.checks]]
id = "source-check"
before = ["pull_request"]
paths = ["src/**"]
command = ["./scripts/check-source"]
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();

    run_with_args_and_services(
        ["jx", "stack", "publish", "-t", "ABC-123"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    let calls = services.check_command_calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "source-check");
    assert_eq!(calls[0].command, vec!["./scripts/check-source".to_owned()]);
}

#[test]
fn push_skips_repo_check_when_changed_files_do_not_match() {
    // Verifies: Checks are path-filtered by repo-root-relative changed files.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_remote_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.checks]]
id = "docs"
before = ["push"]
paths = ["docs/**"]
command = ["./check-docs"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    run_with_args_and_services(["jx", "push", "--tracked"], &environment, &services)
        .expect("tracked push succeeds");

    assert!(services.check_command_calls.borrow().is_empty());
    assert_eq!(services.push_tracked_roots.borrow().len(), 1);
}

#[test]
fn push_aborts_when_repo_check_fails() {
    // Verifies: Failing checks surface captured output and stop before push mutation.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_remote_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.checks]]
id = "source-check"
before = ["push"]
paths = ["src/**"]
command = ["./scripts/check-source"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        check_command_outputs: std::cell::RefCell::new(vec![CheckCommandOutput::failure(
            "exit code 1",
            "source check failed\nrun ./scripts/fix-source",
        )]),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "push", "--tracked"], &environment, &services)
        .expect_err("check failure aborts push");

    assert_eq!(
        error.to_string(),
        "check `source-check` failed before push\n\n  source check failed\n  run ./scripts/fix-source"
    );
    assert!(services.push_tracked_roots.borrow().is_empty());
}

#[test]
fn sync_runs_repo_check_when_current_working_copy_file_matches() {
    // Verifies: Sync checks include dirty current-work files even when no tracked push changes match.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_remote_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.checks]]
id = "dictionary-check"
before = ["sync"]
paths = ["dotfiles/config/aspell/.aspell.en.pws"]
command = ["./check-dictionaries"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut current = workspace_facts();
    current.changed_files = vec!["dotfiles/config/aspell/.aspell.en.pws".to_owned()];
    let services = FakeServices {
        workspace: current,
        tracked_changed_files: Vec::new(),
        check_command_outputs: std::cell::RefCell::new(vec![CheckCommandOutput::failure(
            "exit code 1",
            "dictionaries differ\nrun ./check-dictionaries",
        )]),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect_err("current working-copy check failure aborts sync");

    assert_eq!(
        error.to_string(),
        "check `dictionary-check` failed before sync\n\n  dictionaries differ\n  run ./check-dictionaries"
    );
    assert_eq!(services.check_command_calls.borrow().len(), 1);
    assert!(services.fetch_origin_roots.borrow().is_empty());
}

#[test]
fn sync_aborts_when_repo_check_modifies_working_copy() {
    // Verifies: Successful checks are still rejected if jj observes a changed working-copy commit.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_remote_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.checks]]
id = "source-check"
before = ["sync"]
paths = ["src/**"]
command = ["./scripts/check-source"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        check_snapshots: std::cell::RefCell::new(vec![snapshot("before"), snapshot("after")]),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect_err("working-copy mutation aborts sync");

    assert!(error
        .to_string()
        .contains("check `source-check` modified the working copy before sync"));
    assert!(services.fetch_origin_roots.borrow().is_empty());
}

fn snapshot(commit_id: &str) -> WorkingCopySnapshot {
    WorkingCopySnapshot {
        commit_id: commit_id.to_owned(),
    }
}

fn origin_remote_config() -> &'static str {
    r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#
}
