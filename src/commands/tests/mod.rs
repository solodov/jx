use super::*;
use crate::{
    domain::{
        BookmarkAction, BookmarkPlan, CheckWorkspaceSummary, GitHubReadiness, PullRequestAction,
        RepositorySummary, StatusComparison, StatusState,
    },
    github::{PullRequestHead, PullRequestRecord, ReviewerSelection},
    jj::{
        ChangeSummary, PushedBookmarkSummary, PushedCommitSummary, RebaseOnTrunkOutcome,
        RebasedCommitSummary, StatusRemoteFacts, StatusWorkspaceFacts, TrackedPushOutcome,
        TrunkSummary, WorkspaceAddOptions, WorkspaceEntry, WorkspaceRemoveOptions, WorkspaceStatus,
        WorkspaceVisibility,
    },
};
use jj_lib::{
    config::StackedConfig,
    git,
    ref_name::RemoteName,
    repo::StoreFactories,
    settings::UserSettings,
    workspace::{default_working_copy_factories, Workspace},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

fn visible_in(workspaces: &[&str], includes_current: bool) -> WorkspaceVisibility {
    WorkspaceVisibility {
        names: workspaces
            .iter()
            .map(|workspace| (*workspace).to_owned())
            .collect(),
        includes_current,
    }
}

fn current_workspace_visibility() -> WorkspaceVisibility {
    visible_in(&["default"], true)
}

fn example_bookmark_link(bookmark: &str) -> String {
    linked_bookmark_text("https://github.com/example-owner/example-repo", bookmark)
}

fn example_pull_request_link(number: u64) -> String {
    osc8_link(
        &format!("https://github.com/example-owner/example-repo/pull/{number}"),
        &format!("#{number}"),
    )
}

#[derive(Default)]
struct RecordingProgress {
    messages: std::cell::RefCell<Vec<String>>,
    finished: std::cell::Cell<bool>,
}

impl RecordingProgress {
    fn messages(&self) -> Vec<String> {
        self.messages.borrow().clone()
    }
}

impl ProgressSink for RecordingProgress {
    fn status(&self, message: &str) {
        self.messages.borrow_mut().push(message.to_owned());
    }

    fn finish(&self) {
        self.finished.set(true);
    }
}

#[test]
fn no_args_renders_workspace_scoped_log() {
    // Verifies: No-argument invocation renders the workspace-scoped log.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx"], &environment, &services).expect("log succeeds");

    assert_eq!(result.stdout, "workspace log\n");
}

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
fn clone_uses_owner_rule_and_default_source_shorthand() {
    // Verifies: Clone resolves owner/repo shorthands through layout rules before invoking jj.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[layout]
default_root = "~/src"

[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.path().join("work/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "clone", "example-owner/example-repo"],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/work/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            )
        )
    );
}

#[test]
fn clone_uses_default_layout_for_unmatched_github_repos() {
    // Verifies: Unmatched repositories stay globally discoverable under the default root.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace
        .path()
        .join("src/github.com/example-owner/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "clone", "example-owner/example-repo"],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/src/github.com/example-owner/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            )
        )
    );
}

#[test]
fn clone_accepts_host_owner_repo_form() {
    // Verifies: Explicit host input still uses the matching source and layout rules.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.path().join("work/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "clone", "github.com/example-owner/example-repo"],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/work/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            )
        )
    );
}

#[test]
fn clone_uses_configured_clone_url_format() {
    // Verifies: Source config owns generated clone URL shape for shorthand inputs.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[layout.sources]]
name = "github"
provider = "github"
host = "github.com"
clone_url = "https"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace
        .path()
        .join("src/github.com/example-owner/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "https://github.com/example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "clone", "example-owner/example-repo"],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/src/github.com/example-owner/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "https://github.com/example-owner/example-repo.git"
            )
        )
    );
}

#[test]
fn clone_preserves_explicit_url_but_uses_layout_destination() {
    // Verifies: Explicit URLs decide clone transport while normalized identity decides placement.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.path().join("work/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "https://github.com/example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        [
            "jx",
            "clone",
            "https://github.com/example-owner/example-repo.git",
        ],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/work/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "https://github.com/example-owner/example-repo.git"
            )
        )
    );
}

#[test]
fn work_add_uses_hidden_layout_workspace_path() {
    // Verifies: Add creates jj workspaces under the configured hidden `.work` layout.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: Some("main".to_owned()),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "add", "fix", "-r", "main"],
        &environment,
        &services,
    )
    .expect("workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!("Added workspace: {}\n", expected_destination.display())
    );
}

#[test]
fn work_add_task_id_prefixes_workspace_name_and_writes_metadata() {
    // Verifies: Task workspaces use task-visible names while metadata remains the source of truth.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/ABC-123-fix");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "ABC-123-fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "add", "fix", "--task-id", "ABC-123"],
        &environment,
        &services,
    )
    .expect("task workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!("Added workspace: {}\n", expected_destination.display())
    );
    assert_eq!(
        read_workspace_metadata(&expected_destination).expect("metadata reads"),
        WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
        }
    );
    assert_eq!(
        fs::read_to_string(expected_destination.join(".jx/.gitignore")).expect("gitignore"),
        "*\n"
    );
}

#[test]
fn work_add_rejects_invalid_workspace_name() {
    // Verifies: Workspace names are validated before they become filesystem paths.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error =
        run_with_args_and_services(["jx", "work", "add", "bad/name"], &environment, &services)
            .expect_err("invalid workspace names are rejected");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::InvalidWorkspaceName { .. })
    ));
}

#[test]
fn work_list_marks_current_workspace_and_aligns_paths() {
    // Verifies: List renders jj workspaces as concise name/path rows with `@` on current.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "default".to_owned(),
                root: PathBuf::from("/Users/example/projects/jx"),
                is_current: true,
            },
            WorkspaceEntry {
                name: "fix".to_owned(),
                root: PathBuf::from("/Users/example/projects/.work/jx/fix"),
                is_current: false,
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "work", "list"], &environment, &services)
        .expect("workspace list succeeds");

    assert_eq!(
        result.stdout,
        "default@  /Users/example/projects/jx\nfix       /Users/example/projects/.work/jx/fix\n"
    );
}

