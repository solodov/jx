use super::*;
use crate::{
    domain::{
        BookmarkAction, BookmarkPlan, CheckWorkspaceSummary, ForkStatusComparison, GitHubReadiness,
        PullRequestAction, RepositorySummary, StatusComparison, StatusState,
    },
    github::{PullRequestHead, PullRequestRecord, ReviewerSelection},
    jj::{
        ChangeSummary, PushedBookmarkSummary, PushedCommitSummary, RebaseOnTrunkOutcome,
        RebasedCommitSummary, StatusRemoteFacts, StatusWorkspaceFacts, TrackedPushOutcome,
        TrunkSummary, WorkspaceAddOptions, WorkspaceEntry, WorkspaceRemoveOptions, WorkspaceStatus,
        WorkspaceVisibility,
    },
    repository::StackMetadataNode,
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
    collections::{BTreeMap, BTreeSet},
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

fn fork_status(
    state: ForkStatusState,
    source_ahead_by: i64,
    fork_ahead_by: i64,
) -> ForkStatusReport {
    ForkStatusReport {
        fork: GitHubRepository {
            owner: "example-owner".to_owned(),
            name: "example-repo".to_owned(),
        },
        fork_branch: "main".to_owned(),
        source: GitHubRepository {
            owner: "source-owner".to_owned(),
            name: "example-repo".to_owned(),
        },
        source_branch: "main".to_owned(),
        comparison: ForkStatusComparison {
            state,
            source_ahead_by,
            fork_ahead_by,
        },
    }
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
fn clone_infers_owner_from_current_layout_prefix() {
    // Verifies: Repo-only clone shorthands use the cwd when it supplies the configured slug prefix.
    let workspace = TestWorkspace::new_uninitialized_under("projects");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "solodov"
root = "~/projects"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/termflow");
    let services = FakeServices {
        expected_clone: Some((
            "git@github.com:solodov/termflow.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "clone", "termflow"], &environment, &services)
        .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/projects/termflow\n",
            osc8_link(
                "https://github.com/solodov/termflow",
                "git@github.com:solodov/termflow.git"
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
            shared_paths: Vec::new(),
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
fn work_add_from_managed_workspace_invokes_jj_from_primary_checkout() {
    // Verifies: Workspace add planning runs jj from the primary checkout, not the current managed workspace.
    let workspace = TestWorkspace::new_under("projects/.work/jx/current");
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
    let primary = workspace.home.join("projects/jx");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let services = FakeServices {
        expected_workspace_add_current_dir: Some(primary),
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: Vec::new(),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "work", "add", "fix"], &environment, &services)
        .expect("workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!("Added workspace: {}\n", expected_destination.display())
    );
}

#[test]
fn work_add_plan_splits_shared_path_sources_from_primary_checkout() {
    // Verifies: Existing shared paths are linked from the primary checkout and missing paths are skipped.
    let workspace = TestWorkspace::new_under("projects/.work/jx/current");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = [".pi", "nested/state", "missing/state"]
"#,
    );
    workspace.write_home_file("projects/jx/.pi/config.toml", "pi state");
    workspace.write_home_file("projects/jx/nested/state", "nested state");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let context = LocalRepositoryContext::discover(&environment).expect("context discovers");
    let request = WorkAddRequest {
        name: "fix".to_owned(),
        revision: Some("main".to_owned()),
        task_id: Some("ABC-123".to_owned()),
        shell_cd_target: false,
    };
    let primary = workspace.home.join("projects/jx");
    let destination = workspace.home.join("projects/.work/jx/ABC-123-fix");

    let plan = plan_work_add(&request, &context, &environment).expect("plan builds");

    assert_eq!(
        plan.identity,
        RepositoryIdentity {
            source: "github".to_owned(),
            host: "github.com".to_owned(),
            owner: "example-owner".to_owned(),
            repo: "jx".to_owned(),
        }
    );
    assert_eq!(plan.primary_checkout_root, primary);
    assert_eq!(plan.destination, destination);
    assert_eq!(plan.workspace_name, "ABC-123-fix");
    assert_eq!(plan.revision.as_deref(), Some("main"));
    assert_eq!(plan.task_id.as_deref(), Some("ABC-123"));
    assert_eq!(
        plan.shared_paths.effective_paths,
        vec![".pi", "nested/state", "missing/state"]
    );
    assert_eq!(
        plan.shared_paths.link_candidates,
        vec![
            SharedWorkspacePathCandidate {
                relative_path: ".pi".to_owned(),
                source: primary.join(".pi"),
                destination: destination.join(".pi"),
            },
            SharedWorkspacePathCandidate {
                relative_path: "nested/state".to_owned(),
                source: primary.join("nested/state"),
                destination: destination.join("nested/state"),
            },
        ]
    );
    assert_eq!(
        plan.shared_paths.missing_sources,
        vec![MissingSharedWorkspacePath {
            relative_path: "missing/state".to_owned(),
            source: primary.join("missing/state"),
        }]
    );
    assert_eq!(
        plan.workspace_options().shared_paths,
        vec![".pi".to_owned(), "nested/state".to_owned()]
    );
}

#[test]
fn work_add_shell_cd_target_prints_new_workspace_path() {
    // Verifies: Shell integration can enter a newly added managed workspace after creation.
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
            revision: None,
            shared_paths: Vec::new(),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "add", "--shell-cd-target", "fix"],
        &environment,
        &services,
    )
    .expect("workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Added workspace: {}\n{}{}\n",
            expected_destination.display(),
            SHELL_CD_TARGET_PREFIX,
            expected_destination.display()
        )
    );
}

#[test]
fn work_add_creation_failure_stops_before_metadata_setup() {
    // Verifies: If jj workspace creation fails, post-create setup is not attempted.
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
            shared_paths: Vec::new(),
        }),
        workspace_add_error: Some("simulated creation failure".to_owned()),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(
        ["jx", "work", "add", "fix", "--task-id", "ABC-123"],
        &environment,
        &services,
    )
    .expect_err("workspace add fails");

    assert!(matches!(
        error,
        CommandError::Jj(JjError::WorkspaceAddFailed { status }) if status == "simulated creation failure"
    ));
    assert!(!expected_destination.join(".jx/workspace.toml").exists());
}

#[test]
fn work_add_post_create_setup_failure_reports_without_rollback() {
    // Verifies: Setup failures after jj creation surface an error and leave the created workspace in place.
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
            shared_paths: Vec::new(),
        }),
        workspace_add_metadata_blocker: Some(expected_destination.clone()),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(
        ["jx", "work", "add", "fix", "--task-id", "ABC-123"],
        &environment,
        &services,
    )
    .expect_err("post-create setup fails");

    assert!(matches!(
        error,
        CommandError::WorkAddSetup { ref workspace, ref destination, ref message }
            if workspace == "ABC-123-fix"
                && destination == &expected_destination
                && message.contains("Could not write workspace metadata")
    ));
    assert!(expected_destination.is_dir());
    assert!(expected_destination.join(".jx").is_file());
}

#[cfg(unix)]
#[test]
fn work_add_links_shared_paths_and_creates_nested_parents() {
    // Verifies: Existing shared-path candidates are symlinked into the created workspace.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = [".pi", "nested/state", ".local-link", "missing/state"]
"#,
    );
    workspace.write_file(".pi/settings.toml", "pi state");
    workspace.write_file("nested/state", "nested state");
    workspace.write_file("real-target", "real state");
    std::os::unix::fs::symlink(
        workspace.path().join("real-target"),
        workspace.path().join(".local-link"),
    )
    .expect("create source symlink");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: vec![
                ".pi".to_owned(),
                "nested/state".to_owned(),
                ".local-link".to_owned(),
            ],
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "work", "add", "fix"], &environment, &services)
        .expect("workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!("Added workspace: {}\n", expected_destination.display())
    );
    assert_eq!(
        fs::read_link(expected_destination.join(".pi")).expect(".pi symlink"),
        workspace.path().join(".pi")
    );
    assert_eq!(
        fs::read_link(expected_destination.join("nested/state")).expect("nested symlink"),
        workspace.path().join("nested/state")
    );
    assert_eq!(
        fs::read_link(expected_destination.join(".local-link")).expect("source symlink linked"),
        workspace.path().join(".local-link")
    );
    assert!(expected_destination.join("nested").is_dir());
    assert!(!expected_destination.join("missing/state").exists());
}

#[test]
fn work_add_shared_path_setup_reports_blocked_nested_parent() {
    // Verifies: Nested shared-path parent creation reports filesystem blockers clearly.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = ["nested/state"]
"#,
    );
    workspace.write_file("nested/state", "nested state");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let parent_blocker = expected_destination.join("nested");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: vec!["nested/state".to_owned()],
        }),
        workspace_add_existing_shared_path: Some(parent_blocker.clone()),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "work", "add", "fix"], &environment, &services)
        .expect_err("shared path setup fails");

    assert!(matches!(
        error,
        CommandError::WorkAddSetup { ref workspace, ref destination, ref message }
            if workspace == "fix"
                && destination == &expected_destination
                && message.contains("create parent directories")
                && message.contains("nested/state")
    ));
    assert_eq!(
        fs::read_to_string(&parent_blocker).expect("parent blocker remains"),
        "existing content"
    );
}

#[test]
fn work_add_shared_path_setup_failure_keeps_workspace_and_existing_content() {
    // Verifies: Shared-path setup fails conservatively without rollback or overwrite.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = [".pi"]
"#,
    );
    workspace.write_file(".pi/settings.toml", "pi state");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let existing_shared_path = expected_destination.join(".pi");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: vec![".pi".to_owned()],
        }),
        workspace_add_existing_shared_path: Some(existing_shared_path.clone()),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "work", "add", "fix"], &environment, &services)
        .expect_err("shared path setup fails");

    assert!(matches!(
        error,
        CommandError::WorkAddSetup { ref workspace, ref destination, ref message }
            if workspace == "fix"
                && destination == &expected_destination
                && message.contains("destination already exists")
    ));
    assert!(expected_destination.is_dir());
    assert_eq!(
        fs::read_to_string(&existing_shared_path).expect("existing content remains"),
        "existing content"
    );
}

#[test]
fn work_add_shell_cd_target_setup_failure_returns_error_without_cd_target() {
    // Verifies: Shell integration receives no hidden cd target when post-create setup fails.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = [".pi"]
"#,
    );
    workspace.write_file(".pi/settings.toml", "pi state");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let existing_shared_path = expected_destination.join(".pi");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: vec![".pi".to_owned()],
        }),
        workspace_add_existing_shared_path: Some(existing_shared_path),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(
        ["jx", "work", "add", "--shell-cd-target", "fix"],
        &environment,
        &services,
    )
    .expect_err("shared path setup fails");

    assert!(matches!(
        error,
        CommandError::WorkAddSetup { ref workspace, ref destination, .. }
            if workspace == "fix" && destination == &expected_destination
    ));
    assert!(!error.to_string().contains(SHELL_CD_TARGET_PREFIX));
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
            shared_paths: Vec::new(),
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
        "/.gitignore\n/workspace.toml\n/stack.toml\n"
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
fn work_complete_workspaces_lists_only_deletable_workspace_names() {
    // Verifies: Delete completion offers managed workspaces but omits the primary default checkout.
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
    let mut workspaces = project_workspaces(&workspace);
    workspaces.push(WorkspaceEntry {
        name: "review".to_owned(),
        root: workspace.home.join("projects/.work/jx/review"),
        is_current: false,
    });
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--workspaces", "--prefix", ""],
        &environment,
        &services,
    )
    .expect("workspace completion succeeds");

    assert_eq!(result.stdout, "fix\nreview\n");
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
fn work_root_navigation_prefers_current_workspace_names_over_global_keys() {
    // Verifies: `u name` resolves same-repository workspaces before unrelated global keys.
    let workspace = TestWorkspace::new_under("projects/.work/project/current");
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
    let sibling_root = workspace.home.join("projects/.work/project/other");
    let unrelated_root = workspace.home.join("projects/other");
    create_jj_workspace_marker(&sibling_root);
    create_jj_workspace_marker(&unrelated_root);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "current".to_owned(),
                root: workspace.path(),
                is_current: true,
            },
            WorkspaceEntry {
                name: "other".to_owned(),
                root: sibling_root.clone(),
                is_current: false,
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "other"],
        &environment,
        &services,
    )
    .expect("navigation root succeeds");

    assert_eq!(result.stdout, format!("{}\n", sibling_root.display()));
}

