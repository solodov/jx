//! Black-box coverage for the revision-scoped CLI and its cross-process coordination.

use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::Duration,
};

#[path = "change/help.rs"]
mod help;

#[test]
fn creates_and_edits_changes_without_switching_or_snapshotting_the_checkout() {
    let repo = TestRepo::new();
    let current = repo.current_commit();
    fs::write(repo.root.join("file.txt"), "unsnapshotted work\n").unwrap();
    let created = repo.new_change("@");
    assert_eq!(repo.current_commit(), current);
    assert_eq!(
        repo.read(&created, "file.txt")["content"],
        "alpha beta gamma\n"
    );
    let applied = repo.apply(&created, json!([
        { "op": "edit", "path": "file.txt", "edits": [{ "old_text": "alpha", "new_text": "updated" }] },
        { "op": "write", "path": "nested/new.txt", "content": "new file\n" },
        { "op": "move", "from": "nested/new.txt", "to": "other/moved.txt" },
        { "op": "write", "path": "temporary.txt", "content": "temporary" },
        { "op": "delete", "path": "temporary.txt" }
    ]));
    assert_eq!(
        repo.read(&applied, "file.txt")["content"],
        "updated beta gamma\n"
    );
    assert_eq!(
        repo.read(&applied, "other/moved.txt")["content"],
        "new file\n"
    );
    assert_eq!(repo.current_commit(), current);
    assert_eq!(
        fs::read_to_string(repo.root.join("file.txt")).unwrap(),
        "unsnapshotted work\n"
    );
    assert!(!repo.root.join("other").exists());
    let search = repo.json(&[
        "change",
        "search",
        change_id(&applied),
        "--pattern",
        "updated",
        "--json",
    ]);
    assert!(search["output"]
        .as_str()
        .unwrap()
        .contains("file.txt:1:updated"));
}

#[cfg(unix)]
#[test]
fn file_reads_treat_paths_as_literals_not_fileset_expressions() {
    let repo = TestRepo::new();
    let target = repo.new_change("@");
    let applied = repo.apply(
        &target,
        json!([
            { "op": "write", "path": "literal", "content": "wrong file" },
            { "op": "write", "path": "glob:literal", "content": "exact path" }
        ]),
    );
    assert_eq!(repo.read(&applied, "glob:literal")["content"], "exact path");
}

#[test]
fn a_failed_batch_publishes_nothing_and_a_noop_keeps_the_operation() {
    let repo = TestRepo::new();
    let target = repo.new_change("@");
    let failed = repo.apply_output(&target, json!([
        { "op": "write", "path": "new.txt", "content": "must not publish" },
        { "op": "edit", "path": "file.txt", "edits": [{ "old_text": "missing", "new_text": "bad" }] }
    ]));
    assert!(!failed.status.success());
    assert_eq!(json_output(failed)["error"]["code"], "invalid_request");
    let unchanged = repo.read(&target, "file.txt");
    assert_eq!(unchanged["snapshot"], target["snapshot"]);
    assert!(!repo
        .jj(&["file", "list", "-r", change_id(&target)])
        .contains("new.txt"));
    let noop = repo.apply(
        &target,
        json!([{ "op": "write", "path": "file.txt", "content": "alpha beta gamma\n" }]),
    );
    assert_eq!(noop["changed"], false);
    assert_eq!(noop["snapshot"], target["snapshot"]);
}

#[test]
fn immutable_changes_and_overwriting_moves_are_rejected_without_publication() {
    let repo = TestRepo::new();
    let target = repo.new_change("@");
    let output = repo.apply_output(
        &target,
        json!([
            { "op": "write", "path": "destination", "content": "must not publish" },
            { "op": "move", "from": "file.txt", "to": "destination" }
        ]),
    );
    assert_eq!(json_output(output)["error"]["code"], "invalid_request");
    assert_eq!(
        repo.read(&target, "file.txt")["snapshot"],
        target["snapshot"]
    );
    repo.jj(&[
        "config",
        "set",
        "--repo",
        "revset-aliases.\"immutable_heads()\"",
        commit_id(&target),
    ]);
    let output = repo.apply_output(
        &target,
        json!([{ "op": "write", "path": "file.txt", "content": "bad" }]),
    );
    assert_eq!(json_output(output)["error"]["code"], "invalid_request");
    assert_eq!(
        repo.read(&target, "file.txt")["snapshot"],
        target["snapshot"]
    );
}