#[test]
fn work_complete_lists_global_repositories_and_workspaces() {
    // Verifies: Completion scans configured layout roots and orders each repo before workspaces.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let project_root = workspace.home.join("projects/project");
    create_jj_workspace_marker(&project_root);
    create_jj_workspace_marker(&workspace.home.join("projects/.work/project/fix"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/project/review"));
    create_jj_workspace_marker(&workspace.home.join("projects/other"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--prefix", "p"],
        &environment,
        &services,
    )
    .expect("work completion succeeds");

    assert_eq!(result.stdout, "project\nproject@fix\nproject@review\n");
}

#[test]
fn work_complete_can_list_only_primary_repositories() {
    // Verifies: Project-argument shell completion omits secondary workspaces.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    create_jj_workspace_marker(&workspace.home.join("projects/project"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/project/fix"));
    create_jj_workspace_marker(&workspace.home.join("projects/other"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--repositories", "--prefix", "p"],
        &environment,
        &services,
    )
    .expect("repository completion succeeds");

    assert_eq!(result.stdout, "project\n");
}

#[test]
fn work_root_resolves_global_workspace_key() {
    // Verifies: Shell integration can resolve a global repo@workspace key to a path-only result.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let expected_root = workspace.home.join("projects/.work/project/fix");
    create_jj_workspace_marker(&workspace.home.join("projects/project"));
    create_jj_workspace_marker(&expected_root);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "root", "project@fix"],
        &environment,
        &services,
    )
    .expect("work root succeeds");

    assert_eq!(result.stdout, format!("{}\n", expected_root.display()));
}

#[test]
fn work_complete_qualifies_colliding_repository_names() {
    // Verifies: Global keys stay deterministic when two configured repos share a basename.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[layout.default]
path = "{owner}/{repo}"

[[layout.rules]]
source = "github"
root = "~/projects"
path = "{owner}/{repo}"
owner = "alpha"

[[layout.rules]]
source = "github"
root = "~/projects"
path = "{owner}/{repo}"
owner = "beta"
"#,
    );
    create_jj_workspace_marker(&workspace.home.join("projects/alpha/tool"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/alpha/tool/fix"));
    create_jj_workspace_marker(&workspace.home.join("projects/beta/tool"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--prefix", ""],
        &environment,
        &services,
    )
    .expect("work completion succeeds");

    assert_eq!(result.stdout, "alpha/tool\nalpha/tool@fix\nbeta/tool\n");
}

#[test]
fn work_list_all_renders_global_keys_and_paths() {
    // Verifies: The global work list exposes the same keys that shell completion resolves.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let project_root = workspace.home.join("projects/project");
    let workspace_root = workspace.home.join("projects/.work/project/fix");
    create_jj_workspace_marker(&project_root);
    create_jj_workspace_marker(&workspace_root);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "list", "--all", "--prefix", "project"],
        &environment,
        &services,
    )
    .expect("global work list succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "project      {}\nproject@fix  {}\n",
            project_root.display(),
            workspace_root.display()
        )
    );
}

#[test]
fn shell_init_bash_emits_configured_navigation_with_completion() {
    // Verifies: Shell init includes work navigation, completion, and auto zoxide fallback.
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
    assert!(result.stdout.contains("u() {"));
    assert!(result.stdout.contains("command jx work root \"$1\""));
    assert!(result
        .stdout
        .contains("command jx work complete --prefix \"$cur\""));
    assert!(result.stdout.contains("command -v zoxide"));
    assert!(result.stdout.contains("zoxide query \"$@\""));
    assert!(result
        .stdout
        .contains("complete -o nospace -F __jx_u_completion u"));
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
    assert!(!result.stdout.contains("u() {"));
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

#[test]
fn work_remove_can_be_cancelled() {
    // Verifies: Declining removal stops before jj forget or filesystem deletion.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: project_workspaces(&workspace),
        ..FakeServices::default()
    };
    let confirmer = FixedWorkspaceRemoveConfirmer { confirmed: false };

    let result = run_with_args_and_workspace_remove_confirmer(
        ["jx", "work", "remove", "fix"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("workspace removal cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
    assert!(services.workspace_removes.borrow().is_empty());
}

#[test]
fn work_remove_forgets_and_deletes_managed_workspace_after_confirmation() {
    // Verifies: Removal delegates one confirmed forget/delete operation for managed workspaces.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: project_workspaces(&workspace),
        ..FakeServices::default()
    };
    let confirmer = FixedWorkspaceRemoveConfirmer { confirmed: true };

    let result = run_with_args_and_workspace_remove_confirmer(
        ["jx", "work", "remove", "fix"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("workspace removal succeeds");

    assert_eq!(result.stdout, "Removed workspace: fix\n");
    assert_eq!(
        services.workspace_removes.borrow().as_slice(),
        [WorkspaceRemoveOptions {
            name: "fix".to_owned(),
            root: workspace.home.join("projects/.work/jx/fix"),
            cleanup_root: workspace.home.join("projects/.work"),
        }]
    );
}

#[test]
fn work_remove_refuses_current_primary_and_unmanaged_paths() {
    // Verifies: Remove only targets non-current workspaces inside the managed `.work` layout.
    enum RootKind {
        Managed,
        Primary,
        Unmanaged,
    }

    enum ExpectedError {
        Current,
        Primary,
        Unmanaged,
    }

    let cases = [
        ("fix", RootKind::Managed, true, ExpectedError::Current),
        ("default", RootKind::Primary, false, ExpectedError::Primary),
        ("fix", RootKind::Unmanaged, false, ExpectedError::Unmanaged),
    ];

    for (name, root, is_current, expected_error) in cases {
        let workspace = TestWorkspace::new_under("projects/jx");
        workspace.write_home_file(
            ".config/jx/config.toml",
            r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
        );
        let target = WorkspaceEntry {
            name: name.to_owned(),
            root: match root {
                RootKind::Managed => workspace.home.join("projects/.work/jx/fix"),
                RootKind::Primary => workspace.home.join("projects/jx"),
                RootKind::Unmanaged => PathBuf::from("/tmp/jx/fix"),
            },
            is_current,
        };
        let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
        let services = FakeServices {
            workspaces: vec![target.clone()],
            ..FakeServices::default()
        };

        let error = run_with_args_and_services(
            ["jx", "work", "remove", target.name.as_str()],
            &environment,
            &services,
        )
        .expect_err("unsafe workspace removal is rejected");

        match expected_error {
            ExpectedError::Current => assert!(matches!(
                error,
                CommandError::Repository(RepositoryError::RefuseRemoveCurrentWorkspace { .. })
            )),
            ExpectedError::Primary => assert!(matches!(
                error,
                CommandError::Repository(RepositoryError::RefuseRemovePrimaryWorkspace { .. })
            )),
            ExpectedError::Unmanaged => assert!(matches!(
                error,
                CommandError::Repository(RepositoryError::RefuseRemoveUnmanagedWorkspace { .. })
            )),
        }
        assert!(services.workspace_removes.borrow().is_empty());
    }
}

#[test]
fn diff_uses_configured_default_external_tool_and_appends_args() {
    // Verifies: Diff uses the jx default tool and appends operator args after config args.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".jx.toml",
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
fn diff_tool_flag_selects_configured_pipe_tool_and_appends_args() {
    // Verifies: Tool selection can choose a pipe renderer and append renderer args.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".jx.toml",
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
fn diff_rejects_file_arguments() {
    // Verifies: Diff intentionally does not grow a parallel `jj diff` argument surface.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx", "diff", "src/main.rs"], &environment, &services)
        .expect_err("file arguments are not supported");

    assert!(matches!(error, CommandError::Usage(_)));
}

#[test]
fn open_prints_current_repository_url() {
    // Verifies: Open can resolve the current repository without launching a browser.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "open", "--print"], &environment, &services)
        .expect("open print succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo\n"
    );
    assert!(services.opened_urls.borrow().is_empty());
}

#[test]
fn o_alias_runs_open() {
    // Verifies: The short open alias keeps repository navigation quick.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "o", "--print"], &environment, &services)
        .expect("open alias succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo\n"
    );
}

#[test]
fn open_accepts_specific_repository_argument() {
    // Verifies: Open resolves a layout project key and launches that repository URL.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let target = workspace.create_jj_workspace("projects/target");
    TestWorkspace::write_git_config_at(
        &target,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/target.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "open", "target"], &environment, &services)
        .expect("open project succeeds");

    assert_eq!(
        services.opened_urls.borrow().as_slice(),
        ["https://github.com/example-owner/target".to_owned()]
    );
    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/target\n"
    );
}

#[test]
fn open_repo_filter_prints_matching_repository_urls() {
    // Verifies: Open uses the same configured project glob matching as global status commands.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let api = workspace.create_jj_workspace("projects/api");
    let web = workspace.create_jj_workspace("projects/web");
    TestWorkspace::write_git_config_at(
        &api,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &web,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/web.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "open", "--print", "--repo", "*"],
        &environment,
        &services,
    )
    .expect("open filtered repos succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/api\nhttps://github.com/example-owner/web\n"
    );
}

#[test]
fn open_prs_builds_authored_pull_request_search_url() {
    // Verifies: PR navigation uses the authenticated login and repo qualifiers for glob matches.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let api = workspace.create_jj_workspace("projects/api");
    let web = workspace.create_jj_workspace("projects/web");
    TestWorkspace::write_git_config_at(
        &api,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &web,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/web.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "open", "prs", "--print", "--repo", "*"],
        &environment,
        &services,
    )
    .expect("open prs succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/pulls?q=is%3Apr+is%3Aopen+author%3Aexample-user+repo%3Aexample-owner%2Fapi+repo%3Aexample-owner%2Fweb\n"
    );
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
                example_bookmark_link("example-user/02-a1b2c3d4")
            )
        );
}

#[test]
fn remote_status_loads_context_and_renders_github_freshness() {
    // Verifies: Remote status loads context and renders GitHub freshness.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), 3 commits ahead\n"
        );
}

#[test]
fn rs_alias_runs_remote_status() {
    // Verifies: The short alias keeps remote-status distinct from jj status.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "rs"], &environment, &services).expect("rs succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), 3 commits ahead\n"
        );
}

#[test]
fn remote_status_format_json_renders_current_repository_report() {
    // Verifies: JSON remote status keeps the same top-level shape for single-repo output.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "remote-status", "--format", "json"],
        &environment,
        &services,
    )
    .expect("remote-status json succeeds");
    let value: serde_json::Value = serde_json::from_str(&result.stdout).expect("valid json");

    assert_eq!(value["command"], "remote-status");
    assert_eq!(value["version"], 1);
    assert_eq!(
        value["repositories"][0]["root"],
        workspace.path().display().to_string()
    );
    assert_eq!(
        value["repositories"][0]["repository"],
        "example-owner/example-repo"
    );
    assert_eq!(
        value["repositories"][0]["url"],
        "https://github.com/example-owner/example-repo"
    );
    assert_eq!(value["repositories"][0]["remotes"][0]["name"], "origin");
    assert_eq!(
        value["repositories"][0]["remotes"][0]["state"],
        "github-ahead"
    );
    assert_eq!(value["repositories"][0]["remotes"][0]["githubAheadBy"], 3);
}

#[test]
fn remote_status_format_json_renders_global_repository_keys() {
    // Verifies: Global JSON output includes layout keys and absolute roots for org table consumers.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let alpha = workspace.create_jj_workspace("projects/alpha");
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/alpha.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "remote-status", "--all", "--format", "json"],
        &environment,
        &services,
    )
    .expect("global remote-status json succeeds");
    let value: serde_json::Value = serde_json::from_str(&result.stdout).expect("valid json");

    assert_eq!(value["repositories"].as_array().expect("repos").len(), 1);
    assert_eq!(value["repositories"][0]["key"], "alpha");
    assert_eq!(
        value["repositories"][0]["root"],
        alpha.display().to_string()
    );
    assert_eq!(
        value["repositories"][0]["repository"],
        "example-owner/alpha"
    );
}

#[test]
fn remote_status_jobs_rejects_zero_parallelism() {
    // Verifies: Global remote-status keeps a positive batch size so progress cannot stall.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let error = run_with_args_and_services(
        ["jx", "remote-status", "--all", "--jobs", "0"],
        &environment,
        &services,
    )
    .expect_err("zero jobs is rejected");

    assert!(matches!(error, CommandError::Usage(_)));
}

#[test]
fn remote_status_shows_local_commits_as_remote_behind() {
    // Verifies: A synchronized remote still reports unpublished local workspace commits.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        status: StatusReport {
            remotes: vec![domain::RemoteStatusReport {
                name: "origin".to_owned(),
                url: "https://github.com/example-owner/example-repo.git".to_owned(),
                github_url: "https://github.com/example-owner/example-repo".to_owned(),
                branch: "main".to_owned(),
                local_trunk_sha: "1111222233334444".to_owned(),
                local_trunk_short_sha: "11112222".to_owned(),
                local_ahead_by: 2,
                comparison: StatusComparison {
                    state: StatusState::UpToDate,
                    github_ahead_by: 0,
                    github_behind_by: 0,
                },
            }],
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), 2 commits behind\n"
        );
}

#[test]
fn remote_status_renders_one_line_per_github_remote() {
    // Verifies: Remote status output stays remote-oriented when multiple remotes are configured.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
[remote "upstream"]
    url = https://github.com/upstream-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        status: StatusReport {
            remotes: vec![
                domain::RemoteStatusReport {
                    name: "origin".to_owned(),
                    url: "ssh://git@github.com/example-owner/example-repo.git".to_owned(),
                    github_url: "https://github.com/example-owner/example-repo".to_owned(),
                    branch: "main".to_owned(),
                    local_trunk_sha: "1111222233334444".to_owned(),
                    local_trunk_short_sha: "11112222".to_owned(),
                    local_ahead_by: 2,
                    comparison: StatusComparison {
                        state: StatusState::GithubAhead,
                        github_ahead_by: 1,
                        github_behind_by: 0,
                    },
                },
                domain::RemoteStatusReport {
                    name: "upstream".to_owned(),
                    url: "https://github.com/upstream-owner/example-repo.git".to_owned(),
                    github_url: "https://github.com/upstream-owner/example-repo".to_owned(),
                    branch: "main".to_owned(),
                    local_trunk_sha: "aaaabbbbccccdddd".to_owned(),
                    local_trunk_short_sha: "aaaabbbb".to_owned(),
                    local_ahead_by: 1,
                    comparison: StatusComparison {
                        state: StatusState::LocalAhead,
                        github_ahead_by: 0,
                        github_behind_by: 2,
                    },
                },
            ],
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), 1 commit ahead, 2 commits behind\nremote: upstream (\x1b]8;;https://github.com/upstream-owner/example-repo/tree/main\x1b\\https://github.com/upstream-owner/example-repo.git\x1b]8;;\x1b\\), 3 commits behind\n"
        );
}

#[test]
fn remote_status_global_renderer_sorts_entries_by_directory() {
    // Verifies: All-project remote-status output follows stable filesystem order when checks complete out of order.
    let entries = vec![
        GlobalStatusEntry {
            key: Some("alpha".to_owned()),
            root: PathBuf::from("/workspace/src/alpha"),
            display_root: "alpha".to_owned(),
            repository: None,
            result: Err("alpha failed".to_owned()),
        },
        GlobalStatusEntry {
            key: Some("beta".to_owned()),
            root: PathBuf::from("/workspace/projects/beta"),
            display_root: "beta".to_owned(),
            repository: None,
            result: Err("beta failed".to_owned()),
        },
    ];

    let output = render_global_status(&entries, Path::new("/workspace"), false)
        .expect("global status renders");

    assert_eq!(
        output,
        "beta error: beta failed\nalpha error: alpha failed\n"
    );
}