#[test]
fn work_root_navigation_resolves_unique_location_fragment() {
    // Verifies: `u fragment` can resolve one layout location without requiring a prefix match.
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
    let expected_root = workspace.home.join("projects/termflow");
    create_jj_workspace_marker(&expected_root);
    create_jj_workspace_marker(&workspace.home.join("projects/other"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "flow"],
        &environment,
        &services,
    )
    .expect("navigation root succeeds");

    assert_eq!(result.stdout, format!("{}\n", expected_root.display()));
}

#[test]
fn work_root_navigation_resolves_fragment_subpaths() {
    // Verifies: Slash-separated `u` fragments can pick a repo and nested directory by partial names.
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
    let termflow = workspace.home.join("projects/termflow");
    create_jj_workspace_marker(&termflow);
    fs::create_dir_all(termflow.join("bin")).expect("create bin directory");
    fs::create_dir_all(termflow.join("hammerspoon")).expect("create hammerspoon directory");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let bin = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "flow/bin"],
        &environment,
        &services,
    )
    .expect("bin navigation root succeeds");
    let hammerspoon = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "term/spoon"],
        &environment,
        &services,
    )
    .expect("hammerspoon navigation root succeeds");

    assert_eq!(bin.stdout, format!("{}\n", termflow.join("bin").display()));
    assert_eq!(
        hammerspoon.stdout,
        format!("{}\n", termflow.join("hammerspoon").display())
    );
}

#[test]
fn work_root_navigation_accepts_explicit_paths() {
    // Verifies: `u` treats absolute and dot-relative paths as filesystem navigation before fuzzy matching.
    let workspace = TestWorkspace::new_uninitialized_under("projects/current");
    let relative_target = workspace.home.join("projects/foo");
    let absolute_target = workspace.home.join("absolute-target");
    fs::create_dir_all(&relative_target).expect("create relative target");
    fs::create_dir_all(&absolute_target).expect("create absolute target");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let relative = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "../foo"],
        &environment,
        &services,
    )
    .expect("relative path navigation succeeds");
    let absolute_arg = absolute_target.to_string_lossy().into_owned();
    let absolute = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", absolute_arg.as_str()],
        &environment,
        &services,
    )
    .expect("absolute path navigation succeeds");

    assert_eq!(
        relative.stdout,
        format!("{}\n", fs::canonicalize(relative_target).unwrap().display())
    );
    assert_eq!(
        absolute.stdout,
        format!("{}\n", fs::canonicalize(absolute_target).unwrap().display())
    );
}

#[test]
fn work_root_navigation_rejects_ambiguous_fragments() {
    // Verifies: Fuzzy navigation only succeeds when the fragment selects one best target.
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
    create_jj_workspace_marker(&workspace.home.join("projects/termflow"));
    create_jj_workspace_marker(&workspace.home.join("projects/terminal"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "term"],
        &environment,
        &services,
    )
    .expect_err("ambiguous navigation roots are rejected");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::WorkLocationAmbiguous { .. })
    ));
}

#[test]
fn work_complete_navigation_orders_current_repo_before_global_locations() {
    // Verifies: navigation completion presents local workspace names and trunk aliases first.
    let workspace = TestWorkspace::new_under("projects/.work/project/current");
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
    let primary_root = workspace.home.join("projects/project");
    let sibling_root = workspace.home.join("projects/.work/project/fix");
    let unrelated_root = workspace.home.join("projects/other");
    create_jj_workspace_marker(&primary_root);
    create_jj_workspace_marker(&sibling_root);
    create_jj_workspace_marker(&unrelated_root);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "current".to_owned(),
                root: workspace.path(),
                is_current: true,
            },
            WorkspaceEntry {
                name: "fix".to_owned(),
                root: sibling_root,
                is_current: false,
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--navigation", "--prefix", ""],
        &environment,
        &services,
    )
    .expect("navigation completion succeeds");

    assert_eq!(
        result.stdout,
        "current\nfix\ndefault\ntrunk\nroot\nproject\nproject@current\nproject@fix\nother\n"
    );

    let default = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "default"],
        &environment,
        &services,
    )
    .expect("default navigation root succeeds");
    assert_eq!(default.stdout, format!("{}\n", primary_root.display()));
}

#[test]
fn work_trunk_resolves_primary_checkout_from_current_workspace() {
    // Verifies: A managed workspace can resolve the trunk checkout for quick navigation back.
    let workspace = TestWorkspace::new_under("projects/.work/tool/fix");
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
    let trunk = workspace.home.join("projects/tool");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "work", "trunk"], &environment, &services)
        .expect("work trunk succeeds");

    assert_eq!(result.stdout, format!("{}\n", trunk.display()));
}

#[test]
fn work_trunk_shell_cd_target_prints_only_trunk_checkout_target() {
    // Verifies: Shell integration can jump to the trunk checkout without printing an extra path row.
    let workspace = TestWorkspace::new_under("projects/.work/tool/fix");
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
    let trunk = workspace.home.join("projects/tool");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "trunk", "--shell-cd-target"],
        &environment,
        &services,
    )
    .expect("work trunk shell target succeeds");

    assert_eq!(
        result.stdout,
        format!("{}{}\n", SHELL_CD_TARGET_PREFIX, trunk.display())
    );
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

