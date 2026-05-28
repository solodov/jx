use super::*;

#[test]
fn shell_init_bash_emits_configured_navigation_with_completion() {
    // Verifies: Shell init includes work navigation, cached completion, and auto zoxide fallback.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[shell]
navigation = "u"
zoxide = "auto"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "shell", "init", "bash"], &environment, &services)
            .expect("shell init succeeds");

    assert!(result.stdout.contains("complete -F _jx"));
    assert!(result
        .stdout
        .contains("complete -F __jx_project_arg_completion jx"));
    assert!(result.stdout.contains("_jx \"$@\""));
    assert!(result.stdout.contains("remote-status"));
    assert!(result.stdout.contains("--changed"));
    assert!(result
        .stdout
        .contains("command jx work complete --repositories --prefix \"$cur\""));
    assert!(result
        .stdout
        .contains("command jx work complete --workspaces --prefix \"$cur\""));
    assert!(result.stdout.contains("if [[ \"$cur\" == -* ]]; then"));
    assert!(result.stdout.contains("u() {"));
    assert!(result.stdout.contains("__jx_u_path_like"));
    assert!(result.stdout.contains("compgen -d -- \"$cur\""));
    assert!(result
        .stdout
        .contains("command jx work root --navigation \"$1\""));
    assert!(result.stdout.contains("\"$2\" == \"trunk\""));
    assert!(result.stdout.contains("\"$2\" == \"delete\""));
    assert!(result
        .stdout
        .contains("if (( status == 0 )) && [[ -n \"$cd_target\" ]]"));
    assert!(result
        .stdout
        .contains("command jx work complete --navigation --prefix \"\""));
    assert!(result.stdout.contains("command -v zoxide"));
    assert!(result.stdout.contains("zoxide query \"$@\""));
    assert!(result
        .stdout
        .contains("complete -o nospace -F __jx_u_completion u"));
}

#[test]
fn shell_init_bash_can_prefer_zoxide_navigation() {
    // Verifies: Prefer mode uses zoxide before jx lookup while reserving jj aliases for jx.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[shell]
navigation = "u"
zoxide = "prefer"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "shell", "init", "bash"], &environment, &services)
            .expect("shell init succeeds");

    assert!(result.stdout.contains("__jx_u_jx_first_key"));
    assert!(result.stdout.contains(
        "__jx_u_jx_first_key \"$1\" && target=\"$(command jx work root --navigation \"$1\""
    ));
    let zoxide_lookup = result
        .stdout
        .find("__jx_u_zoxide_enabled && target=\"$(zoxide query \"$@\"")
        .expect("prefer mode includes zoxide lookup");
    let jx_fallback = result
        .stdout
        .rfind("elif (( $# == 1 )) && target=\"$(command jx work root --navigation \"$1\"")
        .expect("prefer mode includes jx fallback");
    assert!(zoxide_lookup < jx_fallback, "{}", result.stdout);
    assert!(result
        .stdout
        .contains("__jx_u_add_zoxide_candidates \"$cur\""));
    assert!(result
        .stdout
        .contains("if (( ${#candidates[@]} > 0 )); then"));
}

#[test]
fn shell_init_bash_emits_zellij_tab_navigation_when_configured() {
    // Verifies: Tab navigation reuses `u` resolution while keeping zellij as the only tab opener.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[shell]
navigation = "u"
navigation_tab = "ut"
zoxide = "auto"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "shell", "init", "bash"], &environment, &services)
            .expect("shell init succeeds");

    assert!(result.stdout.contains("u() {"));
    assert!(result.stdout.contains("ut() {"));
    assert!(result
        .stdout
        .contains("__jx_u_resolve_and_navigate tab \"$@\""));
    assert!(result
        .stdout
        .contains("zellij action new-tab --cwd \"$1\" >/dev/null"));
    assert!(result
        .stdout
        .contains("opening tabs is only supported inside zellij"));
    assert!(result
        .stdout
        .contains("complete -o nospace -F __jx_u_completion u ut"));
}

#[test]
fn shell_init_bash_can_disable_zoxide_fallback() {
    // Verifies: zoxide integration is config-enabled and omitted when disabled.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[shell]
navigation = "u"
zoxide = "never"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "shell", "init", "bash"], &environment, &services)
            .expect("shell init succeeds");

    assert!(result.stdout.contains("u() {"));
    assert!(!result.stdout.contains("zoxide"));
}

#[test]
fn shell_init_bash_emits_cli_completion_without_navigation_when_unconfigured() {
    // Verifies: CLI completion is default while navigation remains an explicit user preference.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "shell", "init", "bash"], &environment, &services)
            .expect("shell init succeeds");

    assert!(result.stdout.contains("complete -F _jx"));
    assert!(result.stdout.contains("remote-status"));
    assert!(result.stdout.contains("open"));
    assert!(result.stdout.contains("--shell-cd-target"));
    assert!(result.stdout.contains("\"$2\" == \"add\""));
    assert!(result.stdout.contains("\"$2\" == \"trunk\""));
    assert!(result.stdout.contains("\"$2\" == \"delete\""));
    assert!(result.stdout.contains("pushd \"$cd_target\""));
    assert!(result.stdout.contains("cd \"$cd_target\""));
    assert!(!result.stdout.contains("u() {"));
}

#[test]
fn shell_init_rejects_invalid_navigation_tab_function_name() {
    // Verifies: Tab navigation command names are also sanitized before shell interpolation.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[shell]
navigation = "u"
navigation_tab = "bad-name"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error =
        run_with_args_and_services(["jx", "shell", "init", "bash"], &environment, &services)
            .expect_err("invalid tab command names are rejected");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::InvalidConfig { .. })
    ));
}

#[test]
fn shell_init_rejects_tab_navigation_without_navigation() {
    // Verifies: Tab navigation is a companion to the configured current-shell navigator.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[shell]
navigation_tab = "ut"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error =
        run_with_args_and_services(["jx", "shell", "init", "bash"], &environment, &services)
            .expect_err("tab navigation requires normal navigation");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::InvalidConfig { .. })
    ));
}

#[test]
fn shell_init_rejects_invalid_navigation_function_name() {
    // Verifies: Generated shell snippets only interpolate safe function names.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[shell]
navigation = "bad-name"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error =
        run_with_args_and_services(["jx", "shell", "init", "bash"], &environment, &services)
            .expect_err("invalid shell command names are rejected");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::InvalidConfig { .. })
    ));
}