#[test]
fn remote_status_all_prefixes_each_layout_repository_path() {
    // Verifies: Global remote-status scans configured repos with a custom concurrency limit and keeps the normal per-remote row.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let alpha = workspace.create_jj_workspace("projects/alpha");
    let beta = workspace.create_jj_workspace("projects/beta");
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/alpha.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &beta,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/beta.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let progress = RecordingProgress::default();
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };
    let result = run_with_args_and_progress(
        ["jx", "remote-status", "--all", "--jobs", "1"],
        &environment,
        &services,
        &progress,
        prompts,
        OutputMode::plain(),
    )
    .expect("global remote-status succeeds");

    assert_eq!(
        progress.messages(),
        [
            "Checking remote status… 0%",
            "Checking remote status… 50%",
            "Checking remote status… 100%"
        ]
    );
    assert!(progress.finished.get());

    assert_eq!(
        result.stdout,
        "~/projects/alpha remote: origin (\x1b]8;;https://github.com/example-owner/alpha/tree/main\x1b\\ssh://git@github.com/example-owner/alpha.git\x1b]8;;\x1b\\), 3 commits ahead\n~/projects/beta remote: origin (\x1b]8;;https://github.com/example-owner/beta/tree/main\x1b\\ssh://git@github.com/example-owner/beta.git\x1b]8;;\x1b\\), 3 commits ahead\n"
    );
}

#[test]
fn remote_status_accepts_specific_repository_argument() {
    // Verifies: A positional project key runs remote-status from that configured repository only.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let alpha = workspace.create_jj_workspace("projects/alpha");
    let beta = workspace.create_jj_workspace("projects/beta");
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/alpha.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &beta,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/beta.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "rs", "beta"], &environment, &services)
        .expect("specific remote-status succeeds");

    assert_eq!(
        result.stdout,
        "remote: origin (\x1b]8;;https://github.com/example-owner/beta/tree/main\x1b\\ssh://git@github.com/example-owner/beta.git\x1b]8;;\x1b\\), 3 commits ahead\n"
    );
}

#[test]
fn remote_status_repo_filter_accepts_globs() {
    // Verifies: --repo selects configured repository keys with glob matching.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let alpha = workspace.create_jj_workspace("projects/api-alpha");
    let beta = workspace.create_jj_workspace("projects/web-beta");
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api-alpha.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &beta,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/web-beta.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "remote-status", "--repo", "api-*"],
        &environment,
        &services,
    )
    .expect("filtered global remote-status succeeds");

    assert_eq!(
        result.stdout,
        "~/projects/api-alpha remote: origin (\x1b]8;;https://github.com/example-owner/api-alpha/tree/main\x1b\\ssh://git@github.com/example-owner/api-alpha.git\x1b]8;;\x1b\\), 3 commits ahead\n"
    );
}

#[test]
fn remote_status_changed_omits_up_to_date_repositories() {
    // Verifies: --changed keeps global status focused on repos with local or remote deltas.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let changed = workspace.create_jj_workspace("projects/changed");
    let clean = workspace.create_jj_workspace("projects/clean");
    TestWorkspace::write_git_config_at(
        &changed,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/changed.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &clean,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/clean.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        clean_status_repos: vec!["clean".to_owned()],
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "remote-status", "-a", "-c"], &environment, &services)
            .expect("changed global remote-status succeeds");

    assert_eq!(
        result.stdout,
        "~/projects/changed remote: origin (\x1b]8;;https://github.com/example-owner/changed/tree/main\x1b\\ssh://git@github.com/example-owner/changed.git\x1b]8;;\x1b\\), 3 commits ahead\n"
    );
}

#[test]
fn remote_status_all_renders_repository_errors_as_rows() {
    // Verifies: One misconfigured repo does not hide status for the rest of the layout.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let ok = workspace.create_jj_workspace("projects/ok");
    let _missing = workspace.create_jj_workspace("projects/missing-origin");
    TestWorkspace::write_git_config_at(
        &ok,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/ok.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "remote-status", "--all"], &environment, &services)
            .expect("global remote-status keeps going");

    assert_eq!(
        result.stdout,
        "~/projects/missing-origin error: The fixed `origin` remote is missing. Add an `origin` GitHub remote before running `jx`.\n~/projects/ok remote: origin (\x1b]8;;https://github.com/example-owner/ok/tree/main\x1b\\ssh://git@github.com/example-owner/ok.git\x1b]8;;\x1b\\), 3 commits ahead\n"
    );
}

#[test]
fn fetch_renders_rebased_commit_details_like_sync() {
    // Verifies: Fetch uses the same rebased-commit detail section as sync.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "fetch"], &environment, &services)
        .expect("fetch succeeds");

    assert_eq!(
            result.stdout,
            "Fetched: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n\nRebased on origin/main:\n  default@  aaaabbbb -> ccccdddd  example change\n  default@  eeeeffff -> 12345678  follow-up change\n"
        );
}

#[test]
fn fetch_omits_rebase_section_when_no_commits_move() {
    // Verifies: Fetch omits the rebase detail section when no local jj work moved.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: false,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "fetch"], &environment, &services)
        .expect("fetch succeeds");

    assert_eq!(
            result.stdout,
            "Fetched: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n"
        );
}

#[test]
fn fetch_omits_rebase_section_when_repair_has_no_commit_detail() {
    // Verifies: Fetch matches sync by omitting details when repair produced no visible row.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 1,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: true,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "fetch"], &environment, &services)
        .expect("fetch succeeds");

    assert_eq!(
            result.stdout,
            "Fetched: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n"
        );
}

#[test]
fn f_alias_runs_fetch() {
    // Verifies: The short alias keeps the common fetch workflow quick to invoke.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "f"], &environment, &services)
        .expect("fetch alias succeeds");

    assert!(result.stdout.starts_with("Fetched: origin/main "));
}

#[test]
fn fetch_all_only_fetches_global_ready_repositories() {
    // Verifies: Global fetch mutates only repos whose working copy is safe to auto-fetch.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let ready = workspace.create_jj_workspace("projects/ready");
    let local_work = workspace.create_jj_workspace("projects/local-work");
    TestWorkspace::write_git_config_at(
        &ready,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/ready.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &local_work,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/local-work.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        global_fetch_ready_roots: Some(BTreeSet::from([ready.clone()])),
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: false,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "fetch", "--all"], &environment, &services)
        .expect("global fetch succeeds");

    assert_eq!(result.stdout, "~/projects/ready\n");
    assert_eq!(services.fetch_origin_roots.borrow().as_slice(), [ready]);
}

#[test]
fn fetch_accepts_specific_repository_argument() {
    // Verifies: A positional project key fetches that repository with normal single-repo behavior.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let target = workspace.create_jj_workspace("projects/target");
    TestWorkspace::write_git_config_at(
        &target,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/target.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: false,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "fetch", "target"], &environment, &services)
        .expect("specific fetch succeeds");

    assert_eq!(services.fetch_origin_roots.borrow().as_slice(), [target]);
    assert_eq!(
        result.stdout,
        "Fetched: origin/main (\x1b]8;;https://github.com/example-owner/target/tree/main\x1b\\ssh://git@github.com/example-owner/target.git\x1b]8;;\x1b\\)\n"
    );
}

#[test]
fn f_alias_accepts_all_flag() {
    // Verifies: The short fetch alias supports global fetch ergonomics.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let ready = workspace.create_jj_workspace("projects/ready");
    TestWorkspace::write_git_config_at(
        &ready,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/ready.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: false,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "f", "-a"], &environment, &services)
        .expect("global fetch alias succeeds");

    assert_eq!(result.stdout, "~/projects/ready\n");
}

#[test]
fn rebase_on_trunk_rebases_current_by_default() {
    // Verifies: Rebase-on-trunk defaults to rebasing the working-copy change.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        expected_rebase_sources: Some(Vec::new()),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "rebase-on-trunk"], &environment, &services)
        .expect("rebase-on-trunk succeeds");

    assert_eq!(
            result.stdout,
            "Rebased: a1b2c3d4 onto origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), rebased 2 commits\n"
        );
}

#[test]
fn rt_alias_accepts_source_flag() {
    // Verifies: The rt alias can rebase a specific source revision and descendants.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        expected_rebase_sources: Some(vec!["deadbeef".to_owned()]),
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "rt", "-s", "deadbeef"], &environment, &services)
            .expect("rt succeeds");

    assert!(result
        .stdout
        .starts_with("Rebased: a1b2c3d4 onto origin/main "));
}

#[test]
fn rt_alias_accepts_repeated_source_flags() {
    // Verifies: The rt alias forwards repeated source revisions as one rebase operation.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        expected_rebase_sources: Some(vec!["aaaabbbb".to_owned(), "ccccdddd".to_owned()]),
        rebase_on_trunk: RebaseOnTrunkOutcome {
            source_short_commit_ids: vec!["aaaabbbb".to_owned(), "ccccdddd".to_owned()],
            ..FakeServices::default().rebase_on_trunk
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "rt", "-s", "aaaabbbb", "--source", "ccccdddd"],
        &environment,
        &services,
    )
    .expect("rt succeeds");

    assert!(result
        .stdout
        .starts_with("Rebased: 2 sources onto origin/main "));
}

#[test]
fn rebase_on_trunk_renders_up_to_date_when_no_commits_move() {
    // Verifies: Rebase-on-trunk says up to date when the source already sits on trunk.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        rebase_on_trunk: RebaseOnTrunkOutcome {
            rebased_commits: 0,
            skipped_commits: 1,
            current_updated: false,
            ..FakeServices::default().rebase_on_trunk
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "rebase-on-trunk"], &environment, &services)
        .expect("rebase-on-trunk succeeds");

    assert!(result.stdout.ends_with(", up to date\n"));
}

#[test]
fn push_reuses_existing_bookmark_on_current_change() {
    // Verifies: Push uses the current change and existing bookmark by default.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut workspace_facts = workspace_facts();
    workspace_facts.local_bookmarks = vec!["example-user/current".to_owned()];
    workspace_facts.local_bookmarks_at_target = workspace_facts.local_bookmarks.clone();
    let services = FakeServices {
        workspace: workspace_facts,
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "push"], &environment, &services).expect("push succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Pushed: {} -> a1b2c3d4\n",
            example_bookmark_link("example-user/current")
        )
    );
}

#[test]
fn push_revision_creates_generated_bookmark_after_confirmation() {
    // Verifies: Push can target a selected revision and create the displayed bookmark.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "push", "-r", "deadbeef"], &environment, &services)
            .expect("push succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Pushed: {} -> deadbeef (created bookmark)\n",
            example_bookmark_link("push-zzzzzzzz")
        )
    );
}