#[test]
fn work_delete_can_be_cancelled() {
    // Verifies: Declining deletion stops before jj forget or filesystem deletion.
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
        ["jx", "work", "delete", "fix"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("workspace deletion cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
    assert!(services.workspace_removes.borrow().is_empty());
}

#[test]
fn work_delete_forgets_and_deletes_managed_workspace_after_confirmation() {
    // Verifies: Deletion delegates one confirmed forget/delete operation for managed workspaces.
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
        ["jx", "work", "delete", "fix"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("workspace deletion succeeds");

    assert_eq!(result.stdout, "Deleted workspace: fix\n");
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
fn work_delete_current_managed_workspace_returns_shell_cd_target() {
    // Verifies: Deleting the active managed workspace runs from the trunk checkout and tells shell integration where to land.
    let workspace = TestWorkspace::new_under("projects/.work/tool/fix");
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
    let primary = workspace.home.join("projects/tool");
    let managed = workspace.home.join("projects/.work/tool/fix");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "default".to_owned(),
                root: primary.clone(),
                is_current: false,
            },
            WorkspaceEntry {
                name: "fix".to_owned(),
                root: managed.clone(),
                is_current: true,
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "delete", "--shell-cd-target"],
        &environment,
        &services,
    )
    .expect("current managed workspace deletion succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Deleted workspace: fix\n{}{}\n",
            SHELL_CD_TARGET_PREFIX,
            primary.display()
        )
    );
    assert_eq!(
        services.workspace_remove_current_dirs.borrow().as_slice(),
        [primary]
    );
    assert_eq!(
        services.workspace_removes.borrow().as_slice(),
        [WorkspaceRemoveOptions {
            name: "fix".to_owned(),
            root: managed,
            cleanup_root: workspace.home.join("projects/.work"),
        }]
    );
}

#[test]
fn work_delete_refuses_primary_and_unmanaged_paths() {
    // Verifies: Delete only targets workspaces inside the managed `.work` layout.
    enum RootKind {
        Primary,
        Unmanaged,
    }

    enum ExpectedError {
        Primary,
        Unmanaged,
    }

    let cases = [
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
            ["jx", "work", "delete", target.name.as_str()],
            &environment,
            &services,
        )
        .expect_err("unsafe workspace deletion is rejected");

        match expected_error {
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
fn diff_accepts_paths_before_trailing_tool_args() {
    // Verifies: Paths remain jj diff filters while `--` still separates renderer arguments.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".jx.toml",
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
fn open_pr_prints_first_candidate_with_pull_request() {
    // Verifies: Open PR follows candidate bookmark order until GitHub has a pull request to open.
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
    let services = FakeServices {
        open_pull_request_candidates: vec![
            "topic/without-review".to_owned(),
            "topic/with-review".to_owned(),
        ],
        pull_requests_by_head: BTreeMap::from([(
            "topic/with-review".to_owned(),
            PullRequestRecord {
                number: 24,
                title: "Reviewable change".to_owned(),
                body: None,
                head_branch: "topic/with-review".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some("https://github.com/example-owner/example-repo/pull/24".to_owned()),
                draft: false,
                merged: false,
            },
        )]),
        ..Default::default()
    };

    let result =
        run_with_args_and_services(["jx", "open", "pr", "--print"], &environment, &services)
            .expect("open pr succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo/pull/24\n"
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [None]
    );
    assert_eq!(
        services.pull_request_head_calls.borrow().as_slice(),
        ["topic/without-review", "topic/with-review"]
    );
    assert!(services.opened_urls.borrow().is_empty());
}

#[test]
fn open_pr_accepts_positional_commit_or_bookmark_selector() {
    // Verifies: Open PR accepts the selector UX used for commit prefixes and slash bookmarks.
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
    let services = FakeServices {
        open_pull_request_candidates: vec!["example-user/00-1977d9cd".to_owned()],
        pull_requests_by_head: BTreeMap::from([(
            "example-user/00-1977d9cd".to_owned(),
            PullRequestRecord {
                number: 1977,
                title: "Selected change".to_owned(),
                body: None,
                head_branch: "example-user/00-1977d9cd".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some(
                    "https://github.com/example-owner/example-repo/pull/1977".to_owned(),
                ),
                draft: false,
                merged: false,
            },
        )]),
        ..Default::default()
    };

    let result = run_with_args_and_services(
        ["jx", "open", "pr", "--print", "example-user/00-1977d9cd"],
        &environment,
        &services,
    )
    .expect("open pr succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo/pull/1977\n"
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [Some("example-user/00-1977d9cd".to_owned())]
    );
}

#[test]
fn open_pr_accepts_commit_option_as_selector() {
    // Verifies: The legacy --commit option keeps working while using selector resolution.
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
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/with-review".to_owned()],
        pull_requests_by_head: BTreeMap::from([(
            "topic/with-review".to_owned(),
            PullRequestRecord {
                number: 25,
                title: "Selected change".to_owned(),
                body: None,
                head_branch: "topic/with-review".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some("https://github.com/example-owner/example-repo/pull/25".to_owned()),
                draft: false,
                merged: false,
            },
        )]),
        ..Default::default()
    };

    let result = run_with_args_and_services(
        ["jx", "open", "pr", "--print", "--commit", "abc123"],
        &environment,
        &services,
    )
    .expect("open pr succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo/pull/25\n"
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [Some("abc123".to_owned())]
    );
}

#[test]
fn open_pr_interactive_opens_selected_local_bookmark_pull_request() {
    // Verifies: Interactive Open PR lists PRs attached to local bookmarks and opens the selected one.
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
    let services = FakeServices {
        pull_request_bookmarks: vec![
            "example-user/old".to_owned(),
            "example-user/chosen".to_owned(),
        ],
        authored_open_pull_requests_by_head: BTreeMap::from([
            (
                "example-user/old".to_owned(),
                PullRequestRecord {
                    number: 10,
                    title: "Older change".to_owned(),
                    body: None,
                    head_branch: "example-user/old".to_owned(),
                    base_branch: "main".to_owned(),
                    html_url: Some(
                        "https://github.com/example-owner/example-repo/pull/10".to_owned(),
                    ),
                    draft: false,
                    merged: false,
                },
            ),
            (
                "example-user/chosen".to_owned(),
                PullRequestRecord {
                    number: 11,
                    title: "Chosen change".to_owned(),
                    body: None,
                    head_branch: "example-user/chosen".to_owned(),
                    base_branch: "main".to_owned(),
                    html_url: Some(
                        "https://github.com/example-owner/example-repo/pull/11".to_owned(),
                    ),
                    draft: true,
                    merged: false,
                },
            ),
        ]),
        ..Default::default()
    };
    let selector = RecordingPullRequestSelector::new(1);

    let result = run_with_args_and_pull_request_selector(
        ["jx", "open", "pr", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive open pr succeeds");

    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/example-repo/pull/11\n"
    );
    assert_eq!(
        selector.labels.borrow().as_slice(),
        &[vec![
            "◯ #10     Older change".to_owned(),
            "\x1b[2m\x1b[38;2;150;142;132m◌ #11     Chosen change\x1b[0m".to_owned(),
        ]]
    );
    assert_eq!(
        services.opened_urls.borrow().as_slice(),
        ["https://github.com/example-owner/example-repo/pull/11"]
    );
    assert_eq!(
        services
            .authored_open_pull_request_head_calls
            .borrow()
            .as_slice(),
        [
            ("example-user/old".to_owned(), "example-user".to_owned()),
            ("example-user/chosen".to_owned(), "example-user".to_owned())
        ]
    );
    assert!(services.pull_request_head_calls.borrow().is_empty());
}

#[test]
fn open_pr_interactive_uses_stack_snapshot_component() {
    // Verifies: Interactive Open PR shows stored stack context around the current local PR.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                draft: false,
                merged: true,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/child".to_owned()],
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        authored_open_pull_requests_by_head: BTreeMap::from([(
            "topic/child".to_owned(),
            PullRequestRecord {
                number: 11,
                title: "Child".to_owned(),
                body: None,
                head_branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                html_url: Some("https://github.com/example-owner/example-repo/pull/11".to_owned()),
                draft: false,
                merged: false,
            },
        )]),
        ..Default::default()
    };
    let selector = RecordingPullRequestSelector::new(1);

    let result = run_with_args_and_pull_request_selector(
        ["jx", "open", "pr", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive open pr succeeds");

    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/example-repo/pull/11\n"
    );
    assert_eq!(
        selector.labels.borrow().as_slice(),
        &[vec![
            "✓ #10     Root".to_owned(),
            "└─ ◉ #11     Child".to_owned(),
        ]]
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [None]
    );
    assert_eq!(
        services
            .authored_open_pull_request_head_calls
            .borrow()
            .as_slice(),
        [("topic/child".to_owned(), "example-user".to_owned())]
    );
}

#[test]
fn open_pr_interactive_prints_selected_pull_request_url() {
    // Verifies: --print still suppresses browser launch after interactive PR selection.
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
    let services = FakeServices {
        pull_request_bookmarks: vec!["example-user/selected".to_owned()],
        authored_open_pull_requests_by_head: BTreeMap::from([(
            "example-user/selected".to_owned(),
            PullRequestRecord {
                number: 12,
                title: "Selected change".to_owned(),
                body: None,
                head_branch: "example-user/selected".to_owned(),
                base_branch: "main".to_owned(),
                html_url: None,
                draft: false,
                merged: false,
            },
        )]),
        ..Default::default()
    };
    let selector = FixedPullRequestSelector { selected: 0 };

    let result = run_with_args_and_pull_request_selector(
        ["jx", "open", "pr", "--interactive", "--print"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive open pr succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo/pull/12\n"
    );
    assert!(services.opened_urls.borrow().is_empty());
}

#[test]
fn open_pr_interactive_ignores_historical_or_unowned_pull_requests() {
    // Verifies: Interactive Open PR only accepts open authored PRs attached to local bookmarks.
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
    let services = FakeServices {
        pull_request_bookmarks: vec!["example-user/reused".to_owned()],
        pull_requests_by_head: BTreeMap::from([(
            "example-user/reused".to_owned(),
            PullRequestRecord {
                number: 40568,
                title: "Merged or unowned historical PR".to_owned(),
                body: None,
                head_branch: "example-user/reused".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some(
                    "https://github.com/example-owner/example-repo/pull/40568".to_owned(),
                ),
                draft: false,
                merged: false,
            },
        )]),
        ..Default::default()
    };
    let selector = FixedPullRequestSelector { selected: 0 };

    let error = run_with_args_and_pull_request_selector(
        ["jx", "open", "pr", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect_err("historical or unowned pull requests are ignored");

    assert!(matches!(
        error,
        CommandError::Workflow(WorkflowError::MissingLocalBookmarkPullRequests { repository })
            if repository == "example-owner/example-repo"
    ));
    assert_eq!(
        services
            .authored_open_pull_request_head_calls
            .borrow()
            .as_slice(),
        [("example-user/reused".to_owned(), "example-user".to_owned())]
    );
    assert!(services.pull_request_head_calls.borrow().is_empty());
}

#[test]
fn open_pr_interactive_reports_missing_local_bookmark_pull_requests() {
    // Verifies: Interactive Open PR reports when local bookmarks have no PRs.
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
    let selector = FixedPullRequestSelector { selected: 0 };

    let error = run_with_args_and_pull_request_selector(
        ["jx", "open", "pr", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect_err("missing pull requests are reported");

    assert!(matches!(
        error,
        CommandError::Workflow(WorkflowError::MissingLocalBookmarkPullRequests { repository })
            if repository == "example-owner/example-repo"
    ));
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
fn stack_track_persists_hierarchy_and_ignore_rules() {
    // Verifies: stack tracking records PR hierarchy in repo-local ignored metadata.
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
        pull_request_bookmarks: vec![
            "topic/child".to_owned(),
            "topic/root".to_owned(),
            "topic/draft".to_owned(),
        ],
        authored_open_pull_requests_by_head: BTreeMap::from([
            (
                "topic/root".to_owned(),
                pull_request_choice_record(10, "Root", "topic/root", "main", false),
            ),
            (
                "topic/child".to_owned(),
                pull_request_choice_record(11, "Child", "topic/child", "topic/root", false),
            ),
            (
                "topic/draft".to_owned(),
                pull_request_choice_record(12, "Draft", "topic/draft", "topic/root", true),
            ),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "track"], &environment, &services)
        .expect("stack tracking succeeds");

    assert_eq!(
        result.stdout,
        "◯ #10     Root\n├─ ◯ #11     Child\n└─ ◌ #12     Draft\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join(".jx/.gitignore")).expect("read gitignore"),
        "/.gitignore\n/workspace.toml\n/stack.toml\n"
    );
    let stack_file =
        fs::read_to_string(workspace.path().join(".jx/stack.toml")).expect("read stack state");
    assert!(stack_file.contains("pull_request = 10"));
    assert!(stack_file.contains("parent_branch = \"topic/root\""));
}

#[test]
fn stack_reset_removes_state_but_keeps_ignore_rules() {
    // Verifies: reset removes durable stack state without exposing future metadata to Git.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    fs::create_dir_all(workspace.path().join(".jx")).expect("create metadata directory");
    fs::write(workspace.path().join(".jx/stack.toml"), "version = 1\n").expect("write stack file");

    let result = run_with_args_and_services(
        ["jx", "stack", "reset"],
        &environment,
        &FakeServices::default(),
    )
    .expect("stack reset succeeds");

    assert_eq!(result.stdout, "Stack state reset\n");
    assert!(!workspace.path().join(".jx/stack.toml").exists());
    assert_eq!(
        fs::read_to_string(workspace.path().join(".jx/.gitignore")).expect("read gitignore"),
        "/.gitignore\n/workspace.toml\n/stack.toml\n"
    );
}

#[test]
fn stack_without_subcommand_shows_state() {
    // Verifies: bare `jx stack` uses the safe read-only stack view.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result =
        run_with_args_and_services(["jx", "stack"], &environment, &FakeServices::default())
            .expect("stack show succeeds");

    assert_eq!(result.stdout, "◯ #10     Root\n");
}

#[test]
fn stack_show_reads_primary_checkout_state_from_managed_workspace() {
    // Verifies: stack state is repo-local even when the command runs from a managed workspace.
    let workspace = TestWorkspace::new_under("projects/.work/jx/current");
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
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let primary = workspace.home.join("projects/jx");
    write_stack_metadata(
        &primary,
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let result = run_with_args_and_services(
        ["jx", "stack", "show"],
        &environment,
        &FakeServices::default(),
    )
    .expect("stack show succeeds");

    assert_eq!(result.stdout, "◯ #10     Root\n");
    assert!(!workspace.path().join(".jx/stack.toml").exists());
}

#[test]
fn stack_tracking_retains_missing_stored_ancestors() {
    // Verifies: disappeared parents remain in stack state while children are still tracked.
    let existing = StackMetadata {
        version: 1,
        nodes: vec![
            StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: true,
            },
            StackMetadataNode {
                branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                parent_branch: Some("topic/root".to_owned()),
                pull_request: Some(11),
                parent_pull_request: Some(10),
                title: "Old child".to_owned(),
                url: None,
                draft: false,
                merged: false,
            },
        ],
    };
    let child = pull_request_choice_record(11, "Child", "topic/child", "main", false);

    let metadata = stack_metadata_from_pull_requests(&[child], &existing);

    assert_eq!(
        stack_metadata_rows(&metadata.nodes),
        vec!["✓ #10     Root", "└─ ◯ #11     Child"]
    );
}

#[test]
fn stack_track_refreshes_missing_stored_ancestor_by_pull_request_number() {
    // Verifies: stack tracking refreshes disappeared parent metadata from its durable PR number.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Stale root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let root = PullRequestRecord {
        number: 10,
        title: "Merged root".to_owned(),
        body: None,
        head_branch: "deleted/root".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
        draft: false,
        merged: true,
    };
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/child".to_owned()],
        authored_open_pull_requests_by_head: BTreeMap::from([(
            "topic/child".to_owned(),
            pull_request_choice_record(11, "Child", "topic/child", "topic/root", false),
        )]),
        pull_requests_by_number: BTreeMap::from([(10, root)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "track"], &environment, &services)
        .expect("stack tracking succeeds");

    assert_eq!(result.stdout, "✓ #10     Merged root\n└─ ◯ #11     Child\n");
    assert_eq!(services.pull_request_number_calls.borrow().as_slice(), [10]);
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes[0].branch, "topic/root");
    assert_eq!(metadata.nodes[0].title, "Merged root");
    assert!(metadata.nodes[0].merged);
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
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), pull needed: GitHub has 3 new commits\n"
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
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), pull needed: GitHub has 3 new commits\n"
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
            fork: None,
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), push needed: local has 2 unpublished commits\n"
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
            fork: None,
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), diverged: pull 1 commit, push 2 commits\nremote: upstream (\x1b]8;;https://github.com/upstream-owner/example-repo/tree/main\x1b\\https://github.com/upstream-owner/example-repo.git\x1b]8;;\x1b\\), push needed: local has 3 unpublished commits\n"
        );
}

#[test]
fn remote_status_renders_fork_freshness_after_remotes() {
    // Verifies: Single-repo remote status shows the fork's source relationship explicitly.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut status = FakeServices::default().status;
    status.fork = Some(fork_status(ForkStatusState::SourceAhead, 7, 0));
    let services = FakeServices {
        status,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), pull needed: GitHub has 3 new commits\nfork: {} vs source {}, source has 7 new commits\n",
            osc8_link(
                "https://github.com/example-owner/example-repo/tree/main",
                "example-owner/example-repo/main"
            ),
            osc8_link(
                "https://github.com/source-owner/example-repo/tree/main",
                "source-owner/example-repo/main"
            )
        )
    );
}

#[test]
fn remote_status_all_groups_fork_freshness() {
    // Verifies: Global remote status groups fork/source deltas separately from local remotes.
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
    let forked = workspace.create_jj_workspace("projects/forked");
    TestWorkspace::write_git_config_at(
        &forked,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/forked.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
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
                    state: StatusState::UpToDate,
                    github_ahead_by: 0,
                    github_behind_by: 0,
                },
            }],
            fork: Some(fork_status(ForkStatusState::SourceAhead, 7, 0)),
        },
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "remote-status", "--all"], &environment, &services)
            .expect("global remote-status succeeds");

    assert_eq!(
        result.stdout,
        "Remote status: 1 repository checked, 1 needs attention\n\nFork behind source:\n  ~/projects/forked  source-owner/example-repo/main has 7 new commits\n"
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

    let output = render_global_status(&entries, entries.len(), Path::new("/workspace"), false)
        .expect("global status renders");

    assert_eq!(
        output,
        "Remote status: 2 repositories checked, 2 need attention\n\nSetup needed:\n  beta   beta failed\n  alpha  alpha failed\n"
    );
}

