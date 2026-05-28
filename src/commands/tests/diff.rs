use super::*;

#[test]
fn diff_runs_current_jj_diff_without_github_context() {
    // Verifies: Diff delegates to jj without requiring the GitHub workflow context.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "diff"], &environment, &services).expect("diff succeeds");

    assert_eq!(result.stdout, "diff: no_tests=false tool=plain\n");
}

#[test]
fn diff_accepts_revision_or_bookmark() {
    // Verifies: Diff forwards -r to the jj diff boundary without requiring GitHub context.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "diff", "-r", "example-user/current"],
        &environment,
        &services,
    )
    .expect("diff succeeds");

    assert_eq!(
        result.stdout,
        "diff: revision=example-user/current no_tests=false tool=plain\n"
    );
}

#[test]
fn diff_uses_configured_default_external_tool_and_appends_args() {
    // Verifies: Diff uses the jx default tool and appends operator args after config args.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".jx/config.toml",
        r#"
[diff]
default_tool = "difft"

[diff.tools.difft]
mode = "external"
command = "difft"
args = ["--color=always", "--display=side-by-side"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "diff", "-n", "--", "--display", "inline"],
        &environment,
        &services,
    )
    .expect("diff succeeds");

    assert_eq!(
            result.stdout,
            "diff: no_tests=true tool=external command=difft args=--color=always,--display=side-by-side,--display,inline\n"
        );
}

#[test]
fn diff_accepts_paths_before_trailing_tool_args() {
    // Verifies: Paths remain jj diff filters while `--` still separates renderer arguments.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".jx/config.toml",
        r#"
[diff]
default_tool = "difft"

[diff.tools.difft]
mode = "external"
command = "difft"
args = ["--color=always"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        [
            "jx",
            "diff",
            "src/main.rs",
            "README.md",
            "--",
            "--display",
            "inline",
        ],
        &environment,
        &services,
    )
    .expect("path-limited diff with tool args succeeds");

    assert_eq!(
        result.stdout,
        "diff: paths=src/main.rs,README.md no_tests=false tool=external command=difft args=--color=always,--display,inline\n"
    );
}

#[test]
fn diff_tool_flag_selects_configured_pipe_tool_and_appends_args() {
    // Verifies: Tool selection can choose a pipe renderer and append renderer args.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".jx/config.toml",
        r#"
[diff]
default_tool = "difft"

[diff.tools.difft]
mode = "external"
command = "difft"

[diff.tools.delta]
mode = "pipe"
producer_args = ["-w", "--git"]
command = "delta"
args = ["--features", "jj-diff"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "diff", "--tool", "delta", "--", "--line-numbers"],
        &environment,
        &services,
    )
    .expect("diff succeeds");

    assert_eq!(
            result.stdout,
            "diff: no_tests=false tool=pipe producer=-w,--git command=delta args=--features,jj-diff,--line-numbers\n"
        );
}

#[test]
fn diff_rejects_trailing_args_without_configured_tool() {
    // Verifies: Tool args are only accepted when jx owns a selected renderer.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let error = run_with_args_and_services(
        ["jx", "diff", "--", "--display", "inline"],
        &environment,
        &services,
    )
    .expect_err("tool args require a configured tool");

    assert!(matches!(error, CommandError::Usage(_)));
}

#[test]
fn diff_rejects_unknown_tool() {
    // Verifies: --tool selects configured jx tools instead of passing arbitrary jj tools through.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let error =
        run_with_args_and_services(["jx", "diff", "--tool", "delta"], &environment, &services)
            .expect_err("unknown tools are rejected");

    assert!(matches!(error, CommandError::Usage(_)));
}

#[test]
fn diff_accepts_file_arguments() {
    // Verifies: Diff forwards any number of path filters to the jj diff boundary.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "diff", "src/main.rs", "README.md"],
        &environment,
        &services,
    )
    .expect("path-limited diff succeeds");

    assert_eq!(
        result.stdout,
        "diff: paths=src/main.rs,README.md no_tests=false tool=plain\n"
    );
}