#[test]
fn push_can_be_cancelled_before_creating_generated_bookmark() {
    // Verifies: Declining generated bookmark creation stops before push mutation.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();
    let confirmer = FixedPushConfirmer { confirmed: false };

    let result =
        run_with_args_and_push_confirmer(["jx", "push"], &environment, &services, &confirmer)
            .expect("push cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
}

#[test]
fn push_tracked_pushes_tracked_bookmarks_and_deletions() {
    // Verifies: Tracked push reports both moved and deleted tracked bookmarks.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "push", "--tracked"], &environment, &services)
        .expect("tracked push succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Pushed tracked bookmarks:\n  {}: 11112222 -> a1b2c3d4\n  {}: deleted from 99990000\n",
            example_bookmark_link("example-user/current"),
            example_bookmark_link("example-user/old")
        )
    );
}

#[test]
fn sync_fetches_then_pushes_tracked_state_with_commit_lists() {
    // Verifies: Sync renders rebased commits first, then pushed bookmark heads and deletions.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert_eq!(services.advance_trunk_calls.get(), 0);
    assert_eq!(
            result.stdout,
            format!(
                "Synced: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n\nRebased on origin/main:\n  default@  aaaabbbb -> ccccdddd  example change\n  default@  eeeeffff -> 12345678  follow-up change\n\nPushed commits:\n  default@  a1b2c3d4 -> {}  example change\n\nDeleted bookmarks:\n  {}: 99990000 obsolete example change\n",
                example_bookmark_link("example-user/current"),
                example_bookmark_link("example-user/old")
            )
        );
}

#[test]
fn sync_accepts_specific_repository_argument() {
    // Verifies: A positional project key runs the normal sync flow from that configured repository.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let target = workspace.create_jj_workspace("projects/target");
    TestWorkspace::write_git_config_at(
        &target,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/target.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "sync", "target"], &environment, &services)
        .expect("specific sync succeeds");

    assert_eq!(services.fetch_origin_roots.borrow().as_slice(), [target]);
    assert!(result.stdout.starts_with(
        "Synced: origin/main (\x1b]8;;https://github.com/example-owner/target/tree/main"
    ));
}

#[test]
fn sync_creates_missing_origin_repository_from_layout() {
    // Verifies: Missing-origin sync can create the expected private GitHub repo and push main.
    let workspace = TestWorkspace::new_under("work/example-repo");
    workspace.write_file(
        ".jx.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let target = InitialPublishTarget {
        commit_id: "a1b2c3d4e5f6".to_owned(),
        short_commit_id: "a1b2c3d4".to_owned(),
        description: "example change".to_owned(),
    };
    let services = FakeServices {
        initial_publish_target: target.clone(),
        expected_bootstrap: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            target,
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect("sync bootstrap succeeds");

    assert_eq!(services.create_repository_calls.get(), 1);
    assert_eq!(
            result.stdout,
            format!(
                "Created private {} repo\nPushed a1b2c3d4 to {}\nWorking copy now at bf4799d5 (empty)\n",
                osc8_link(
                    "https://github.com/example-owner/example-repo",
                    "git@github.com:example-owner/example-repo.git"
                ),
                osc8_link("https://github.com/example-owner/example-repo/tree/main", "main")
            )
        );
}

#[test]
fn sync_initializes_layout_directory_before_bootstrap() {
    // Verifies: Missing-workspace sync initializes the inferred layout repo before bootstrap.
    let workspace = TestWorkspace::new_uninitialized_under("work/example-repo");
    workspace.write_file(
        ".jx.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    workspace.write_file("README.md", "hello\n");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let target = InitialPublishTarget {
        commit_id: "a1b2c3d4e5f6".to_owned(),
        short_commit_id: "a1b2c3d4".to_owned(),
        description: "example change".to_owned(),
    };
    let services = FakeServices {
        expected_init_repository: Some(workspace.path()),
        initial_publish_target: target.clone(),
        expected_bootstrap: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            target,
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect("sync bootstrap succeeds");

    assert_eq!(services.init_repository_calls.get(), 1);
    assert_eq!(services.create_repository_calls.get(), 1);
    assert_eq!(
        result.stdout,
        format!(
            "Created private {} repo\nPushed a1b2c3d4 to {}\nWorking copy now at bf4799d5 (empty)\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            ),
            osc8_link("https://github.com/example-owner/example-repo/tree/main", "main")
        )
    );
}

#[test]
fn sync_refuses_uninitialized_directory_outside_configured_layout() {
    // Verifies: Sync only initializes directories whose GitHub identity is layout-derived.
    let workspace = TestWorkspace::new_uninitialized_under("misc/example-repo");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect_err("off-layout directory is not initialized");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::LayoutPathNotMatched { .. })
    ));
    assert_eq!(services.init_repository_calls.get(), 0);
    assert_eq!(services.create_repository_calls.get(), 0);
}

#[test]
fn sync_prepares_undescribed_initial_commit_before_bootstrap() {
    // Verifies: Missing-origin sync describes a fresh initial commit before pushing main.
    let workspace = TestWorkspace::new_under("work/example-repo");
    workspace.write_file(
        ".jx.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let target = InitialPublishTarget {
        commit_id: "a1b2c3d4e5f6".to_owned(),
        short_commit_id: "a1b2c3d4".to_owned(),
        description: String::new(),
    };
    let prepared = InitialPublishTarget {
        commit_id: "111122223333".to_owned(),
        short_commit_id: "11112222".to_owned(),
        description: "initial commit".to_owned(),
    };
    let services = FakeServices {
        initial_publish_target: target,
        prepared_initial_publish_target: Some(prepared.clone()),
        expected_bootstrap: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            prepared.clone(),
        )),
        bootstrap_push: BootstrapPushOutcome {
            branch: "main".to_owned(),
            short_commit_id: prepared.short_commit_id.clone(),
            description: prepared.description.clone(),
            working_copy_short_commit_id: Some("bf4799d5".to_owned()),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect("sync bootstrap succeeds");

    assert_eq!(services.prepare_initial_publish_calls.get(), 1);
    assert_eq!(services.create_repository_calls.get(), 1);
    assert_eq!(
        result.stdout,
        format!(
            "Created private {} repo\nPushed 11112222 to {}\nWorking copy now at bf4799d5 (empty)\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            ),
            osc8_link("https://github.com/example-owner/example-repo/tree/main", "main")
        )
    );
}

#[test]
fn sync_can_cancel_missing_origin_repository_creation() {
    // Verifies: Declining repository creation stops before GitHub or jj mutation.
    let workspace = TestWorkspace::new_under("work/example-repo");
    workspace.write_file(
        ".jx.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let services = FakeServices::default();
    let confirmer = FixedRepositoryCreationConfirmer { confirmed: false };

    let result = run_with_args_and_repository_creation_confirmer(
        ["jx", "sync"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("sync cancellation succeeds");

    assert_eq!(services.prepare_initial_publish_calls.get(), 0);
    assert_eq!(services.create_repository_calls.get(), 0);
    assert_eq!(result.stdout, "cancelled\n");
}

#[test]
fn sync_refuses_missing_origin_outside_configured_layout() {
    // Verifies: Repository bootstrap only runs when layout can infer the GitHub identity.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect_err("off-layout repo cannot be bootstrapped");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::LayoutPathNotMatched { .. })
    ));
    assert_eq!(services.create_repository_calls.get(), 0);
}

#[test]
fn sync_advances_trunk_when_repo_policy_enables_it() {
    // Verifies: Sync runs the optional trunk-advance preparation only for matching repo policy.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    workspace.write_file(
        ".jx.toml",
        r#"
[repo]
advance_trunk = true
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert_eq!(services.advance_trunk_calls.get(), 1);
}

#[test]
fn sync_links_pull_requests_under_changed_bookmarks() {
    // Verifies: Sync shows PR annotations as secondary, linked rows under changed bookmarks.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        sync_pull_requests: vec![
            PullRequestRecord {
                number: 1234,
                title: "current pull request".to_owned(),
                body: None,
                head_branch: "example-user/current".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some(
                    "https://github.com/example-owner/example-repo/pull/1234".to_owned(),
                ),
                draft: false,
            },
            PullRequestRecord {
                number: 1200,
                title: "old pull request".to_owned(),
                body: None,
                head_branch: "example-user/old".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some(
                    "https://github.com/example-owner/example-repo/pull/1200".to_owned(),
                ),
                draft: false,
            },
        ],
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.contains(&format!(
        "Pushed commits:\n  default@  a1b2c3d4 -> {}  example change\n{}↳ PR {}\n",
        example_bookmark_link("example-user/current"),
        " ".repeat(24),
        example_pull_request_link(1234)
    )));
    assert!(result.stdout.contains(&format!(
        "Deleted bookmarks:\n  {}: 99990000 obsolete example change\n  ↳ PR {}\n",
        example_bookmark_link("example-user/old"),
        example_pull_request_link(1200)
    )));
}

#[test]
fn sync_aligns_pushed_bookmark_targets_and_deleted_bookmarks() {
    // Verifies: Sync aligns pushed targets and keeps deleted bookmark rows workspace-free.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut services = FakeServices::default();
    services.tracked_push.bookmarks = vec![
        PushedBookmarkSummary {
            branch: "b".to_owned(),
            old_short_commit_id: Some("11112222".to_owned()),
            new_short_commit_id: Some("22223333".to_owned()),
            old_description: Some("previous short branch".to_owned()),
            new_description: Some("short branch".to_owned()),
            new_workspace_visibility: current_workspace_visibility(),
        },
        PushedBookmarkSummary {
            branch: "long".to_owned(),
            old_short_commit_id: Some("22223333".to_owned()),
            new_short_commit_id: Some("33334444".to_owned()),
            old_description: Some("previous long branch".to_owned()),
            new_description: Some("long branch".to_owned()),
            new_workspace_visibility: current_workspace_visibility(),
        },
        PushedBookmarkSummary {
            branch: "old".to_owned(),
            old_short_commit_id: Some("44445555".to_owned()),
            new_short_commit_id: None,
            old_description: Some("old branch".to_owned()),
            new_description: None,
            new_workspace_visibility: WorkspaceVisibility::default(),
        },
    ];

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.contains(&format!(
            "Pushed commits:\n  default@  22223333 -> {}     short branch\n  default@  33334444 -> {}  long branch\n",
            example_bookmark_link("b"),
            example_bookmark_link("long")
        )));
    assert!(result.stdout.contains(&format!(
        "Deleted bookmarks:\n  {}: 44445555 old branch\n",
        example_bookmark_link("old")
    )));
}