#[test]
fn remote_status_all_groups_repositories_by_action_needed() {
    // Verifies: Global remote-status scans configured repos with a custom concurrency limit and groups pull work by path.
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
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
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
            "  0% Checking remote status…",
            " 50% Checking remote status…",
            "100% Checking remote status…"
        ]
    );
    assert!(progress.finished.get());

    assert_eq!(
        result.stdout,
        "Remote status: 2 repositories checked, 2 need attention\n\nPull needed: GitHub has new commits\n  ~/projects/alpha  3 commits to pull\n  ~/projects/beta   3 commits to pull\n"
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
        "remote: origin (\x1b]8;;https://github.com/example-owner/beta/tree/main\x1b\\ssh://git@github.com/example-owner/beta.git\x1b]8;;\x1b\\), pull needed: GitHub has 3 new commits\n"
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
        "Remote status: 1 repository checked, 1 needs attention\n\nPull needed: GitHub has new commits\n  ~/projects/api-alpha  3 commits to pull\n"
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
        "Remote status: 2 repositories checked, 1 needs attention\n\nPull needed: GitHub has new commits\n  ~/projects/changed  3 commits to pull\n\nSynced: 1 repository\n"
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
        "Remote status: 2 repositories checked, 2 need attention\n\nPull needed: GitHub has new commits\n  ~/projects/ok              3 commits to pull\n\nSetup needed:\n  ~/projects/missing-origin  The fixed `origin` remote is missing. Add an `origin` GitHub remote before running `jx`.\n"
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
fn global_fetch_renderer_sorts_entries_by_directory() {
    // Verifies: Global fetch output follows filesystem order rather than discovery order.
    let entries = vec![
        GlobalFetchEntry {
            root: PathBuf::from("/workspace/src/beta"),
            display_root: "beta".to_owned(),
            result: Ok(()),
        },
        GlobalFetchEntry {
            root: PathBuf::from("/workspace/projects/alpha"),
            display_root: "alpha".to_owned(),
            result: Err("alpha failed".to_owned()),
        },
    ];

    let output = render_global_fetch(&entries, Path::new("/workspace"), false)
        .expect("global fetch renders");

    assert_eq!(
        output,
        "Fetched:\n  beta\n\nErrors:\n  alpha  alpha failed\n"
    );
}

#[test]
fn global_sync_renderer_sorts_each_section_by_directory() {
    // Verifies: Global sync output keeps each status section in stable filesystem order.
    let entries = vec![
        GlobalSyncEntry {
            root: PathBuf::from("/workspace/src/zeta"),
            display_root: "zeta".to_owned(),
            outcome: GlobalSyncOutcome::Synced,
        },
        GlobalSyncEntry {
            root: PathBuf::from("/workspace/src/read-only"),
            display_root: "read-only".to_owned(),
            outcome: GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::ReadOnlyOrigin),
        },
        GlobalSyncEntry {
            root: PathBuf::from("/workspace/projects/alpha"),
            display_root: "alpha".to_owned(),
            outcome: GlobalSyncOutcome::Synced,
        },
        GlobalSyncEntry {
            root: PathBuf::from("/workspace/projects/read-only"),
            display_root: "projects-read-only".to_owned(),
            outcome: GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::ReadOnlyOrigin),
        },
    ];

    let output =
        render_global_sync(&entries, Path::new("/workspace"), false).expect("global sync renders");

    assert_eq!(
        output,
        "Synced:\n  alpha\n  zeta\n\nSkipped: read-only origin\n  projects-read-only\n  read-only\n"
    );
}

#[test]
fn global_sync_renderer_does_not_require_current_workspace_for_color_output() {
    // Verifies: All-repository output can render after running from a non-repository directory.
    let entries = vec![GlobalSyncEntry {
        root: PathBuf::from("/workspace/projects/alpha"),
        display_root: "alpha".to_owned(),
        outcome: GlobalSyncOutcome::Synced,
    }];

    let output = render_global_sync(&entries, Path::new("/not-a-workspace"), true)
        .expect("global sync renders without current workspace");

    assert_eq!(output, "Synced:\n  alpha\n");
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

    let progress = RecordingProgress::default();
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };
    let result = run_with_args_and_progress(
        ["jx", "fetch", "--all"],
        &environment,
        &services,
        &progress,
        prompts,
        OutputMode::plain(),
    )
    .expect("global fetch succeeds");

    assert_eq!(
        progress.messages(),
        [
            "  0% Fetching local-work…",
            " 50% Fetching local-work…",
            " 50% Fetching ready…",
            "100% Fetching ready…",
        ]
    );
    assert!(progress.finished.get());
    assert_eq!(result.stdout, "Fetched:\n  ~/projects/ready\n");
    assert_eq!(services.fetch_origin_roots.borrow().as_slice(), [ready]);
}

#[test]
fn sync_all_shorthand_syncs_writable_repositories_when_origin_does_not_need_pulling() {
    // Verifies: -a selects global sync and can push tracked state even with local jj work present.
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
    let local_work = workspace.create_jj_workspace("projects/local-work");
    let _missing_origin = workspace.create_jj_workspace("projects/missing-origin");
    let pull_needed = workspace.create_jj_workspace("projects/pull-needed");
    let read_only = workspace.create_jj_workspace("projects/read-only");
    let up_to_date = workspace.create_jj_workspace("projects/up-to-date");
    let writable = workspace.create_jj_workspace("projects/writable");
    for (root, name) in [
        (&local_work, "local-work"),
        (&pull_needed, "pull-needed"),
        (&read_only, "read-only"),
        (&up_to_date, "up-to-date"),
        (&writable, "writable"),
    ] {
        TestWorkspace::write_git_config_at(
            root,
            &format!(
                r#"
[remote "origin"]
    url = https://github.com/example-owner/{name}.git
"#,
            ),
        );
    }
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        global_fetch_ready_roots: Some(BTreeSet::new()),
        origin_push_access_roots: Some(BTreeSet::from([
            local_work.clone(),
            pull_needed.clone(),
            up_to_date.clone(),
            writable.clone(),
        ])),
        up_to_date_sync_roots: BTreeSet::from([up_to_date.clone()]),
        clean_status_repos: vec![
            "local-work".to_owned(),
            "up-to-date".to_owned(),
            "writable".to_owned(),
        ],
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 1,
            current_repaired: false,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let progress = RecordingProgress::default();
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };
    let result = run_with_args_and_progress(
        ["jx", "sync", "-a"],
        &environment,
        &services,
        &progress,
        prompts,
        OutputMode::plain(),
    )
    .expect("global sync succeeds");

    assert_eq!(
        progress.messages(),
        [
            "  0% Syncing local-work…",
            " 16% Syncing local-work…",
            " 16% Syncing missing-origin…",
            " 33% Syncing missing-origin…",
            " 33% Syncing pull-needed…",
            " 50% Syncing pull-needed…",
            " 50% Syncing read-only…",
            " 66% Syncing read-only…",
            " 66% Syncing up-to-date…",
            " 83% Syncing up-to-date…",
            " 83% Syncing writable…",
            "100% Syncing writable…",
        ]
    );
    assert!(progress.finished.get());
    assert_eq!(
        services.fetch_origin_roots.borrow().as_slice(),
        [local_work.clone(), up_to_date.clone(), writable.clone()]
    );
    assert_eq!(
        services.push_tracked_roots.borrow().as_slice(),
        [local_work.clone(), up_to_date, writable]
    );
    assert_eq!(
        result.stdout,
        "Synced:\n  ~/projects/local-work\n  ~/projects/writable\n\nSkipped: up to date\n  ~/projects/up-to-date\n\nSkipped: pull needed\n  ~/projects/pull-needed  GitHub has 3 new commits\n\nSkipped: read-only origin\n  ~/projects/read-only\n\nSetup needed:\n  ~/projects/missing-origin  The fixed `origin` remote is missing. Add an `origin` GitHub remote before running `jx`.\n"
    );
}

