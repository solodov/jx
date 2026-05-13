use jj_lib::{config::StackedConfig, settings::UserSettings, workspace::Workspace};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn root_help_uses_operator_facing_command_descriptions() {
    // Verifies: Root help uses operator-facing command descriptions.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .arg("--help")
        .output()
        .expect("run jx binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Show a jj diff"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("st"));
    assert!(stdout.contains("Show current jj commit status with description"));
    assert!(stdout.contains("work"));
    assert!(stdout.contains("Manage layout workspaces"));
    assert!(stdout.contains("Check repository and PR readiness"));
    assert!(stdout.contains("remote-status"));
    assert!(stdout.contains("rs"));
    assert!(stdout.contains("Compare local remote trunks with GitHub"));
    assert!(stdout.contains("Fetch origin and rebase/repair the jj stack"));
    assert!(stdout.contains("rebase-on-trunk"));
    assert!(stdout.contains("rt"));
    assert!(stdout.contains("Rebase jj source revisions onto origin trunk"));
    assert!(stdout.contains("Push a selected jj change or tracked bookmark state"));
    assert!(stdout.contains("Fetch origin"));
    assert!(stdout.contains("initialize/create the configured repository"));
    assert!(stdout.contains("then push bookmark"));
    assert!(stdout.contains("state"));
    assert!(stdout.contains("Publish or update a GitHub pull request for a jj change"));
    assert!(output.stderr.is_empty());
}

#[test]
fn diff_help_documents_test_filter_and_tool_without_loading_repo() {
    // Verifies: Diff help documents its focused options without loading repository state.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .args(["diff", "--help"])
        .output()
        .expect("run jx binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Show a jj diff"));
    assert!(stdout.contains("--revision"));
    assert!(stdout.contains("-r"));
    assert!(stdout.contains("COMMIT_OR_BOOKMARK"));
    assert!(stdout.contains("--no-tests"));
    assert!(stdout.contains("Exclude test files from the selected diff"));
    assert!(stdout.contains("--tool"));
    assert!(stdout.contains("Use a configured jx diff tool"));
    assert!(stdout.contains("TOOL_ARG"));
    assert!(stdout.contains("Append arguments to the selected diff tool"));
    assert!(output.stderr.is_empty());
}

#[test]
fn work_help_documents_workspace_group_without_loading_repo() {
    // Verifies: Workspace help describes the command group without loading repository state.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .args(["work", "--help"])
        .output()
        .expect("run jx binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Manage layout workspaces"));
    assert!(stdout.contains("add"));
    assert!(stdout.contains("list"));
    assert!(stdout.contains("remove"));
    assert!(output.stderr.is_empty());
}

#[test]
fn status_help_explains_commit_status_without_loading_repo() {
    // Verifies: Status help explains the shared commit status block without loading repository state.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .args(["status", "--help"])
        .output()
        .expect("run jx binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Show current jj commit status with description"));
    assert!(output.stderr.is_empty());
}

#[test]
fn fetch_help_explains_stack_update_without_loading_repo() {
    // Verifies: Fetch help explains the stack update without loading repository state.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .args(["fetch", "--help"])
        .output()
        .expect("run jx binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Fetch origin and rebase/repair the jj stack"));
    assert!(output.stderr.is_empty());
}

#[test]
fn rebase_on_trunk_help_documents_source_without_loading_repo() {
    // Verifies: Rebase-on-trunk help documents source selection without loading repository state.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .args(["rebase-on-trunk", "--help"])
        .output()
        .expect("run jx binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Rebase jj source revisions onto origin trunk"));
    assert!(stdout.contains("--source"));
    assert!(stdout.contains("-s"));
    assert!(stdout.contains("COMMIT"));
    assert!(
        stdout.contains("Rebase a specific jj revision and its descendants; repeat for multiple")
    );
    assert!(stdout.contains("sources"));
    assert!(output.stderr.is_empty());
}

#[test]
fn push_help_documents_revision_and_tracked_modes_without_loading_repo() {
    // Verifies: Push help documents selected-revision and tracked-bookmark modes.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .args(["push", "--help"])
        .output()
        .expect("run jx binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Push a selected jj change or tracked bookmark state"));
    assert!(stdout.contains("--revision"));
    assert!(stdout.contains("-r"));
    assert!(stdout.contains("COMMIT_OR_BOOKMARK"));
    assert!(stdout.contains("Push a specific jj revision or local bookmark"));
    assert!(stdout.contains("--tracked"));
    assert!(stdout.contains("including deleted"));
    assert!(output.stderr.is_empty());
}

#[test]
fn sync_help_explains_fetch_then_push_without_loading_repo() {
    // Verifies: Sync help documents fetch, repository bootstrap, and push behavior.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .args(["sync", "--help"])
        .output()
        .expect("run jx binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains(
        "Fetch origin, or initialize/create the configured repository, then push bookmark state"
    ));
    assert!(stdout.contains("--all"));
    assert!(stdout.contains("sync every eligible primary repository"));
    assert!(stdout.contains("token has origin push access"));
    assert!(stdout.contains("origin is not diverged"));
    assert!(stdout.contains("Pull-only repositories are synced only when"));
    assert!(stdout.contains("empty jj child of origin trunk"));
    assert!(!stdout.contains("current workspace is safe for global mutation"));
    assert!(output.stderr.is_empty());
}

#[test]
fn pull_request_help_documents_task_id_and_commit_without_loading_repo() {
    // Verifies: Pull request help documents task and revision flags without loading repository state.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .args(["pull-request", "--help"])
        .output()
        .expect("run jx binary");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Publish or update a GitHub pull request for a jj change"));
    assert!(stdout.contains("--task-id"));
    assert!(stdout.contains("Associate a task identifier with generated workspace or PR bookmark"));
    assert!(stdout.contains("TASK_ID"));
    assert!(stdout.contains("--commit"));
    assert!(stdout.contains("Publish a specific jj revision instead of the working copy"));
    assert!(stdout.contains("COMMIT"));
}

#[test]
fn remote_status_fails_when_origin_is_missing() {
    // Verifies: Remote status fails when origin is missing.
    let workspace = TestWorkspace::new();

    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .arg("remote-status")
        .current_dir(workspace.path())
        .output()
        .expect("run jx binary");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .expect("stderr is utf-8")
        .contains("fixed `origin` remote is missing"));
}

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let unique = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("jx-cli-test-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root).expect("create workspace root");
        let settings =
            UserSettings::from_config(StackedConfig::with_defaults()).expect("test settings");
        pollster::block_on(Workspace::init_internal_git(&settings, &root))
            .expect("initialize jj workspace");
        Self { root }
    }

    fn path(&self) -> PathBuf {
        self.root.clone()
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