#[test]
fn sync_expands_and_aligns_workspace_rows() {
    // Verifies: Workspace labels define scan order while unowned rows keep commit columns aligned.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut services = FakeServices::default();
    services.fetch.rebased_commits = vec![
        RebasedCommitSummary {
            old_short_commit_id: "aaaamult".to_owned(),
            new_short_commit_id: "bbbbmult".to_owned(),
            description: "multi workspace".to_owned(),
            has_conflict: false,
            is_empty: false,
            workspace_visibility: visible_in(&["default", "review"], true),
        },
        RebasedCommitSummary {
            old_short_commit_id: "aaaaothr".to_owned(),
            new_short_commit_id: "bbbbothr".to_owned(),
            description: "other workspace".to_owned(),
            has_conflict: false,
            is_empty: false,
            workspace_visibility: visible_in(&["review"], false),
        },
        RebasedCommitSummary {
            old_short_commit_id: "aaaacurr".to_owned(),
            new_short_commit_id: "bbbbcurr".to_owned(),
            description: "current workspace".to_owned(),
            has_conflict: false,
            is_empty: false,
            workspace_visibility: current_workspace_visibility(),
        },
        RebasedCommitSummary {
            old_short_commit_id: "aaaanone".to_owned(),
            new_short_commit_id: "bbbbnone".to_owned(),
            description: "no workspace".to_owned(),
            has_conflict: false,
            is_empty: false,
            workspace_visibility: WorkspaceVisibility::default(),
        },
    ];
    services.tracked_push.bookmarks = Vec::new();

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.contains(
            "Rebased on origin/main:\n  default@  aaaamult -> bbbbmult  multi workspace\n  default@  aaaacurr -> bbbbcurr  current workspace\n  review@   aaaamult -> bbbbmult  multi workspace\n  review@   aaaaothr -> bbbbothr  other workspace\n            aaaanone -> bbbbnone  no workspace\n"
        ));
}

#[test]
fn sync_omits_deleted_bookmark_section_when_none_were_deleted() {
    // Verifies: Sync only shows deleted bookmark details when tracked deletions were pushed.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut tracked_push = FakeServices::default().tracked_push;
    tracked_push
        .bookmarks
        .retain(|bookmark| bookmark.new_short_commit_id.is_some());
    let services = FakeServices {
        tracked_push,
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.contains(&format!(
        "Pushed commits:\n  default@  a1b2c3d4 -> {}  example change\n",
        example_bookmark_link("example-user/current")
    )));
    assert!(!result.stdout.contains("Deleted bookmarks:"));
}

#[test]
fn sync_omits_empty_rebase_and_push_sections_when_only_deletions_changed() {
    // Verifies: Sync only renders sections whose underlying operation changed visible state.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut tracked_push = FakeServices::default().tracked_push;
    tracked_push
        .bookmarks
        .retain(|bookmark| bookmark.new_short_commit_id.is_none());
    let services = FakeServices {
        fetch: FetchOutcome {
            rebased_commits: Vec::new(),
            ..FakeServices::default().fetch
        },
        tracked_push,
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(!result.stdout.contains("Rebased on origin/main:"));
    assert!(!result.stdout.contains("Pushed commits:"));
    assert!(result.stdout.contains(&format!(
        "Deleted bookmarks:\n  {}:",
        example_bookmark_link("example-user/old")
    )));
}

#[test]
fn sync_renders_only_summary_when_nothing_changed() {
    // Verifies: A no-op sync remains glanceable by omitting empty detail sections.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        fetch: FetchOutcome {
            rebased_commits: Vec::new(),
            ..FakeServices::default().fetch
        },
        tracked_push: TrackedPushOutcome {
            bookmarks: Vec::new(),
            pushed_commits: Vec::new(),
            pushed_refs: 0,
        },
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert_eq!(
            result.stdout,
            "Synced: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n"
        );
}

#[test]
fn sync_aborts_when_fetch_creates_conflicts() {
    // Verifies: Sync refuses to push after fetch reports conflicted rebased commits.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut fetch = FakeServices::default().fetch;
    fetch.rebased_commits[0].has_conflict = true;
    let services = FakeServices {
        fetch,
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect_err("conflicted sync fails");

    assert!(matches!(
        error,
        CommandError::Workflow(WorkflowError::FetchConflicts { .. })
    ));
    assert!(error.to_string().contains("ccccdddd"));
}

#[test]
fn pull_request_accepts_short_task_id_flag_and_renders_published_pr() {
    // Verifies: Pull request accepts short task ID flag and renders the published PR URL.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "pull-request", "-t", "ABC-123"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        "Created https://github.com/example-owner/example-repo/pull/42\n"
    );
}

#[test]
fn pull_request_infers_task_id_from_workspace_metadata() {
    // Verifies: Workspace metadata supplies the default task ID for PR planning.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_workspace_metadata(
        &workspace.path(),
        &WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
        },
    )
    .expect("metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        expected_task_id: Some(Some("ABC-123".to_owned())),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "pr"], &environment, &services)
        .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        "Created https://github.com/example-owner/example-repo/pull/42\n"
    );
}

#[test]
fn pull_request_task_id_flag_overrides_workspace_metadata() {
    // Verifies: Explicit PR task IDs win over workspace-local defaults.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_workspace_metadata(
        &workspace.path(),
        &WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
        },
    )
    .expect("metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        expected_task_id: Some(Some("XYZ-9".to_owned())),
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "pr", "--task-id", "XYZ-9"], &environment, &services)
        .expect("pull request publishes");
}

#[test]
fn pull_request_accepts_repeated_label_flags() {
    // Verifies: PR publishing applies each operator-supplied label once.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        expected_labels: vec!["bug".to_owned(), "help wanted".to_owned()],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        [
            "jx",
            "pull-request",
            "--label",
            "bug",
            "-l",
            "help wanted",
            "-l",
            "bug",
        ],
        &environment,
        &services,
    )
    .expect("pull request publishes with labels");

    assert_eq!(
        result.stdout,
        "Created https://github.com/example-owner/example-repo/pull/42\n"
    );
}

#[test]
fn pull_request_renders_updated_pr_url() {
    // Verifies: Pull request update success uses the same one-line URL format as creation.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_action: PullRequestAction::Updated,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "pull-request"], &environment, &services)
        .expect("pull request updates");

    assert_eq!(
        result.stdout,
        "Updated https://github.com/example-owner/example-repo/pull/42\n"
    );
}

#[test]
fn pull_request_falls_back_to_number_when_github_omits_url() {
    // Verifies: Pull request output remains concise when GitHub omits a PR URL.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_url: None,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "pull-request"], &environment, &services)
        .expect("pull request publishes without html url");

    assert_eq!(result.stdout, "Created PR #42\n");
}

#[test]
fn pr_alias_accepts_commit_flag_and_plans_that_commit() {
    // Verifies: The PR alias accepts a revision flag and plans the selected commit.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "pr", "-c", "deadbeef", "-t", "ABC-123"],
        &environment,
        &services,
    )
    .expect("pr publishes selected commit");

    assert_eq!(
        result.stdout,
        "Created https://github.com/example-owner/example-repo/pull/42\n"
    );
}

#[test]
fn pull_request_accepts_repeated_reviewer_flags() {
    // Verifies: Explicit reviewer flags become the requested reviewers and skip prompting.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        expected_reviewers: Some(ReviewerSelection {
            users: vec!["example-reviewer".to_owned()],
            teams: vec!["frontend".to_owned()],
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_reviewer_selector(
        [
            "jx",
            "pull-request",
            "-r",
            "example-reviewer",
            "--reviewer",
            "ExampleOrg/frontend",
        ],
        &environment,
        &services,
        &CheckedReviewerSelector,
    )
    .expect("pull request publishes with explicit reviewers");

    assert_eq!(
        result.stdout,
        "Created https://github.com/example-owner/example-repo/pull/42\n"
    );
}

#[test]
fn pull_request_respects_draft_flag() {
    // Verifies: The draft flag is carried into PR creation planning.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        expected_draft: Some(true),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "pr", "--draft"], &environment, &services)
        .expect("draft pull request publishes");

    assert_eq!(
        result.stdout,
        "Created https://github.com/example-owner/example-repo/pull/42\n"
    );
}

#[test]
fn pull_request_can_be_cancelled_after_planning() {
    // Verifies: Declining the final confirmation stops before bookmark, push, or PR mutation.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let confirmer = FixedPullRequestConfirmer { confirmed: false };

    let result = run_with_args_and_prompts(
        ["jx", "pull-request"],
        &environment,
        &services,
        &SelectAllReviewers,
        &confirmer,
    )
    .expect("pull request cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
}

#[test]
fn pull_request_rejects_invalid_cli_reviewer() {
    // Verifies: Reviewer flags use the same user/team shape as config reviewers.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let error = run_with_args_and_services(
        ["jx", "pull-request", "--reviewer", "bad/reviewer/name"],
        &environment,
        &services,
    )
    .expect_err("invalid reviewer is rejected during parsing");

    assert!(matches!(error, CommandError::Usage(_)));
}

#[test]
fn pull_request_uses_interactively_selected_reviewers() {
    // Verifies: PR publishing syncs only the configured reviewers selected by the operator.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let selected = ReviewerSelection::new(["second-reviewer"], std::iter::empty::<&str>());
    let services = FakeServices {
        reviewer_candidates: vec![
            ReviewerCandidate::new(
                ReviewerTarget::user("example-reviewer"),
                vec!["global".to_owned()],
            ),
            ReviewerCandidate::new(
                ReviewerTarget::user("second-reviewer"),
                vec!["src/** matched 1 file".to_owned()],
            ),
        ],
        expected_reviewers: Some(selected.clone()),
        ..FakeServices::default()
    };
    let selector = FixedReviewerSelector { selected };

    let result = run_with_args_and_reviewer_selector(
        ["jx", "pull-request"],
        &environment,
        &services,
        &selector,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        "Created https://github.com/example-owner/example-repo/pull/42\n"
    );
}

#[test]
fn reviewer_selection_formats_cli_reviewers_first_and_summarizes_reasons() {
    // Verifies: Reviewer choices show explicit reviewers first and keep ownership hints concise.
    let candidates = vec![
        ReviewerCandidate::new(
            ReviewerTarget::user("example-reviewer"),
            vec!["global".to_owned()],
        ),
        ReviewerCandidate::new(
            ReviewerTarget::team("ExampleOrg/frontend", "frontend"),
            vec![
                "src/** matched 2 files".to_owned(),
                "tests/** matched 1 file".to_owned(),
            ],
        ),
    ];
    let choices = reviewer_choices(
        &candidates,
        &[
            ReviewerTarget::user("cli-reviewer"),
            ReviewerTarget::team("ExampleOrg/frontend", "frontend"),
        ],
    );

    assert_eq!(choices[0].target.display_name(), "cli-reviewer");
    assert!(choices[0].checked);
    assert_eq!(choices[0].label(), "cli-reviewer");
    assert_eq!(choices[1].target.display_name(), "ExampleOrg/frontend");
    assert!(choices[1].checked);
    assert_eq!(
        choices[1].label(),
        "ExampleOrg/frontend      \x1b[38;5;244mmatched 3 files\x1b[0m"
    );
    assert_eq!(
        choices[2].label(),
        "example-reviewer         \x1b[38;5;244mglobal\x1b[0m"
    );
    assert!(!choices[2].checked);
    assert_eq!(
        selection_from_indexes(&choices, &[1]),
        ReviewerSelection {
            users: Vec::new(),
            teams: vec!["frontend".to_owned()],
        }
    );
}