#[test]
fn sync_all_reports_local_work_when_tracked_push_has_nothing_to_sync() {
    // Verifies: Global sync does not call a repo up to date when unpushed jj work remains.
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
    let local_work = workspace.create_jj_workspace("projects/local-work");
    TestWorkspace::write_git_config_at(
        &local_work,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/local-work.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        origin_push_access_roots: Some(BTreeSet::from([local_work.clone()])),
        up_to_date_sync_roots: BTreeSet::from([local_work.clone()]),
        status: StatusReport {
            remotes: vec![domain::RemoteStatusReport {
                name: "origin".to_owned(),
                url: "https://github.com/example-owner/local-work.git".to_owned(),
                github_url: "https://github.com/example-owner/local-work".to_owned(),
                branch: "main".to_owned(),
                local_trunk_sha: "1111222233334444".to_owned(),
                local_trunk_short_sha: "11112222".to_owned(),
                local_ahead_by: 1,
                comparison: StatusComparison {
                    state: StatusState::UpToDate,
                    github_ahead_by: 0,
                    github_behind_by: 0,
                },
            }],
            fork: None,
        },
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

    let result = run_with_args_and_services(["jx", "sync", "--all"], &environment, &services)
        .expect("global sync succeeds");

    assert_eq!(
        services.fetch_origin_roots.borrow().as_slice(),
        std::slice::from_ref(&local_work)
    );
    assert_eq!(
        services.push_tracked_roots.borrow().as_slice(),
        [local_work]
    );
    assert_eq!(
        result.stdout,
        "Skipped: local work\n  ~/projects/local-work  working copy has 1 local change\n"
    );
}

#[test]
fn sync_all_fetches_pull_needed_repo_when_only_empty_working_copy_is_local() {
    // Verifies: Global sync may pull when jj has only its empty working-copy child locally.
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
    let pull_only = workspace.create_jj_workspace("projects/pull-only");
    TestWorkspace::write_git_config_at(
        &pull_only,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/pull-only.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        global_fetch_ready_roots: Some(BTreeSet::from([pull_only.clone()])),
        origin_push_access_roots: Some(BTreeSet::from([pull_only.clone()])),
        up_to_date_sync_roots: BTreeSet::from([pull_only.clone()]),
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

    let result = run_with_args_and_services(["jx", "sync", "--all"], &environment, &services)
        .expect("global sync succeeds");

    assert_eq!(
        services.fetch_origin_roots.borrow().as_slice(),
        std::slice::from_ref(&pull_only)
    );
    assert_eq!(services.push_tracked_roots.borrow().as_slice(), [pull_only]);
    assert_eq!(result.stdout, "Synced:\n  ~/projects/pull-only\n");
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

    assert_eq!(result.stdout, "Fetched:\n  ~/projects/ready\n");
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
fn sync_fetches_then_pushes_repository_tracked_state_with_commit_lists() {
    // Verifies: Bare sync uses repository policy, rendering rebases before pushed heads/deletions.
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
        services.push_tracked_roots.borrow().as_slice(),
        [workspace.path()]
    );
    assert!(services.push_syncable_revision_requests.borrow().is_empty());
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
fn sync_stack_pushes_current_pull_request_stack() {
    // Verifies: -s syncs every local bookmark in the tracked PR stack, not every repository bookmark.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "topic/root".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Root".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "topic/draft".to_owned(),
                    base_branch: "topic/child".to_owned(),
                    parent_branch: Some("topic/child".to_owned()),
                    pull_request: Some(12),
                    parent_pull_request: Some(11),
                    title: "Draft".to_owned(),
                    url: None,
                    draft: true,
                    merged: false,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        pull_request_bookmarks: vec![
            "topic/root".to_owned(),
            "topic/child".to_owned(),
            "topic/draft".to_owned(),
            "other/topic".to_owned(),
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync", "-s"], &environment, &services)
        .expect("stack sync succeeds");

    assert_eq!(
        services.push_syncable_revision_requests.borrow().as_slice(),
        [
            Some("topic/root".to_owned()),
            Some("topic/child".to_owned()),
            Some("topic/draft".to_owned())
        ]
    );
    assert!(result.stdout.starts_with("Synced: origin/main ("));
}

#[test]
fn sync_stack_refreshes_stored_metadata_by_pull_request_number() {
    // Verifies: sync -s refreshes stack context metadata for durable PR nodes before syncing descriptions.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "topic/root".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Stale root".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let root = PullRequestRecord {
        number: 10,
        title: "Merged root".to_owned(),
        body: None,
        head_branch: "deleted/root".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
        draft: false,
        merged: true,
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        pull_request_bookmarks: vec!["topic/child".to_owned()],
        pull_requests_by_number: BTreeMap::from([(10, root)]),
        tracked_push: TrackedPushOutcome {
            pushed_refs: 1,
            bookmarks: vec![PushedBookmarkSummary {
                branch: "topic/child".to_owned(),
                old_short_commit_id: Some("11112222".to_owned()),
                new_short_commit_id: Some("33334444".to_owned()),
                old_description: Some("old child".to_owned()),
                new_description: Some("Child".to_owned()),
                pull_request_description: Some("Child".to_owned()),
                pull_request_base: Some("topic/root".to_owned()),
                new_workspace_visibility: current_workspace_visibility(),
            }],
            pushed_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync", "-s"], &environment, &services)
        .expect("stack sync succeeds");

    assert!(result.stdout.starts_with("Synced: origin/main ("));
    assert_eq!(
        services.pull_request_number_calls.borrow().as_slice(),
        [10, 11]
    );
    let synced_metadata = services.sync_pull_request_metadata.borrow();
    assert_eq!(synced_metadata.len(), 1);
    assert_eq!(synced_metadata[0].nodes[0].branch, "topic/root");
    assert_eq!(synced_metadata[0].nodes[0].title, "Merged root");
    assert!(synced_metadata[0].nodes[0].merged);
}

#[test]
fn sync_stack_prunes_completed_tree_before_selecting_bookmarks() {
    // Verifies: sync -s garbage-collects already completed stack trees before selecting branches to push.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "topic/root".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Root".to_owned(),
                    url: None,
                    draft: false,
                    merged: true,
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: None,
                    draft: false,
                    merged: true,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        pull_request_bookmarks: vec!["topic/root".to_owned(), "topic/child".to_owned()],
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "sync", "-s"], &environment, &services)
        .expect_err("completed stack is not syncable");

    assert!(matches!(
        error,
        CommandError::Workflow(WorkflowError::MissingPullRequest)
    ));
    assert!(services.push_syncable_revision_requests.borrow().is_empty());
    assert!(read_stack_metadata(&workspace.path())
        .expect("stack metadata reads")
        .nodes
        .is_empty());
    assert!(!workspace.path().join(".jx/stack.toml").exists());
}

#[test]
fn sync_prunes_completed_stack_tree_after_refresh() {
    // Verifies: sync removes completed PR stack trees even when no bookmarks need PR description updates.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "topic/root".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Root".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        tracked_push: TrackedPushOutcome {
            pushed_refs: 0,
            bookmarks: Vec::new(),
            pushed_commits: Vec::new(),
        },
        pull_requests_by_number: BTreeMap::from([
            (
                10,
                PullRequestRecord {
                    number: 10,
                    title: "Root".to_owned(),
                    body: None,
                    head_branch: "deleted/root".to_owned(),
                    base_branch: "main".to_owned(),
                    html_url: Some(
                        "https://github.com/example-owner/example-repo/pull/10".to_owned(),
                    ),
                    draft: false,
                    merged: true,
                },
            ),
            (
                11,
                PullRequestRecord {
                    number: 11,
                    title: "Child".to_owned(),
                    body: None,
                    head_branch: "deleted/child".to_owned(),
                    base_branch: "deleted/root".to_owned(),
                    html_url: Some(
                        "https://github.com/example-owner/example-repo/pull/11".to_owned(),
                    ),
                    draft: false,
                    merged: true,
                },
            ),
        ]),
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.starts_with("Synced: origin/main ("));
    assert_eq!(
        services.pull_request_number_calls.borrow().as_slice(),
        [10, 11]
    );
    assert!(services.sync_pull_request_metadata.borrow().is_empty());
    assert!(read_stack_metadata(&workspace.path())
        .expect("stack metadata reads")
        .nodes
        .is_empty());
    assert!(!workspace.path().join(".jx/stack.toml").exists());
}

#[test]
fn sync_accepts_revision_argument() {
    // Verifies: A positional argument selects one jj target instead of changing repository scope.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "sync", "example-user/current"],
        &environment,
        &services,
    )
    .expect("specific sync succeeds");

    assert_eq!(
        services.push_syncable_revision_requests.borrow().as_slice(),
        [Some("example-user/current".to_owned())]
    );
    assert!(result.stdout.starts_with("Synced: origin/main ("));
}

#[test]
fn sync_repo_shorthand_creates_missing_origin_repository_from_layout() {
    // Verifies: -r forces repository sync while preserving missing-origin bootstrap behavior.
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

    let result = run_with_args_and_services(["jx", "sync", "-r"], &environment, &services)
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
fn sync_offers_layout_repository_initialization_before_bootstrap() {
    // Verifies: Missing-workspace sync initializes an inferred layout repo before bootstrap.
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
fn sync_can_cancel_layout_repository_initialization() {
    // Verifies: Declining local initialization stops before jj, GitHub, or push mutation.
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
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [(
            "HOME".to_owned(),
            workspace.home.to_string_lossy().into_owned(),
        )],
    );
    let services = FakeServices {
        expected_init_repository: Some(workspace.path()),
        ..FakeServices::default()
    };
    let confirmer = FixedRepositoryInitializationConfirmer { confirmed: false };

    let result = run_with_args_and_repository_initialization_confirmer(
        ["jx", "sync"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("sync cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
    assert_eq!(services.init_repository_calls.get(), 0);
    assert_eq!(services.create_repository_calls.get(), 0);
    assert!(services.push_tracked_roots.borrow().is_empty());
}

#[test]
fn sync_refuses_uninitialized_directory_outside_configured_layout() {
    // Verifies: Sync only initializes directories whose GitHub identity is layout-derived.
    let workspace = TestWorkspace::new_uninitialized_under("misc/example-repo");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx", "sync", "--repo"], &environment, &services)
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

    let result = run_with_args_and_services(["jx", "sync", "--repo"], &environment, &services)
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
        ["jx", "sync", "--repo"],
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

    let error = run_with_args_and_services(["jx", "sync", "--repo"], &environment, &services)
        .expect_err("off-layout repo cannot be bootstrapped");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::LayoutPathNotMatched { .. })
    ));
    assert_eq!(services.create_repository_calls.get(), 0);
}

#[test]
fn sync_advances_trunk_when_repo_policy_enables_it() {
    // Verifies: Bare sync runs repository trunk-advance preparation for matching repo policy.
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
                merged: false,
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
                merged: false,
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
            pull_request_description: Some("short branch".to_owned()),
            pull_request_base: Some("main".to_owned()),
            new_workspace_visibility: current_workspace_visibility(),
        },
        PushedBookmarkSummary {
            branch: "long".to_owned(),
            old_short_commit_id: Some("22223333".to_owned()),
            new_short_commit_id: Some("33334444".to_owned()),
            old_description: Some("previous long branch".to_owned()),
            new_description: Some("long branch".to_owned()),
            pull_request_description: Some("long branch".to_owned()),
            pull_request_base: Some("main".to_owned()),
            new_workspace_visibility: current_workspace_visibility(),
        },
        PushedBookmarkSummary {
            branch: "old".to_owned(),
            old_short_commit_id: Some("44445555".to_owned()),
            new_short_commit_id: None,
            old_description: Some("old branch".to_owned()),
            new_description: None,
            pull_request_description: None,
            pull_request_base: None,
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
fn sync_pushes_clean_bookmarks_and_reports_conflicted_skips() {
    // Verifies: Sync keeps pushing safe bookmark updates and reports conflicted ones with a failing exit code.
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
        sync_conflicted_bookmarks: vec![crate::jj::SkippedPushBookmarkSummary {
            branch: "example-user/conflicted".to_owned(),
            conflicted_commits: vec![crate::jj::ConflictedCommitSummary {
                short_commit_id: "ccccdddd".to_owned(),
                description: "example change".to_owned(),
                workspace_visibility: current_workspace_visibility(),
            }],
        }],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect("conflicted sync returns normal output");

    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.contains(&format!(
        "Pushed commits:\n  default@  a1b2c3d4 -> {}  example change\n",
        example_bookmark_link("example-user/current")
    )));
    assert!(result.stdout.contains(&format!(
        "Skipped bookmarks with conflicts:\n  {}  ccccdddd  example change (conflicted)\n",
        example_bookmark_link("example-user/conflicted")
    )));
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
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_records_published_pr_in_stack_state() {
    // Verifies: Publishing a PR upserts durable stack state even before a full stack exists.
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

    let result = run_with_args_and_services(["jx", "pull-request"], &environment, &services)
        .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
    assert!(services.sync_pull_request_pushes.borrow().is_empty());
    assert_eq!(
        read_stack_metadata(&workspace.path()).expect("stack metadata reads"),
        StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "example-user/02-zzzzzzzz".to_owned(),
                base_branch: "example-user/01-ancestor".to_owned(),
                parent_branch: None,
                pull_request: Some(42),
                parent_pull_request: None,
                title: "example change".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
                draft: false,
                merged: false,
            }],
        }
    );
}