#[cfg(unix)]
#[test]
fn an_interrupted_writer_stops_while_waiting_for_the_gate() {
    let repo = TestRepo::new();
    let target = repo.new_change("@");
    let _lock = repo.lock();
    let mut child = repo.spawn_apply(
        &repo.root,
        &target,
        json!([{ "op": "write", "path": "file.txt", "content": "must not publish" }]),
    );
    thread::sleep(Duration::from_millis(150));
    // SAFETY: This is our live child process, and SIGINT is handled cooperatively by jx.
    assert_eq!(
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
        0
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("interrupted writer did not stop while waiting for the lock");
        }
        thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130));
    assert_eq!(json_output(output)["error"]["code"], "interrupted");
    assert_eq!(
        repo.read(&target, "file.txt")["snapshot"],
        target["snapshot"]
    );
}

#[test]
fn stale_versions_and_hidden_commit_ids_cannot_overwrite_newer_edits() {
    let repo = TestRepo::new();
    let old = repo.new_change("@");
    let updated = repo.apply(
        &old,
        json!([{ "op": "write", "path": "file.txt", "content": "newer\n" }]),
    );
    for revision in [change_id(&old), commit_id(&old)] {
        let output = repo.apply_at(
            &repo.root,
            revision,
            commit_id(&old),
            json!([{ "op": "write", "path": "file.txt", "content": "stale\n" }]),
        );
        assert!(!output.status.success());
        let error = json_output(output);
        assert_eq!(error["error"]["code"], "stale_change");
        assert_eq!(error["error"]["actual"], commit_id(&updated));
    }
    assert_eq!(repo.read(&updated, "file.txt")["content"], "newer\n");
}