#[test]
fn workspace_status_renderer_orders_commit_description_and_jj_changes() {
    // Verifies: Status rendering puts jj commit lines first, then description, then jj file lines.
    let status = WorkspaceStatus {
        commit_lines: vec![
            "Working copy  (@) : kvxvwztp b9e8f888".to_owned(),
            "Parent commit (@-): xskrmynn 6257dd5a main | parent".to_owned(),
        ],
        description: "Add rebase-on-trunk command".to_owned(),
        change_lines: vec!["M README.md".to_owned(), "M src/commands.rs".to_owned()],
        extra_lines: Vec::new(),
    };

    assert_eq!(
            render_workspace_status_with_width(&status, 80),
            "Working copy  (@) : kvxvwztp b9e8f888\nParent commit (@-): xskrmynn 6257dd5a main | parent\n\nAdd rebase-on-trunk command\n\nM README.md\nM src/commands.rs\n"
        );
}

#[test]
fn workspace_status_renderer_renders_markdown_description_for_readability() {
    // Verifies: The shared status renderer keeps PR-description markdown readable in terminal output.
    let rendered = render_status_description(
        "# Summary\n\nThis is **important** markdown with enough words to wrap.",
        28,
    );

    assert!(rendered.lines().count() > 2, "{rendered:?}");
    assert!(rendered.contains("Summary"), "{rendered:?}");
    assert!(rendered.contains("important"), "{rendered:?}");
    assert!(!rendered.contains("**important**"), "{rendered:?}");
}

#[test]
fn pull_request_preview_reuses_workspace_status_renderer_and_adds_labels() {
    // Verifies: PR preview shares status output with jx status and appends PR-only metadata.
    let mut plan = preview_plan();
    plan.labels = vec!["bug".to_owned(), "help wanted".to_owned()];
    let status = workspace_status();

    let preview = render_pull_request_preview(&plan, &status);

    assert!(preview.starts_with(&expected_workspace_status()));
    assert!(preview.ends_with("\nLabels: bug, help wanted\n"));
    assert_eq!(
        pull_request_confirmation_prompt(&plan),
        "Create pull request?"
    );
    plan.draft = true;
    assert_eq!(
        pull_request_confirmation_prompt(&plan),
        "Create draft pull request?"
    );
    plan.existing_pull_request = Some(existing_pull_request(false));
    assert_eq!(
        pull_request_confirmation_prompt(&plan),
        "Update pull request?"
    );
    plan.existing_pull_request = Some(existing_pull_request(true));
    assert_eq!(
        pull_request_confirmation_prompt(&plan),
        "Update draft pull request?"
    );
}

#[test]
fn remote_status_rejects_missing_origin_with_actionable_error() {
    // Verifies: Remote status rejects missing origin with actionable error.
    let workspace = TestWorkspace::new();
    workspace.write_git_config("");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect_err("origin is required");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::MissingOrigin)
    ));
    assert!(error
        .to_string()
        .contains("fixed `origin` remote is missing"));
}

#[test]
fn fetch_rejects_non_github_origin_with_actionable_error() {
    // Verifies: Fetch rejects non-GitHub origin URLs with an actionable error.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://example.invalid/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let error =
        run_with_args(["jx", "fetch"], &environment).expect_err("github origin is required");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::OriginNotGitHub { .. })
    ));
    assert!(error.to_string().contains("not a GitHub repository URL"));
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

struct FixedReviewerSelector {
    selected: ReviewerSelection,
}

impl ReviewerSelector for FixedReviewerSelector {
    fn select_reviewers(
        &self,
        _candidates: &[ReviewerCandidate],
        _preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError> {
        Ok(self.selected.clone())
    }
}

struct CheckedReviewerSelector;

impl ReviewerSelector for CheckedReviewerSelector {
    fn select_reviewers(
        &self,
        candidates: &[ReviewerCandidate],
        preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError> {
        let choices = reviewer_choices(candidates, preselected);
        Ok(selection_from_choices(
            choices.iter().filter(|choice| choice.checked),
        ))
    }
}

struct FixedPullRequestConfirmer {
    confirmed: bool,
}

impl PullRequestConfirmer for FixedPullRequestConfirmer {
    fn confirm_pull_request(
        &self,
        _plan: &PullRequestPlan,
    ) -> Result<bool, PullRequestConfirmationError> {
        Ok(self.confirmed)
    }
}

struct FixedPushConfirmer {
    confirmed: bool,
}

impl PushConfirmer for FixedPushConfirmer {
    fn confirm_push(&self, _plan: &PushPlan) -> Result<bool, PushConfirmationError> {
        Ok(self.confirmed)
    }
}

struct FixedWorkspaceRemoveConfirmer {
    confirmed: bool,
}

impl WorkspaceRemoveConfirmer for FixedWorkspaceRemoveConfirmer {
    fn confirm_workspace_remove(
        &self,
        _workspace: &WorkspaceEntry,
    ) -> Result<bool, WorkspaceRemoveConfirmationError> {
        Ok(self.confirmed)
    }
}

fn existing_pull_request(draft: bool) -> PullRequestRecord {
    PullRequestRecord {
        number: 7,
        title: "existing PR".to_owned(),
        body: Some("existing body".to_owned()),
        head_branch: "example-user/02-a1b2c3d4".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
        draft,
    }
}

fn preview_plan() -> PullRequestPlan {
    PullRequestPlan {
        repository: RepositorySummary {
            origin_name: "origin",
            origin_url: "https://github.com/example-owner/example-repo.git".to_owned(),
            github_slug: "example-owner/example-repo".to_owned(),
            github_url: "https://github.com/example-owner/example-repo".to_owned(),
            token_source: "GH_TOKEN environment variable".to_owned(),
            config: "defaults",
            default_reviewers: "none".to_owned(),
        },
        task_id: None,
        bookmark: BookmarkPlan {
            branch: "example-user/02-a1b2c3d4".to_owned(),
            action: BookmarkAction::Create,
        },
        target_commit_id: "a1b2c3d4e5f6".to_owned(),
        title: "example change".to_owned(),
        body: "example change".to_owned(),
        changed_files: vec!["src/main.rs".to_owned()],
        base: "main".to_owned(),
        head: PullRequestHead::same_repository("example-owner", "example-user/02-a1b2c3d4"),
        labels: Vec::new(),
        draft: false,
        existing_pull_request: None,
        reviewer_candidates: Vec::new(),
        reviewers: ReviewerSelection::default(),
    }
}

struct FakeServices {
    workspace_log: String,
    workspace_status: WorkspaceStatus,
    workspace: WorkspaceFacts,
    status_workspace: StatusWorkspaceFacts,
    check: CheckReport,
    status: StatusReport,
    status_uses_context_remotes: bool,
    clean_status_repos: Vec<String>,
    github_login: String,
    opened_urls: std::cell::RefCell<Vec<String>>,
    global_fetch_ready_roots: Option<BTreeSet<PathBuf>>,
    fetch_origin_roots: std::cell::RefCell<Vec<PathBuf>>,
    fetch: FetchOutcome,
    rebase_on_trunk: RebaseOnTrunkOutcome,
    expected_rebase_sources: Option<Vec<String>>,
    bookmark_update: BookmarkUpdate,
    push: PushOutcome,
    advance_trunk: AdvanceTrunkOutcome,
    advance_trunk_calls: std::cell::Cell<usize>,
    tracked_push: TrackedPushOutcome,
    sync_pull_requests: Vec<PullRequestRecord>,
    pull_request_action: PullRequestAction,
    pull_request_url: Option<String>,
    existing_pull_request: Option<PullRequestRecord>,
    reviewer_candidates: Vec<ReviewerCandidate>,
    expected_reviewers: Option<ReviewerSelection>,
    expected_task_id: Option<Option<String>>,
    expected_labels: Vec<String>,
    expected_draft: Option<bool>,
    expected_clone: Option<(String, PathBuf)>,
    expected_init_repository: Option<PathBuf>,
    init_repository_calls: std::cell::Cell<usize>,
    expected_workspace_add: Option<WorkspaceAddOptions>,
    workspaces: Vec<WorkspaceEntry>,
    workspace_removes: std::cell::RefCell<Vec<WorkspaceRemoveOptions>>,
    initial_publish_target: InitialPublishTarget,
    prepared_initial_publish_target: Option<InitialPublishTarget>,
    prepare_initial_publish_calls: std::cell::Cell<usize>,
    created_repository: RepositoryCreation,
    create_repository_calls: std::cell::Cell<usize>,
    expected_bootstrap: Option<(String, InitialPublishTarget)>,
    bootstrap_push: BootstrapPushOutcome,
}

impl Default for FakeServices {
    fn default() -> Self {
        let repository = RepositorySummary {
            origin_name: "origin",
            origin_url: "https://github.com/example-owner/example-repo.git".to_owned(),
            github_slug: "example-owner/example-repo".to_owned(),
            github_url: "https://github.com/example-owner/example-repo".to_owned(),
            token_source: "GH_TOKEN environment variable".to_owned(),
            config: "defaults",
            default_reviewers: "none".to_owned(),
        };

        Self {
            workspace_log: "workspace log\n".to_owned(),
            workspace_status: workspace_status(),
            workspace: workspace_facts(),
            status_workspace: status_workspace_facts(),
            check: CheckReport {
                repository: repository.clone(),
                workspace: CheckWorkspaceSummary {
                    trunk_branch: "main".to_owned(),
                    trunk_short_commit_id: "11112222".to_owned(),
                    current_short_commit_id: "a1b2c3d4".to_owned(),
                    current_is_empty: false,
                    stack_index: 2,
                },
                github: GitHubReadiness {
                    login: "example-user".to_owned(),
                    default_branch: Some("main".to_owned()),
                    can_push: true,
                },
                bookmark: BookmarkPlan {
                    branch: "example-user/02-a1b2c3d4".to_owned(),
                    action: BookmarkAction::Create,
                },
            },
            status: StatusReport {
                remotes: vec![domain::RemoteStatusReport {
                    name: "origin".to_owned(),
                    url: "https://github.com/example-owner/example-repo.git".to_owned(),
                    github_url: "https://github.com/example-owner/example-repo".to_owned(),
                    branch: "main".to_owned(),
                    local_trunk_sha: "1111222233334444".to_owned(),
                    local_trunk_short_sha: "11112222".to_owned(),
                    local_ahead_by: 0,
                    comparison: StatusComparison {
                        state: StatusState::GithubAhead,
                        github_ahead_by: 3,
                        github_behind_by: 0,
                    },
                }],
            },
            status_uses_context_remotes: false,
            clean_status_repos: Vec::new(),
            github_login: "example-user".to_owned(),
            opened_urls: std::cell::RefCell::new(Vec::new()),
            global_fetch_ready_roots: None,
            fetch_origin_roots: std::cell::RefCell::new(Vec::new()),
            fetch: FetchOutcome {
                branch: "main".to_owned(),
                changed_remote_bookmarks: 1,
                changed_remote_tags: 0,
                abandoned_commits: 0,
                rebased_trunk_children: 1,
                rebased_descendants: 2,
                skipped_trunk_children: 0,
                current_repaired: true,
                rebased_commits: vec![
                    RebasedCommitSummary {
                        old_short_commit_id: "aaaabbbb".to_owned(),
                        new_short_commit_id: "ccccdddd".to_owned(),
                        description: "example change".to_owned(),
                        has_conflict: false,
                        is_empty: false,
                        workspace_visibility: current_workspace_visibility(),
                    },
                    RebasedCommitSummary {
                        old_short_commit_id: "eeeeffff".to_owned(),
                        new_short_commit_id: "12345678".to_owned(),
                        description: "follow-up change".to_owned(),
                        has_conflict: false,
                        is_empty: false,
                        workspace_visibility: current_workspace_visibility(),
                    },
                    RebasedCommitSummary {
                        old_short_commit_id: "9999aaaa".to_owned(),
                        new_short_commit_id: "bbbbcccc".to_owned(),
                        description: "(no description)".to_owned(),
                        has_conflict: false,
                        is_empty: true,
                        workspace_visibility: current_workspace_visibility(),
                    },
                ],
            },
            rebase_on_trunk: RebaseOnTrunkOutcome {
                branch: "main".to_owned(),
                source_short_commit_ids: vec!["a1b2c3d4".to_owned()],
                trunk_short_commit_id: "11112222".to_owned(),
                rebased_commits: 2,
                skipped_commits: 0,
                current_updated: true,
            },
            expected_rebase_sources: None,
            bookmark_update: BookmarkUpdate {
                branch: "example-user/ABC-123-02-a1b2c3d4".to_owned(),
                created: true,
            },
            push: PushOutcome {
                branch: "example-user/ABC-123-02-a1b2c3d4".to_owned(),
                pushed_refs: 1,
                pushed_commits: vec![PushedCommitSummary {
                    short_commit_id: "a1b2c3d4".to_owned(),
                    description: "example change".to_owned(),
                }],
            },
            advance_trunk: AdvanceTrunkOutcome {
                branch: "main".to_owned(),
                old_short_commit_id: "11112222".to_owned(),
                new_short_commit_id: "a1b2c3d4".to_owned(),
                current_updated: true,
            },
            advance_trunk_calls: std::cell::Cell::new(0),
            tracked_push: TrackedPushOutcome {
                pushed_refs: 2,
                bookmarks: vec![
                    PushedBookmarkSummary {
                        branch: "example-user/current".to_owned(),
                        old_short_commit_id: Some("11112222".to_owned()),
                        new_short_commit_id: Some("a1b2c3d4".to_owned()),
                        old_description: Some("previous example change".to_owned()),
                        new_description: Some("example change".to_owned()),
                        new_workspace_visibility: current_workspace_visibility(),
                    },
                    PushedBookmarkSummary {
                        branch: "example-user/old".to_owned(),
                        old_short_commit_id: Some("99990000".to_owned()),
                        new_short_commit_id: None,
                        old_description: Some("obsolete example change".to_owned()),
                        new_description: None,
                        new_workspace_visibility: WorkspaceVisibility::default(),
                    },
                ],
                pushed_commits: vec![PushedCommitSummary {
                    short_commit_id: "a1b2c3d4".to_owned(),
                    description: "example change".to_owned(),
                }],
            },
            sync_pull_requests: Vec::new(),
            pull_request_action: PullRequestAction::Created,
            pull_request_url: Some(
                "https://github.com/example-owner/example-repo/pull/42".to_owned(),
            ),
            existing_pull_request: None,
            reviewer_candidates: Vec::new(),
            expected_reviewers: None,
            expected_task_id: None,
            expected_labels: Vec::new(),
            expected_draft: None,
            expected_clone: None,
            expected_init_repository: None,
            init_repository_calls: std::cell::Cell::new(0),
            expected_workspace_add: None,
            workspaces: vec![WorkspaceEntry {
                name: "default".to_owned(),
                root: PathBuf::from("/workspace"),
                is_current: true,
            }],
            workspace_removes: std::cell::RefCell::new(Vec::new()),
            initial_publish_target: InitialPublishTarget {
                commit_id: "a1b2c3d4e5f6".to_owned(),
                short_commit_id: "a1b2c3d4".to_owned(),
                description: "example change".to_owned(),
            },
            prepared_initial_publish_target: None,
            prepare_initial_publish_calls: std::cell::Cell::new(0),
            created_repository: RepositoryCreation {
                repository: GitHubRepository {
                    owner: "example-owner".to_owned(),
                    name: "example-repo".to_owned(),
                },
                html_url: "https://github.com/example-owner/example-repo".to_owned(),
                private: true,
            },
            create_repository_calls: std::cell::Cell::new(0),
            expected_bootstrap: None,
            bootstrap_push: BootstrapPushOutcome {
                branch: "main".to_owned(),
                short_commit_id: "a1b2c3d4".to_owned(),
                description: "example change".to_owned(),
                working_copy_short_commit_id: Some("bf4799d5".to_owned()),
            },
        }
    }
}

impl FakeServices {
    fn fake_workspace_facts(&self, revision: Option<&str>) -> Result<WorkspaceFacts, JjError> {
        let mut workspace = self.workspace.clone();
        if revision.is_some() {
            workspace.target_change.commit_id = "deadbeefcafebabe".to_owned();
            workspace.target_change.short_commit_id = "deadbeef".to_owned();
            workspace.stack_index = 3;
        }
        Ok(workspace)
    }
}

impl CommandServices for FakeServices {
    fn workspace_log(&self) -> Result<String, JjError> {
        Ok(self.workspace_log.clone())
    }