#[test]
fn pull_request_refreshes_stack_context_for_published_stack_component() {
    // Verifies: Publishing a stacked PR refreshes generated stack context for every known PR in the component.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut facts = workspace_facts();
    facts.nearest_ancestor_bookmark = Some("topic/root".to_owned());
    let root = pull_request_choice_record(10, "Root", "topic/root", "main", false);
    let child = PullRequestRecord {
        number: 42,
        title: "example change".to_owned(),
        body: None,
        head_branch: "example-user/02-zzzzzzzz".to_owned(),
        base_branch: "topic/root".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
        draft: false,
        merged: false,
    };
    let services = FakeServices {
        workspace: facts,
        pull_requests_by_head: BTreeMap::from([("topic/root".to_owned(), root.clone())]),
        sync_pull_requests: vec![root.clone(), child.clone()],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "pull-request"], &environment, &services)
        .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nStack: refreshed stack context on {}, {}\n",
            example_pull_request_link(42),
            example_pull_request_link(10),
            example_pull_request_link(42)
        )
    );
    let sync_pushes = services.sync_pull_request_pushes.borrow();
    assert_eq!(sync_pushes.len(), 1);
    assert_eq!(
        sync_pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["topic/root", "example-user/02-zzzzzzzz"]
    );
    assert_eq!(
        sync_pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.pull_request_base.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("main"), Some("topic/root")]
    );
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(
        stack_metadata_rows(&metadata.nodes),
        vec!["◯ #10     Root", "└─ ◯ #42     example change"]
    );
    assert_eq!(
        metadata.nodes[1].parent_branch.as_deref(),
        Some("topic/root")
    );
    assert_eq!(metadata.nodes[1].parent_pull_request, Some(10));
}

#[test]
fn pull_request_preserves_stored_parent_when_base_pr_is_missing() {
    // Verifies: A published child keeps known parent metadata even when GitHub cannot refresh that parent by branch.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: true,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut facts = workspace_facts();
    facts.nearest_ancestor_bookmark = Some("topic/root".to_owned());
    let services = FakeServices {
        workspace: facts,
        sync_pull_requests: vec![PullRequestRecord {
            number: 42,
            title: "example change".to_owned(),
            body: None,
            head_branch: "example-user/02-zzzzzzzz".to_owned(),
            base_branch: "topic/root".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
            draft: false,
            merged: false,
        }],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "pull-request"], &environment, &services)
        .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nStack: refreshed stack context on {}\n",
            example_pull_request_link(42),
            example_pull_request_link(42)
        )
    );
    assert_eq!(
        services.pull_request_head_calls.borrow().as_slice(),
        ["topic/root"]
    );
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(
        stack_metadata_rows(&metadata.nodes),
        vec!["✓ #10     Root", "└─ ◯ #42     example change"]
    );
    assert_eq!(
        metadata.nodes[1].parent_branch.as_deref(),
        Some("topic/root")
    );
    assert_eq!(metadata.nodes[1].parent_pull_request, Some(10));
}

#[test]
fn pull_request_prepare_event_updates_commit_title_before_planning() {
    // Verifies: Prepare handlers rewrite the selected commit before PR metadata is planned.
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
[[repo.event_handlers]]
id = "prepend-task"
on = "pull_request.prepare"
when = "has:task"
run = "prepend_task_id"
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
    let mut fake_workspace = workspace_facts();
    fake_workspace.target_change.description = "Example title\n\nDetailed body".to_owned();
    let services = FakeServices {
        workspace: fake_workspace,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "pull-request"], &environment, &services)
        .expect("pull request publishes");

    assert_eq!(
        services.description_rewrites.borrow().as_slice(),
        &[(
            "a1b2c3d4e5f6".to_owned(),
            "ABC-123: Example title\n\nDetailed body".to_owned()
        )]
    );
    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_opens_created_pr_when_event_handler_requests_it() {
    // Verifies: PR publishing runs command-side browser effects after the PR is created.
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
        pull_request_event_effects: vec![
            domain::PullRequestEventEffect {
                event: crate::repository::RepoEvent::PullRequestCreated,
                handler_id: Some("label-created".to_owned()),
                kind: PullRequestEventEffectKind::AddLabels {
                    labels: vec!["queued".to_owned()],
                },
            },
            domain::PullRequestEventEffect {
                event: crate::repository::RepoEvent::PullRequestCreated,
                handler_id: Some("open-created".to_owned()),
                kind: PullRequestEventEffectKind::OpenPullRequest {
                    url: "https://github.com/example-owner/example-repo/pull/42".to_owned(),
                },
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "pull-request"], &environment, &services)
        .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nEvent[label-created]: added label queued\nEvent[open-created]: opened {}\n",
            example_pull_request_link(42),
            example_pull_request_link(42)
        )
    );
    assert_eq!(
        services.opened_urls.borrow().as_slice(),
        &["https://github.com/example-owner/example-repo/pull/42".to_owned()]
    );
}

#[test]
fn pull_request_hides_noop_event_effects_by_default() {
    // Verifies: Default output stays quiet when handlers matched but did not change state.
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
        pull_request_event_effects: vec![
            domain::PullRequestEventEffect {
                event: crate::repository::RepoEvent::PullRequestCreated,
                handler_id: Some("labels-present".to_owned()),
                kind: PullRequestEventEffectKind::LabelsAlreadyPresent {
                    labels: vec!["queued".to_owned()],
                },
            },
            domain::PullRequestEventEffect {
                event: crate::repository::RepoEvent::PullRequestPrepare,
                handler_id: Some("title-present".to_owned()),
                kind: PullRequestEventEffectKind::TitleAlready {
                    title: "ABC-123: Example title".to_owned(),
                },
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "pull-request"], &environment, &services)
        .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_no_event_handlers_suppresses_event_effects() {
    // Verifies: Operators can disable configured PR automation for a single publish.
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
        pull_request_event_effects: vec![domain::PullRequestEventEffect {
            event: crate::repository::RepoEvent::PullRequestCreated,
            handler_id: Some("open-created".to_owned()),
            kind: PullRequestEventEffectKind::OpenPullRequest {
                url: "https://github.com/example-owner/example-repo/pull/42".to_owned(),
            },
        }],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "pull-request", "--no-event-handlers"],
        &environment,
        &services,
    )
    .expect("pull request publishes without event handlers");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
    assert!(services.opened_urls.borrow().is_empty());
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
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_no_task_id_ignores_workspace_metadata() {
    // Verifies: Operators can opt out of workspace task metadata for ticketless PRs.
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
        expected_task_id: Some(None),
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "pr", "--no-task-id"], &environment, &services)
        .expect("pull request publishes without task id");
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
        format!("Created {}\n", example_pull_request_link(42))
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
        format!("Updated {}\n", example_pull_request_link(42))
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

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
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
        format!("Created {}\n", example_pull_request_link(42))
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
        format!("Created {}\n", example_pull_request_link(42))
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
        format!("Created {}\n", example_pull_request_link(42))
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
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_previews_before_reviewer_selection() {
    // Verifies: Operators see the PR description before deciding which reviewers to request.
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
        reviewer_candidates: vec![ReviewerCandidate::new(
            ReviewerTarget::user("second-reviewer"),
            vec!["src/** matched 1 file".to_owned()],
        )],
        expected_reviewers: Some(selected.clone()),
        ..FakeServices::default()
    };
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let previewer = RecordingPullRequestPreviewer {
        events: events.clone(),
    };
    let reviewer_selector = RecordingReviewerSelector {
        events: events.clone(),
        selected,
    };
    let prompts = PromptHandlers {
        pull_request_previewer: &previewer,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &reviewer_selector,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };

    let result = run_with_args_and_progress(
        ["jx", "pull-request"],
        &environment,
        &services,
        &NoProgress,
        prompts,
        OutputMode::plain(),
    )
    .expect("pull request publishes");

    assert_eq!(events.borrow().as_slice(), &["preview", "reviewers"]);
    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_selection_formats_draft_state_as_color_only() {
    // Verifies: Draft PR choices keep text aligned and signal state with subdued color only.
    let ready = PullRequestRecord {
        number: 42,
        title: "Ready change".to_owned(),
        body: None,
        head_branch: "topic/ready".to_owned(),
        base_branch: "main".to_owned(),
        html_url: None,
        draft: false,
        merged: false,
    };
    let draft = PullRequestRecord {
        number: 43,
        title: "Work in progress".to_owned(),
        body: None,
        head_branch: "topic/wip".to_owned(),
        base_branch: "main".to_owned(),
        html_url: None,
        draft: true,
        merged: false,
    };

    assert_eq!(pull_request_choice_label(&ready), "◯ #42     Ready change");
    assert_eq!(
        pull_request_choice_label(&draft),
        "\x1b[2m\x1b[38;2;150;142;132m◌ #43     Work in progress\x1b[0m"
    );
    assert!(!pull_request_choice_label(&draft).contains("draft "));
    assert!(!pull_request_choice_label(&ready).contains("topic/ready"));
}

#[test]
fn pull_request_selection_renders_stack_tree_in_merge_order() {
    // Verifies: PR choices show stack hierarchy and keep row selection mapped after sorting.
    let pull_requests = vec![
        pull_request_choice_record(12, "Child 2", "topic/child-2", "topic/root", false),
        pull_request_choice_record(1, "Draft root", "topic/draft-root", "main", true),
        pull_request_choice_record(14, "Child 11", "topic/child-11", "topic/child-1", false),
        pull_request_choice_record(10, "Root", "topic/root", "main", false),
        pull_request_choice_record(2, "Other root", "topic/other", "main", false),
        pull_request_choice_record(11, "Child 1", "topic/child-1", "topic/root", false),
    ];

    let local_branches = pull_requests
        .iter()
        .map(|pull_request| pull_request.head_branch.clone())
        .collect::<Vec<_>>();
    let snapshot = PullRequestStackSnapshot::from_metadata(
        &StackMetadata::default(),
        &local_branches,
        &pull_requests,
        PullRequestStackSelection::default(),
    );
    let rows = pull_request_choice_rows(&snapshot);

    assert_eq!(
        rows.iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "◯ #2      Other root",
            "◯ #10     Root",
            "├─ ◯ #11     Child 1",
            "│  └─ ◯ #14     Child 11",
            "└─ ◯ #12     Child 2",
            "\x1b[2m\x1b[38;2;150;142;132m◌ #1      Draft root\x1b[0m",
        ]
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.pull_request.number)
            .collect::<Vec<_>>(),
        vec![2, 10, 11, 14, 12, 1]
    );
}