#[test]
fn abandoned_and_divergent_changes_return_explicit_errors() {
    let repo = TestRepo::new();
    let abandoned = repo.new_change("@");
    repo.jj(&["abandon", "-r", change_id(&abandoned)]);
    let output = repo.apply_output(
        &abandoned,
        json!([{ "op": "write", "path": "file.txt", "content": "bad" }]),
    );
    assert_eq!(json_output(output)["error"]["code"], "abandoned_change");

    let target = repo.new_change("@");
    let base_op = target["snapshot"]["operation_id"].as_str().unwrap();
    repo.jj(&[
        "--ignore-working-copy",
        "describe",
        "-r",
        change_id(&target),
        "-m",
        "first version",
    ]);
    repo.jj(&[
        "--at-operation",
        base_op,
        "describe",
        "-r",
        change_id(&target),
        "-m",
        "second version",
    ]);
    let concurrent = repo
        .command(env!("CARGO_BIN_EXE_jx"), &repo.root)
        .args(["change", "read", change_id(&target), "file.txt", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        json_output(concurrent)["error"]["code"],
        "concurrent_operations"
    );
    repo.jj(&["status"]);
    let output = repo.apply_output(
        &target,
        json!([{ "op": "write", "path": "file.txt", "content": "bad" }]),
    );
    assert_eq!(json_output(output)["error"]["code"], "divergent_change");
}

#[test]
fn parallel_edits_have_one_winner_and_one_stale_request() {
    let repo = TestRepo::new();
    let target = repo.new_change("@");
    let first = repo.spawn_apply(
        &repo.root,
        &target,
        json!([{ "op": "write", "path": "first", "content": "one" }]),
    );
    let second = repo.spawn_apply(
        &repo.root,
        &target,
        json!([{ "op": "write", "path": "second", "content": "two" }]),
    );
    let results = [
        first.wait_with_output().unwrap(),
        second.wait_with_output().unwrap(),
    ];
    assert_eq!(
        results
            .iter()
            .filter(|result| result.status.success())
            .count(),
        1
    );
    for result in results {
        if !result.status.success() {
            assert_eq!(json_output(result)["error"]["code"], "stale_change");
        }
    }
    // A divergent change would make this single-revision read fail.
    repo.read(&target, "file.txt");
}

#[test]
fn linked_workspaces_share_the_lock_and_unrelated_operations_do_not_stale_an_edit() {
    let repo = TestRepo::new();
    let target = repo.new_change("@");
    let linked = repo.directory.path().join("linked");
    repo.jj(&[
        "workspace",
        "add",
        "--name",
        "linked",
        linked.to_str().unwrap(),
    ]);
    let lock = repo.lock();
    let mut child = repo.spawn_apply(
        &linked,
        &target,
        json!([{ "op": "write", "path": "from-linked", "content": "ok" }]),
    );
    thread::sleep(Duration::from_millis(150));
    assert!(
        child.try_wait().unwrap().is_none(),
        "writer bypassed the shared repository lock"
    );
    drop(lock);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let applied = json_output(output);
    assert_eq!(repo.read(&applied, "from-linked")["content"], "ok");
}

#[test]
fn validation_runs_outside_the_gate_and_discards_file_changes() {
    let repo = TestRepo::new();
    let target = repo.new_change("@");
    let _lock = repo.lock();
    let result = repo.json(&[
        "change",
        "run",
        commit_id(&target),
        "--json",
        "--",
        "sh",
        "-c",
        "printf 'temporary\\n' > file.txt",
    ]);
    assert_eq!(result["exit_code"], 0);
    assert_eq!(result["snapshot"], target["snapshot"]);
    assert_eq!(
        repo.read(&target, "file.txt")["content"],
        "alpha beta gamma\n"
    );
}

#[test]
fn failed_validation_reports_failure_without_rewriting_the_change() {
    let repo = TestRepo::new();
    let target = repo.new_change("@");
    let output = repo
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
    assert!(!output.status.success());
    let result = json_output(output);
    assert_ne!(result["exit_code"], 0);
    assert_eq!(result["snapshot"], target["snapshot"]);
    assert_eq!(
        repo.read(&target, "file.txt")["snapshot"],
        target["snapshot"]
    );
}

#[test]
fn parent_edits_rebase_descendants_and_report_conflicts_without_updating_disk() {
    let repo = TestRepo::new();
    let parent = repo.new_change("@");
    let child = repo.new_change(change_id(&parent));
    let child = repo.apply(
        &child,
        json!([{ "op": "write", "path": "file.txt", "content": "child\n" }]),
    );
    let applied = repo.apply(
        &parent,
        json!([{ "op": "write", "path": "file.txt", "content": "parent\n" }]),
    );
    assert!(applied["conflicted_changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id == change_id(&child)));
    assert_eq!(
        fs::read_to_string(repo.root.join("file.txt")).unwrap(),
        "alpha beta gamma\n"
    );
    assert_eq!(repo.read(&applied, "file.txt")["content"], "parent\n");
}

#[test]
fn explicit_writes_ignore_checkout_ignore_and_auto_track_settings() {
    let repo = TestRepo::new();
    fs::write(repo.root.join(".gitignore"), "ignored/\n").unwrap();
    repo.jj(&["describe", "-m", "base with ignores"]);
    repo.jj(&["config", "set", "--repo", "snapshot.auto-track", "none()"]);
    let target = repo.new_change("@");
    let applied = repo.apply(
        &target,
        json!([{ "op": "write", "path": "ignored/explicit.txt", "content": "requested\n" }]),
    );
    assert_eq!(
        repo.read(&applied, "ignored/explicit.txt")["content"],
        "requested\n"
    );
}

#[test]
fn occupied_revisions_are_rejected_so_later_jj_commands_preserve_dirty_work() {
    for colocated in [false, true] {
        let repo = TestRepo::with_colocation(colocated);
        let parent = repo.json(&["change", "read", "@", "file.txt", "--json"]);
        fs::write(repo.root.join("file.txt"), "unsnapshotted work\n").unwrap();
        let noop = repo.apply(
            &parent,
            json!([{ "op": "write", "path": "file.txt", "content": "alpha beta gamma\n" }]),
        );
        assert_eq!(noop["changed"], false);
        let output = repo.apply_output(
            &parent,
            json!([{ "op": "write", "path": "added.txt", "content": "must not publish\n" }]),
        );
        assert!(!output.status.success());
        let error = json_output(output);
        assert_eq!(error["error"]["code"], "checked_out_changes");
        assert_eq!(error["error"]["workspaces"], json!(["default"]));
        assert_eq!(
            repo.read(&parent, "file.txt")["snapshot"],
            parent["snapshot"]
        );
        repo.jj(&["status"]);
        assert_eq!(
            fs::read_to_string(repo.root.join("file.txt")).unwrap(),
            "unsnapshotted work\n"
        );
        assert!(!repo.root.join("added.txt").exists());
    }
}

#[test]
fn ancestors_of_other_workspaces_are_protected_until_the_workspace_is_forgotten() {
    let repo = TestRepo::new();
    let target = repo.new_change("@");
    let linked = repo.directory.path().join("linked");
    repo.jj(&[
        "workspace",
        "add",
        "-r",
        change_id(&target),
        "--name",
        "linked",
        linked.to_str().unwrap(),
    ]);
    let output = repo.apply_output(
        &target,
        json!([{ "op": "write", "path": "file.txt", "content": "blocked" }]),
    );
    assert_eq!(
        json_output(output)["error"]["workspaces"],
        json!(["linked"])
    );
    repo.jj(&["workspace", "forget", "linked"]);
    let result = repo.apply(
        &target,
        json!([{ "op": "write", "path": "file.txt", "content": "allowed" }]),
    );
    assert_eq!(repo.read(&result, "file.txt")["content"], "allowed");
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed_and_writes_preserve_executable_modes() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};
    let repo = TestRepo::new();
    let outside = repo.directory.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, repo.root.join("link")).unwrap();
    fs::set_permissions(
        repo.root.join("file.txt"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    repo.jj(&["describe", "-m", "base with symlink"]);
    let target = repo.new_change("@");
    let failed = repo.apply_output(
        &target,
        json!([{ "op": "write", "path": "link/outside", "content": "bad" }]),
    );
    assert!(!failed.status.success());
    assert!(!outside.join("outside").exists());
    let applied = repo.apply(&target, json!([{ "op": "write", "path": "file.txt", "content": "updated" }, { "op": "move", "from": "file.txt", "to": "renamed" }]));
    let mode = repo.jj(&[
        "file",
        "list",
        "-r",
        change_id(&applied),
        "-T",
        "if(executable, path ++ \"\\n\")",
    ]);
    assert!(mode.contains("renamed"));
}

struct TestRepo {
    directory: tempfile::TempDir,
    root: PathBuf,
    config: PathBuf,
}

impl TestRepo {
    fn new() -> Self {
        Self::with_colocation(false)
    }

    fn with_colocation(colocated: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("repo");
        fs::create_dir(&root).unwrap();
        let config = directory.path().join("jj.toml");
        fs::write(&config, "[user]\nname = 'Change Test'\nemail = 'change@example.invalid'\n[revset-aliases]\n'immutable_heads()' = 'root()'\n").unwrap();
        let repo = Self {
            directory,
            root,
            config,
        };
        repo.jj(&[
            "git",
            "init",
            if colocated {
                "--colocate"
            } else {
                "--no-colocate"
            },
        ]);
        fs::write(repo.root.join("file.txt"), "alpha beta gamma\n").unwrap();
        repo.jj(&["describe", "-m", "base"]);
        repo
    }

    fn command(&self, binary: &str, root: &Path) -> Command {
        let mut command = Command::new(binary);
        command
            .current_dir(root)
            .env("JJ_CONFIG", &self.config)
            .env("XDG_CONFIG_HOME", self.directory.path().join("config"))
            .env("XDG_STATE_HOME", self.directory.path().join("state"))
            .env("HOME", self.directory.path());
        command
    }

    fn jj(&self, args: &[&str]) -> String {
        let output = self
            .command("jj", &self.root)
            .args(["--no-pager", "--color=never"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "jj {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn current_commit(&self) -> String {
        self.jj(&[
            "--ignore-working-copy",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            "commit_id",
        ])
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self
            .command(env!("CARGO_BIN_EXE_jx"), &self.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "jx {args:?}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        json_output(output)
    }

    fn new_change(&self, from: &str) -> Value {
        self.json(&[
            "change",
            "new",
            "--from",
            from,
            "-m",
            "agent work",
            "--json",
        ])
    }

    fn read(&self, target: &Value, path: &str) -> Value {
        self.json(&["change", "read", change_id(target), path, "--json"])
    }

    fn apply(&self, target: &Value, operations: Value) -> Value {
        let output = self.apply_output(target, operations);
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        json_output(output)
    }

    fn apply_output(&self, target: &Value, operations: Value) -> Output {
        self.spawn_apply(&self.root, target, operations)
            .wait_with_output()
            .unwrap()
    }

    fn spawn_apply(&self, root: &Path, target: &Value, operations: Value) -> Child {
        self.spawn_apply_at(root, change_id(target), commit_id(target), operations)
    }

    fn apply_at(&self, root: &Path, revision: &str, expected: &str, operations: Value) -> Output {
        self.spawn_apply_at(root, revision, expected, operations)
            .wait_with_output()
            .unwrap()
    }

    fn spawn_apply_at(
        &self,
        root: &Path,
        revision: &str,
        expected: &str,
        operations: Value,
    ) -> Child {
        let mut child = self
            .command(env!("CARGO_BIN_EXE_jx"), root)
            .args([
                "change", "apply", revision, "--expect", expected, "--stdin", "--json",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        serde_json::to_writer(
            child.stdin.take().unwrap(),
            &json!({ "operations": operations }),
        )
        .unwrap();
        child
    }

    fn lock(&self) -> fs::File {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join(".jj/repo/jx-mutation.lock"))
            .unwrap();
        file.lock().unwrap();
        file
    }
}

fn change_id(target: &Value) -> &str {
    target["snapshot"]["change_id"].as_str().unwrap()
}
fn commit_id(target: &Value) -> &str {
    target["snapshot"]["commit_id"].as_str().unwrap()
}
fn json_output(output: Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "not JSON: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