    fn current_diff(&self, _current_dir: &Path, options: &DiffOptions) -> Result<String, JjError> {
        let tool = match &options.tool {
            DiffToolInvocation::Plain => "plain".to_owned(),
            DiffToolInvocation::External(tool) => format!(
                "external command={} args={}",
                tool.command,
                tool.args.join(",")
            ),
            DiffToolInvocation::Pipe(tool) => format!(
                "pipe producer={} command={} args={}",
                tool.producer_args.join(","),
                tool.command,
                tool.args.join(",")
            ),
        };
        let revision = options
            .revision
            .as_ref()
            .map(|revision| format!(" revision={revision}"))
            .unwrap_or_default();
        Ok(format!(
            "diff:{revision} no_tests={} tool={tool}\n",
            options.no_tests
        ))
    }

    fn clone_repository(&self, _current_dir: &Path, plan: &ClonePlan) -> Result<(), JjError> {
        if let Some((expected_remote, expected_destination)) = &self.expected_clone {
            assert_eq!(&plan.remote_url, expected_remote);
            assert_eq!(&plan.destination, expected_destination);
        }
        Ok(())
    }

    fn init_repository(&self, current_dir: &Path) -> Result<(), JjError> {
        if let Some(expected) = &self.expected_init_repository {
            assert_eq!(current_dir, expected);
        }
        self.init_repository_calls
            .set(self.init_repository_calls.get() + 1);
        let settings = test_settings();
        pollster::block_on(Workspace::init_internal_git(&settings, current_dir))
            .map(|_| ())
            .map_err(|error| JjError::InitFailed {
                status: error.to_string(),
            })
    }

    fn add_workspace(
        &self,
        _current_dir: &Path,
        options: &WorkspaceAddOptions,
    ) -> Result<(), JjError> {
        if let Some(expected) = &self.expected_workspace_add {
            assert_eq!(options, expected);
        }
        Ok(())
    }

    fn workspace_entries(&self, _current_dir: &Path) -> Result<Vec<WorkspaceEntry>, JjError> {
        Ok(self.workspaces.clone())
    }

    fn remove_workspace(
        &self,
        _current_dir: &Path,
        options: &WorkspaceRemoveOptions,
    ) -> Result<(), JjError> {
        self.workspace_removes.borrow_mut().push(options.clone());
        Ok(())
    }

    fn initial_publish_target(
        &self,
        _workspace_root: &Path,
    ) -> Result<InitialPublishTarget, JjError> {
        Ok(self.initial_publish_target.clone())
    }

    fn prepare_initial_publish_target(
        &self,
        _workspace_root: &Path,
        target: &InitialPublishTarget,
    ) -> Result<InitialPublishTarget, JjError> {
        self.prepare_initial_publish_calls
            .set(self.prepare_initial_publish_calls.get() + 1);
        Ok(self
            .prepared_initial_publish_target
            .clone()
            .unwrap_or_else(|| target.clone()))
    }

    fn create_repository(
        &self,
        _context: &LocalRepositoryContext,
        repository: &GitHubRepository,
    ) -> Result<RepositoryCreation, WorkflowError> {
        self.create_repository_calls
            .set(self.create_repository_calls.get() + 1);
        let mut created = self.created_repository.clone();
        created.repository = repository.clone();
        created.html_url = repository.https_url();
        Ok(created)
    }

    fn bootstrap_origin_main(
        &self,
        _workspace_root: &Path,
        remote_url: &str,
        target: &InitialPublishTarget,
    ) -> Result<BootstrapPushOutcome, JjError> {
        if let Some((expected_remote, expected_target)) = &self.expected_bootstrap {
            assert_eq!(remote_url, expected_remote);
            assert_eq!(target, expected_target);
        }
        Ok(self.bootstrap_push.clone())
    }

    fn workspace_status(
        &self,
        _current_dir: &Path,
        _color: bool,
    ) -> Result<WorkspaceStatus, JjError> {
        Ok(self.workspace_status.clone())
    }