struct RecordingPullRequestSelector {
    selected: usize,
    labels: std::cell::RefCell<Vec<Vec<String>>>,
}

impl RecordingPullRequestSelector {
    fn new(selected: usize) -> Self {
        Self {
            selected,
            labels: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl PullRequestSelector for RecordingPullRequestSelector {
    fn select_pull_request(
        &self,
        choices: &[PullRequestChoice],
    ) -> Result<PullRequestRecord, PullRequestSelectionError> {
        self.labels
            .borrow_mut()
            .push(choices.iter().map(|choice| choice.label.clone()).collect());
        choices
            .get(self.selected)
            .map(|choice| choice.pull_request.clone())
            .ok_or(PullRequestSelectionError::NoPullRequests)
    }
}

fn pull_request_choice_record(
    number: u64,
    title: &str,
    head_branch: &str,
    base_branch: &str,
    draft: bool,
) -> PullRequestRecord {
    PullRequestRecord {
        number,
        title: title.to_owned(),
        body: None,
        head_branch: head_branch.to_owned(),
        base_branch: base_branch.to_owned(),
        html_url: None,
        draft,
        merged: false,
    }
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
fn workspace_status_renderer_renders_markdown_description_without_preview_indent() {
    // Verifies: jx status uses the shared PR markdown renderer without adding preview indentation.
    let status = WorkspaceStatus {
        commit_lines: vec!["Working copy  (@) : kvxvwztp b9e8f888".to_owned()],
        description: "This is **important** markdown with enough words to wrap.".to_owned(),
        change_lines: Vec::new(),
        extra_lines: Vec::new(),
    };

    let rendered = render_workspace_status_with_width(&status, 28);
    let description_block = rendered
        .split("\n\n")
        .nth(1)
        .expect("status renders description after commit lines");

    assert!(description_block.lines().count() > 1, "{rendered:?}");
    assert!(description_block.contains("important"), "{rendered:?}");
    assert!(!description_block.contains("**important**"), "{rendered:?}");
    assert!(
        description_block
            .lines()
            .all(|line| !line.starts_with("  ")),
        "{rendered:?}"
    );
}

#[test]
fn pull_request_preview_focuses_on_publish_state_and_changed_files() {
    // Verifies: PR preview omits commit headers while keeping description, changed files, and metadata.
    let mut plan = preview_plan();
    plan.labels = vec!["bug".to_owned(), "help wanted".to_owned()];
    plan.base_pull_request = Some(existing_pull_request(false));
    let status = workspace_status();
    let prepare_effects = [PullRequestEventEffect {
        event: crate::repository::RepoEvent::PullRequestPrepare,
        handler_id: Some("prepend-task".to_owned()),
        kind: PullRequestEventEffectKind::UpdatedTitle {
            title: "example change".to_owned(),
        },
    }];

    let preview = render_pull_request_preview(&plan, &status, &prepare_effects);

    assert_eq!(
        preview,
        format!(
            "Creating: {} → {}\nEvent[prepend-task]: Added task ID to the title\n\n  example change\n\n  M src/main.rs\n\nLabels: bug, help wanted\n",
            example_bookmark_link("example-user/02-zzzzzzzz"),
            example_pull_request_link(7),
        )
    );
    assert_eq!(pull_request_confirmation_prompt(&plan), "Create?");
    plan.draft = true;
    assert_eq!(pull_request_confirmation_prompt(&plan), "Create draft?");
    plan.existing_pull_request = Some(existing_pull_request(false));
    assert_eq!(pull_request_confirmation_prompt(&plan), "Update?");
    plan.existing_pull_request = Some(existing_pull_request(true));
    assert_eq!(pull_request_confirmation_prompt(&plan), "Update draft?");
}

#[test]
fn pull_request_preview_wraps_description_inside_content_indent() {
    // Verifies: Indented PR content still reserves indentation width before markdown wrapping.
    let mut plan = preview_plan();
    plan.title = "Example preview title".to_owned();
    plan.body = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda".to_owned();
    let preview = render_pull_request_preview_for_width(&plan, &workspace_status(), &[], 28);

    let indented_lines = preview
        .lines()
        .filter(|line| line.starts_with("  ") && !line.trim().is_empty())
        .collect::<Vec<_>>();

    assert!(indented_lines.len() > 3, "{preview:?}");
    for line in indented_lines {
        assert!(
            line.len() <= 28,
            "line exceeded preview width: {line:?}\n{preview}"
        );
    }
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

struct RecordingPullRequestPreviewer {
    events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
}

impl PullRequestPreviewer for RecordingPullRequestPreviewer {
    fn show_preview(
        &self,
        _plan: &PullRequestPlan,
        _status: &WorkspaceStatus,
        _prepare_effects: &[PullRequestEventEffect],
    ) {
        self.events.borrow_mut().push("preview");
    }
}

struct RecordingReviewerSelector {
    events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
    selected: ReviewerSelection,
}

impl ReviewerSelector for RecordingReviewerSelector {
    fn select_reviewers(
        &self,
        _candidates: &[ReviewerCandidate],
        _preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError> {
        self.events.borrow_mut().push("reviewers");
        Ok(self.selected.clone())
    }
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
        merged: false,
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
            branch: "example-user/02-zzzzzzzz".to_owned(),
            action: BookmarkAction::Create,
        },
        target_commit_id: "a1b2c3d4e5f6".to_owned(),
        title: "example change".to_owned(),
        body: String::new(),
        changed_files: vec!["src/main.rs".to_owned()],
        base: "main".to_owned(),
        base_pull_request: None,
        head: PullRequestHead::same_repository("example-owner", "example-user/02-zzzzzzzz"),
        labels: Vec::new(),
        draft: false,
        existing_pull_request: None,
        reviewer_candidates: Vec::new(),
        reviewers: ReviewerSelection::default(),
    }
}

struct FakeServices {
    workspace_log: String,
    previous_commit_log: String,
    next_commit_log: String,
    workspace_status: WorkspaceStatus,
    workspace: WorkspaceFacts,
    description_rewrites: std::cell::RefCell<Vec<(String, String)>>,
    status_workspace: StatusWorkspaceFacts,
    check: CheckReport,
    status: StatusReport,
    status_uses_context_remotes: bool,
    clean_status_repos: Vec<String>,
    github_login: String,
    pull_request_bookmarks: Vec<String>,
    pull_request_bookmark_calls: std::cell::Cell<usize>,
    open_pull_request_candidates: Vec<String>,
    open_pull_request_selectors: std::cell::RefCell<Vec<Option<String>>>,
    authored_open_pull_requests_by_head: BTreeMap<String, PullRequestRecord>,
    authored_open_pull_request_head_calls: std::cell::RefCell<Vec<(String, String)>>,
    pull_requests_by_head: BTreeMap<String, PullRequestRecord>,
    pull_request_head_calls: std::cell::RefCell<Vec<String>>,
    pull_requests_by_number: BTreeMap<u64, PullRequestRecord>,
    pull_request_number_calls: std::cell::RefCell<Vec<u64>>,
    opened_urls: std::cell::RefCell<Vec<String>>,
    global_fetch_ready_roots: Option<BTreeSet<PathBuf>>,
    origin_push_access_roots: Option<BTreeSet<PathBuf>>,
    up_to_date_sync_roots: BTreeSet<PathBuf>,
    fetch_origin_roots: std::cell::RefCell<Vec<PathBuf>>,
    push_tracked_roots: std::cell::RefCell<Vec<PathBuf>>,
    push_syncable_revision_requests: std::cell::RefCell<Vec<Option<String>>>,
    fetch: FetchOutcome,
    rebase_on_trunk: RebaseOnTrunkOutcome,
    expected_rebase_sources: Option<Vec<String>>,
    bookmark_update: BookmarkUpdate,
    push: PushOutcome,
    advance_trunk: AdvanceTrunkOutcome,
    advance_trunk_calls: std::cell::Cell<usize>,
    tracked_push: TrackedPushOutcome,
    sync_conflicted_bookmarks: Vec<crate::jj::SkippedPushBookmarkSummary>,
    sync_pull_requests: Vec<PullRequestRecord>,
    sync_pull_request_pushes: std::cell::RefCell<Vec<TrackedPushOutcome>>,
    sync_pull_request_metadata: std::cell::RefCell<Vec<StackMetadata>>,
    pull_request_action: PullRequestAction,
    pull_request_url: Option<String>,
    pull_request_event_effects: Vec<domain::PullRequestEventEffect>,
    existing_pull_request: Option<PullRequestRecord>,
    reviewer_candidates: Vec<ReviewerCandidate>,
    expected_reviewers: Option<ReviewerSelection>,
    expected_task_id: Option<Option<String>>,
    expected_labels: Vec<String>,
    expected_draft: Option<bool>,
    expected_clone: Option<(String, PathBuf)>,
    expected_init_repository: Option<PathBuf>,
    init_repository_calls: std::cell::Cell<usize>,
    expected_workspace_add_current_dir: Option<PathBuf>,
    expected_workspace_add: Option<WorkspaceAddOptions>,
    workspace_add_error: Option<String>,
    workspace_add_metadata_blocker: Option<PathBuf>,
    workspace_add_existing_shared_path: Option<PathBuf>,
    workspaces: Vec<WorkspaceEntry>,
    workspace_remove_current_dirs: std::cell::RefCell<Vec<PathBuf>>,
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
            previous_commit_log: "previous commit graph\n".to_owned(),
            next_commit_log: "next commit graph\n".to_owned(),
            workspace_status: workspace_status(),
            workspace: workspace_facts(),
            description_rewrites: std::cell::RefCell::new(Vec::new()),
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
                    branch: "example-user/02-zzzzzzzz".to_owned(),
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
                fork: None,
            },
            status_uses_context_remotes: false,
            clean_status_repos: Vec::new(),
            github_login: "example-user".to_owned(),
            pull_request_bookmarks: Vec::new(),
            pull_request_bookmark_calls: std::cell::Cell::new(0),
            open_pull_request_candidates: Vec::new(),
            open_pull_request_selectors: std::cell::RefCell::new(Vec::new()),
            authored_open_pull_requests_by_head: BTreeMap::new(),
            authored_open_pull_request_head_calls: std::cell::RefCell::new(Vec::new()),
            pull_requests_by_head: BTreeMap::new(),
            pull_request_head_calls: std::cell::RefCell::new(Vec::new()),
            pull_requests_by_number: BTreeMap::new(),
            pull_request_number_calls: std::cell::RefCell::new(Vec::new()),
            opened_urls: std::cell::RefCell::new(Vec::new()),
            global_fetch_ready_roots: None,
            origin_push_access_roots: None,
            up_to_date_sync_roots: BTreeSet::new(),
            fetch_origin_roots: std::cell::RefCell::new(Vec::new()),
            push_tracked_roots: std::cell::RefCell::new(Vec::new()),
            push_syncable_revision_requests: std::cell::RefCell::new(Vec::new()),
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
                branch: "example-user/abc-123-02-zzzzzzzz".to_owned(),
                created: true,
            },
            push: PushOutcome {
                branch: "example-user/abc-123-02-zzzzzzzz".to_owned(),
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
                        pull_request_description: Some("example change".to_owned()),
                        pull_request_base: Some("main".to_owned()),
                        new_workspace_visibility: current_workspace_visibility(),
                    },
                    PushedBookmarkSummary {
                        branch: "example-user/old".to_owned(),
                        old_short_commit_id: Some("99990000".to_owned()),
                        new_short_commit_id: None,
                        old_description: Some("obsolete example change".to_owned()),
                        new_description: None,
                        pull_request_description: None,
                        pull_request_base: None,
                        new_workspace_visibility: WorkspaceVisibility::default(),
                    },
                ],
                pushed_commits: vec![PushedCommitSummary {
                    short_commit_id: "a1b2c3d4".to_owned(),
                    description: "example change".to_owned(),
                }],
            },
            sync_conflicted_bookmarks: Vec::new(),
            sync_pull_requests: Vec::new(),
            sync_pull_request_pushes: std::cell::RefCell::new(Vec::new()),
            sync_pull_request_metadata: std::cell::RefCell::new(Vec::new()),
            pull_request_action: PullRequestAction::Created,
            pull_request_url: Some(
                "https://github.com/example-owner/example-repo/pull/42".to_owned(),
            ),
            pull_request_event_effects: Vec::new(),
            existing_pull_request: None,
            reviewer_candidates: Vec::new(),
            expected_reviewers: None,
            expected_task_id: None,
            expected_labels: Vec::new(),
            expected_draft: None,
            expected_clone: None,
            expected_init_repository: None,
            init_repository_calls: std::cell::Cell::new(0),
            expected_workspace_add_current_dir: None,
            expected_workspace_add: None,
            workspace_add_error: None,
            workspace_add_metadata_blocker: None,
            workspace_add_existing_shared_path: None,
            workspaces: vec![WorkspaceEntry {
                name: "default".to_owned(),
                root: PathBuf::from("/workspace"),
                is_current: true,
            }],
            workspace_remove_current_dirs: std::cell::RefCell::new(Vec::new()),
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
        if revision == Some("feedfacecafebeef") {
            workspace.target_change.commit_id = "feedfacecafebeef".to_owned();
            workspace.target_change.short_commit_id = "feedface".to_owned();
            if let Some((_, description)) = self.description_rewrites.borrow().last() {
                workspace.target_change.description = description.clone();
            }
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
        let paths = if options.paths.is_empty() {
            String::new()
        } else {
            format!(" paths={}", options.paths.join(","))
        };
        Ok(format!(
            "diff:{revision}{paths} no_tests={} tool={tool}\n",
            options.no_tests
        ))
    }