    fn workspace_facts(
        &self,
        _context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError> {
        self.fake_workspace_facts(revision)
    }

    fn push_workspace_facts(
        &self,
        _context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError> {
        self.fake_workspace_facts(revision)
    }

    fn check_readiness(
        &self,
        _context: &RepositoryContext,
        _workspace: WorkspaceFacts,
    ) -> Result<CheckReport, WorkflowError> {
        Ok(self.check.clone())
    }

    fn status_workspace_facts(
        &self,
        _context: &RepositoryContext,
    ) -> Result<StatusWorkspaceFacts, JjError> {
        Ok(self.status_workspace.clone())
    }

    fn status_report(
        &self,
        context: &RepositoryContext,
        _workspace: StatusWorkspaceFacts,
    ) -> Result<StatusReport, WorkflowError> {
        let mut status = self.status.clone();
        if self.status_uses_context_remotes {
            for remote in &mut status.remotes {
                if let Some(context_remote) = context
                    .github_remotes
                    .iter()
                    .find(|context_remote| context_remote.name == remote.name)
                {
                    remote.url = context_remote.url.clone();
                    remote.github_url = context_remote.github.https_url();
                }
            }
        }
        if self
            .clean_status_repos
            .iter()
            .any(|repo| repo == &context.origin.github.name)
        {
            for remote in &mut status.remotes {
                remote.local_ahead_by = 0;
                remote.comparison = StatusComparison {
                    state: StatusState::UpToDate,
                    github_ahead_by: 0,
                    github_behind_by: 0,
                };
            }
        }
        Ok(status)
    }

    fn authenticated_login(&self, _token_source: &TokenSource) -> Result<String, WorkflowError> {
        Ok(self.github_login.clone())
    }

    fn open_url(&self, url: &str) -> io::Result<()> {
        self.opened_urls.borrow_mut().push(url.to_owned());
        Ok(())
    }

    fn global_fetch_ready(&self, context: &RepositoryContext) -> Result<bool, JjError> {
        Ok(self
            .global_fetch_ready_roots
            .as_ref()
            .is_none_or(|roots| roots.contains(&context.workspace_root)))
    }

    fn fetch_origin(&self, context: &RepositoryContext) -> Result<FetchOutcome, JjError> {
        self.fetch_origin_roots
            .borrow_mut()
            .push(context.workspace_root.clone());
        Ok(self.fetch.clone())
    }

    fn rebase_on_trunk(
        &self,
        _context: &RepositoryContext,
        sources: &[String],
    ) -> Result<RebaseOnTrunkOutcome, JjError> {
        if let Some(expected) = &self.expected_rebase_sources {
            assert_eq!(sources, expected.as_slice());
        }
        Ok(self.rebase_on_trunk.clone())
    }

    fn ensure_bookmark(
        &self,
        _context: &RepositoryContext,
        branch: &str,
        _target_commit_id: &str,
    ) -> Result<BookmarkUpdate, JjError> {
        let mut update = self.bookmark_update.clone();
        update.branch = branch.to_owned();
        Ok(update)
    }

    fn push_bookmark(
        &self,
        _context: &RepositoryContext,
        branch: &str,
    ) -> Result<PushOutcome, JjError> {
        let mut push = self.push.clone();
        push.branch = branch.to_owned();
        Ok(push)
    }

    fn advance_trunk_for_sync(
        &self,
        _context: &RepositoryContext,
    ) -> Result<AdvanceTrunkOutcome, JjError> {
        self.advance_trunk_calls
            .set(self.advance_trunk_calls.get() + 1);
        Ok(self.advance_trunk.clone())
    }

    fn push_tracked(&self, _context: &RepositoryContext) -> Result<TrackedPushOutcome, JjError> {
        Ok(self.tracked_push.clone())
    }

    fn sync_pull_requests(
        &self,
        _context: &RepositoryContext,
        _push: &TrackedPushOutcome,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError> {
        Ok(self.sync_pull_requests.clone())
    }

    fn pull_request_plan(
        &self,
        _context: &RepositoryContext,
        workspace: WorkspaceFacts,
        task_id: Option<String>,
        labels: Vec<String>,
        draft: bool,
    ) -> Result<PullRequestPlan, WorkflowError> {
        if let Some(expected) = &self.expected_task_id {
            assert_eq!(&task_id, expected);
        }
        let short = workspace.target_change.short_commit_id.as_str();
        let branch = match task_id.as_deref() {
            Some(task_id) => format!(
                "example-user/{task_id}-{stack_index:02}-{short}",
                stack_index = workspace.stack_index,
            ),
            None => format!(
                "example-user/{stack_index:02}-{short}",
                stack_index = workspace.stack_index,
            ),
        };

        Ok(PullRequestPlan {
            repository: self.check.repository.clone(),
            task_id,
            bookmark: BookmarkPlan {
                branch: branch.clone(),
                action: BookmarkAction::Create,
            },
            target_commit_id: workspace.target_change.commit_id.clone(),
            title: "example change".to_owned(),
            body: "example change".to_owned(),
            changed_files: workspace.changed_files,
            base: workspace
                .nearest_ancestor_bookmark
                .unwrap_or(workspace.origin_branch),
            head: PullRequestHead::same_repository("example-owner", branch),
            labels,
            draft,
            existing_pull_request: self.existing_pull_request.clone(),
            reviewer_candidates: self.reviewer_candidates.clone(),
            reviewers: ReviewerSelection::default(),
        })
    }

    fn publish_pull_request(
        &self,
        _context: &RepositoryContext,
        plan: PullRequestPlan,
        bookmark_update: BookmarkUpdate,
        push: PushOutcome,
    ) -> Result<PullRequestReport, WorkflowError> {
        if let Some(expected) = &self.expected_reviewers {
            assert_eq!(&plan.reviewers, expected);
        }
        assert_eq!(plan.labels, self.expected_labels);
        if let Some(expected) = self.expected_draft {
            assert_eq!(plan.draft, expected);
        }

        Ok(PullRequestReport {
            repository: plan.repository,
            task_id: plan.task_id,
            bookmark: plan.bookmark,
            bookmark_update,
            push,
            action: self.pull_request_action,
            pull_request: PullRequestRecord {
                number: 42,
                title: plan.title,
                body: Some(plan.body),
                head_branch: plan.head.branch.clone(),
                base_branch: plan.base.clone(),
                html_url: self.pull_request_url.clone(),
                draft: plan.draft,
            },
            base: plan.base,
            head: plan.head,
            labels: None,
            reviewers: None,
        })
    }
}

fn create_jj_workspace_marker(root: &Path) {
    fs::create_dir_all(root.join(".jj")).expect("create jj workspace marker");
}

fn project_workspaces(workspace: &TestWorkspace) -> Vec<WorkspaceEntry> {
    vec![
        WorkspaceEntry {
            name: "default".to_owned(),
            root: workspace.home.join("projects/jx"),
            is_current: true,
        },
        WorkspaceEntry {
            name: "fix".to_owned(),
            root: workspace.home.join("projects/.work/jx/fix"),
            is_current: false,
        },
    ]
}

fn expected_workspace_status() -> String {
    "Working copy  (@) : a1b2c3d4 abcdef12\nParent commit (@-): 11112222 33334444 main | parent change\n\nexample change\n\nM src/main.rs\n".to_owned()
}

fn workspace_status() -> WorkspaceStatus {
    WorkspaceStatus {
        commit_lines: vec![
            "Working copy  (@) : a1b2c3d4 abcdef12".to_owned(),
            "Parent commit (@-): 11112222 33334444 main | parent change".to_owned(),
        ],
        description: "example change".to_owned(),
        change_lines: vec!["M src/main.rs".to_owned()],
        extra_lines: Vec::new(),
    }
}

fn workspace_facts() -> WorkspaceFacts {
    WorkspaceFacts {
        workspace_root: "/workspace".into(),
        target_change: ChangeSummary {
            change_id: "zzzzzzzz".to_owned(),
            commit_id: "a1b2c3d4e5f6".to_owned(),
            short_commit_id: "a1b2c3d4".to_owned(),
            description: "example change".to_owned(),
            is_empty: false,
        },
        trunk: TrunkSummary {
            branch: "main".to_owned(),
            commit_id: "1111222233334444".to_owned(),
            short_commit_id: "11112222".to_owned(),
        },
        trunk_git_commit_sha: "1111222233334444".to_owned(),
        origin_branch: "main".to_owned(),
        local_bookmarks: Vec::new(),
        local_bookmarks_at_target: Vec::new(),
        nearest_ancestor_bookmark: Some("example-user/01-ancestor".to_owned()),
        changed_files: vec!["src/main.rs".to_owned()],
        stack_index: 2,
    }
}

fn status_workspace_facts() -> StatusWorkspaceFacts {
    StatusWorkspaceFacts {
        remotes: vec![StatusRemoteFacts {
            remote: "origin".to_owned(),
            branch: "main".to_owned(),
            trunk_git_commit_sha: "1111222233334444".to_owned(),
            trunk_short_commit_id: "11112222".to_owned(),
            local_ahead_by: 0,
        }],
    }
}

fn test_settings() -> UserSettings {
    UserSettings::from_config(StackedConfig::with_defaults()).expect("test settings")
}

fn test_config_remotes(contents: &str) -> Vec<(String, String)> {
    let mut current_remote = None;
    let mut remotes = Vec::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            current_remote = line
                .strip_prefix(r#"[remote "#)
                .and_then(|section| section.strip_prefix('"'))
                .and_then(|section| section.strip_suffix(r#""]"#))
                .map(str::to_owned);
            continue;
        }
        let Some(remote_name) = current_remote.as_deref() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        if key.trim() == "url" {
            remotes.push((
                remote_name.to_owned(),
                value.trim().trim_matches('"').to_owned(),
            ));
        }
    }

    remotes
}

struct TestWorkspace {
    home: PathBuf,
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        Self::new_under("")
    }

    fn new_under(relative_path: &str) -> Self {
        let workspace = Self::new_uninitialized_under(relative_path);
        let settings = test_settings();
        pollster::block_on(Workspace::init_internal_git(&settings, &workspace.root))
            .expect("initialize jj workspace");
        workspace
    }

    fn new_uninitialized_under(relative_path: &str) -> Self {
        let unique = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let home =
            std::env::temp_dir().join(format!("jx-command-test-{}-{unique}", std::process::id()));
        let root = if relative_path.is_empty() {
            home.clone()
        } else {
            home.join(relative_path)
        };
        fs::create_dir_all(&root).expect("create workspace root");
        Self { home, root }
    }

    fn path(&self) -> PathBuf {
        self.root.clone()
    }

    fn home_environment(&self) -> [(String, String); 1] {
        [("HOME".to_owned(), self.home.to_string_lossy().into_owned())]
    }

    fn write_file(&self, relative_path: &str, contents: &str) {
        let path = self.root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, contents).expect("write test file");
    }

    fn write_home_file(&self, relative_path: &str, contents: &str) {
        let path = self.home.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, contents).expect("write home test file");
    }

    fn create_jj_workspace(&self, relative_path: &str) -> PathBuf {
        let root = self.home.join(relative_path);
        fs::create_dir_all(&root).expect("create workspace root");
        let settings = test_settings();
        pollster::block_on(Workspace::init_internal_git(&settings, &root))
            .expect("initialize jj workspace");
        root
    }

    fn write_git_config(&self, contents: &str) {
        Self::write_git_config_at(&self.root, contents);
    }

    fn write_git_config_at(root: &Path, contents: &str) {
        for (name, url) in test_config_remotes(contents) {
            let settings = test_settings();
            let store_factories = StoreFactories::default();
            let working_copy_factories = default_working_copy_factories();
            let workspace =
                Workspace::load(&settings, root, &store_factories, &working_copy_factories)
                    .expect("load jj workspace");
            let repo =
                pollster::block_on(workspace.repo_loader().load_at_head()).expect("load jj repo");
            let mut tx = repo.start_transaction();

            git::add_remote(
                tx.repo_mut(),
                RemoteName::new(&name),
                &url,
                None,
                gix::remote::fetch::Tags::None,
            )
            .expect("add remote");
            pollster::block_on(tx.commit(format!("arrange test remote {name}")))
                .expect("commit remote");
        }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}