    fn previous_commit_log(&self, _current_dir: &Path) -> Result<String, JjError> {
        Ok(self.previous_commit_log.clone())
    }

    fn next_commit_log(&self, _current_dir: &Path) -> Result<String, JjError> {
        Ok(self.next_commit_log.clone())
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
        current_dir: &Path,
        options: &WorkspaceAddOptions,
    ) -> Result<(), JjError> {
        if let Some(expected) = &self.expected_workspace_add_current_dir {
            assert_eq!(current_dir, expected);
        }
        if let Some(expected) = &self.expected_workspace_add {
            assert_eq!(options, expected);
        }
        if let Some(status) = &self.workspace_add_error {
            return Err(JjError::WorkspaceAddFailed {
                status: status.clone(),
            });
        }
        if let Some(destination) = &self.workspace_add_metadata_blocker {
            fs::create_dir_all(destination).expect("create simulated workspace destination");
            fs::write(destination.join(".jx"), "not a directory").expect("create metadata blocker");
        }
        if let Some(path) = &self.workspace_add_existing_shared_path {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create existing shared-path parent");
            }
            fs::write(path, "existing content").expect("create existing shared path");
        }
        Ok(())
    }

    fn workspace_entries(&self, _current_dir: &Path) -> Result<Vec<WorkspaceEntry>, JjError> {
        Ok(self.workspaces.clone())
    }

    fn current_workspace_entry(&self, _current_dir: &Path) -> Result<WorkspaceEntry, JjError> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.is_current)
            .cloned()
            .ok_or_else(|| JjError::WorkspaceLoad {
                message: "test current workspace missing".to_owned(),
            })
    }

    fn remove_workspace(
        &self,
        current_dir: &Path,
        options: &WorkspaceRemoveOptions,
    ) -> Result<(), JjError> {
        self.workspace_remove_current_dirs
            .borrow_mut()
            .push(current_dir.to_path_buf());
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

    fn rewrite_commit_description(
        &self,
        _context: &RepositoryContext,
        target_commit_id: &str,
        description: &str,
    ) -> Result<CommitDescriptionRewrite, JjError> {
        self.description_rewrites
            .borrow_mut()
            .push((target_commit_id.to_owned(), description.to_owned()));
        Ok(CommitDescriptionRewrite {
            commit_id: "feedfacecafebeef".to_owned(),
            changed: true,
        })
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
            if let Some(fork) = &mut status.fork {
                fork.comparison = ForkStatusComparison {
                    state: ForkStatusState::Synced,
                    source_ahead_by: 0,
                    fork_ahead_by: 0,
                };
            }
        }
        Ok(status)
    }

    fn origin_can_push(&self, context: &RepositoryContext) -> Result<bool, WorkflowError> {
        Ok(self
            .origin_push_access_roots
            .as_ref()
            .is_none_or(|roots| roots.contains(&context.workspace_root)))
    }

    fn authenticated_login(&self, _token_source: &TokenSource) -> Result<String, WorkflowError> {
        Ok(self.github_login.clone())
    }

    fn pull_request_bookmarks(&self, _context: &RepositoryContext) -> Result<Vec<String>, JjError> {
        self.pull_request_bookmark_calls
            .set(self.pull_request_bookmark_calls.get() + 1);
        Ok(self.pull_request_bookmarks.clone())
    }

    fn pull_request_candidate_bookmarks(
        &self,
        _context: &RepositoryContext,
        selector: Option<&str>,
    ) -> Result<Vec<String>, JjError> {
        self.open_pull_request_selectors
            .borrow_mut()
            .push(selector.map(str::to_owned));
        Ok(self.open_pull_request_candidates.clone())
    }

    fn find_authored_open_pull_request_for_head(
        &self,
        _context: &RepositoryContext,
        branch: &str,
        author: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.authored_open_pull_request_head_calls
            .borrow_mut()
            .push((branch.to_owned(), author.to_owned()));
        Ok(self
            .authored_open_pull_requests_by_head
            .get(branch)
            .cloned())
    }

    fn find_pull_request_for_head(
        &self,
        _context: &RepositoryContext,
        branch: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.pull_request_head_calls
            .borrow_mut()
            .push(branch.to_owned());
        Ok(self.pull_requests_by_head.get(branch).cloned())
    }

    fn find_pull_request_by_number(
        &self,
        _context: &RepositoryContext,
        number: u64,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.pull_request_number_calls.borrow_mut().push(number);
        Ok(self.pull_requests_by_number.get(&number).cloned())
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

    fn push_tracked(&self, context: &RepositoryContext) -> Result<TrackedPushOutcome, JjError> {
        self.push_tracked_roots
            .borrow_mut()
            .push(context.workspace_root.clone());
        if self.up_to_date_sync_roots.contains(&context.workspace_root) {
            return Ok(TrackedPushOutcome {
                pushed_refs: 0,
                bookmarks: Vec::new(),
                pushed_commits: Vec::new(),
            });
        }
        Ok(self.tracked_push.clone())
    }

    fn push_syncable_revision(
        &self,
        context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<SyncPushOutcome, JjError> {
        self.push_tracked_roots
            .borrow_mut()
            .push(context.workspace_root.clone());
        self.push_syncable_revision_requests
            .borrow_mut()
            .push(revision.map(str::to_owned));
        Ok(SyncPushOutcome {
            pushed: self.tracked_push.clone(),
            skipped_conflicted_bookmarks: self.sync_conflicted_bookmarks.clone(),
        })
    }

    fn push_syncable_tracked(
        &self,
        context: &RepositoryContext,
    ) -> Result<SyncPushOutcome, JjError> {
        self.push_tracked_roots
            .borrow_mut()
            .push(context.workspace_root.clone());
        let pushed = if self.up_to_date_sync_roots.contains(&context.workspace_root) {
            TrackedPushOutcome {
                pushed_refs: 0,
                bookmarks: Vec::new(),
                pushed_commits: Vec::new(),
            }
        } else {
            self.tracked_push.clone()
        };
        Ok(SyncPushOutcome {
            pushed,
            skipped_conflicted_bookmarks: self.sync_conflicted_bookmarks.clone(),
        })
    }

    fn sync_pull_requests(
        &self,
        _context: &RepositoryContext,
        push: &TrackedPushOutcome,
        stack_metadata: &StackMetadata,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError> {
        self.sync_pull_request_pushes
            .borrow_mut()
            .push(push.clone());
        self.sync_pull_request_metadata
            .borrow_mut()
            .push(stack_metadata.clone());
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
        let short = workspace
            .target_change
            .change_id
            .chars()
            .take(8)
            .collect::<String>();
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

        let (title, body) = fake_pull_request_description(&workspace.target_change.description);

        Ok(PullRequestPlan {
            repository: self.check.repository.clone(),
            task_id,
            bookmark: BookmarkPlan {
                branch: branch.clone(),
                action: BookmarkAction::Create,
            },
            target_commit_id: workspace.target_change.commit_id.clone(),
            title,
            body,
            changed_files: workspace.changed_files,
            base: workspace
                .nearest_ancestor_bookmark
                .clone()
                .unwrap_or(workspace.trunk.branch),
            base_pull_request: workspace
                .nearest_ancestor_bookmark
                .as_deref()
                .and_then(|branch| self.pull_requests_by_head.get(branch).cloned()),
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
        options: PullRequestPublishOptions,
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
                body: (!plan.body.is_empty()).then_some(plan.body),
                head_branch: plan.head.branch.clone(),
                base_branch: plan.base.clone(),
                html_url: self.pull_request_url.clone(),
                draft: plan.draft,
                merged: false,
            },
            base: plan.base,
            base_pull_request: plan.base_pull_request,
            head: plan.head,
            labels: None,
            reviewers: None,
            event_effects: if options.event_handlers {
                self.pull_request_event_effects.clone()
            } else {
                Vec::new()
            },
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

fn fake_pull_request_description(description: &str) -> (String, String) {
    let trimmed = description.trim();
    let title = trimmed
        .lines()
        .find_map(|line| {
            let line = line.trim();
            (!line.is_empty()).then_some(line.to_owned())
        })
        .unwrap_or_else(|| "example change".to_owned());
    let body = trimmed
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .skip(1)
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    (title, body)
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
