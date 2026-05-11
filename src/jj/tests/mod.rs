use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use jj_lib::{
    backend::{CopyId, TreeValue},
    commit::Commit,
    merge::Merge,
    merged_tree_builder::MergedTreeBuilder,
    op_store::{RefTarget, RemoteRef, RemoteRefState},
    ref_name::{RefName, RemoteName},
    repo::{MutableRepo, Repo as _},
    repo_path::RepoPathBuf,
};

use super::*;

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn short_commit_ids_are_eight_hex_characters() {
    // Verifies: Short commit IDs are eight hex characters.
    let commit_id = CommitId::from_hex("0123456789abcdef");

    assert_eq!(short_commit_id(&commit_id), "01234567");
}

#[test]
fn external_diff_args_append_jj_paths_after_configured_and_extra_args() {
    // Verifies: External tools receive configured/extra args before jj's left/right trees.
    let tool = ExternalDiffTool {
        command: "difft".to_owned(),
        args: vec![
            "--display=side-by-side".to_owned(),
            "--display=inline".to_owned(),
        ],
    };

    assert_eq!(
        external_diff_args(&tool),
        vec![
            "--display=side-by-side",
            "--display=inline",
            "$left",
            "$right"
        ]
    );
}

#[test]
fn no_tests_filter_keeps_source_paths_and_excludes_common_test_paths() {
    // Verifies: Diff test exclusion preserves source paths while dropping common test conventions.
    let paths = [
        "src/main.rs",
        "tests/cli.rs",
        "pkg/test/helper.js",
        "web/__tests__/view.tsx",
        "cmd/foo_test.go",
        "scripts/test_data.py",
        "scripts/test_runner.py",
        "frontend/button.test.tsx",
        "frontend/button.spec.tsx",
        "src/FooTest.java",
        "src/FooTests.kt",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();

    assert_eq!(diff_paths_without_tests(&paths), vec!["src/main.rs"]);
}

#[test]
fn workspace_list_parser_extracts_workspace_names() {
    // Verifies: Workspace list parsing uses jj's stable `name:` prefix and ignores blank lines.
    let names =
        workspace_names_from_jj_list("default: abcdef12 (empty) primary\nfix: 12345678 work\n\n");

    assert_eq!(names, vec!["default", "fix"]);
}

#[test]
fn workspace_cleanup_removes_empty_managed_parents() {
    // Verifies: Workspace removal prunes the repo layout directory and `.work` when both are empty.
    let fixture = TestWorkspace::new("workspace-cleanup-empty");
    let cleanup_root = fixture.path().join(".work");
    let workspace_root = cleanup_root.join("jx/fix");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::remove_dir_all(&workspace_root).expect("delete workspace root");

    remove_empty_workspace_dirs(&workspace_root, &cleanup_root).expect("cleanup succeeds");

    assert!(!cleanup_root.exists());
}

#[test]
fn workspace_cleanup_keeps_non_empty_managed_parents() {
    // Verifies: Workspace cleanup stops before directories that still contain other work.
    let fixture = TestWorkspace::new("workspace-cleanup-non-empty");
    let cleanup_root = fixture.path().join(".work");
    let workspace_root = cleanup_root.join("jx/fix");
    let sibling = cleanup_root.join("jx/other");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&sibling).expect("create sibling workspace");
    fs::remove_dir_all(&workspace_root).expect("delete workspace root");

    remove_empty_workspace_dirs(&workspace_root, &cleanup_root).expect("cleanup succeeds");

    assert!(cleanup_root.join("jx").exists());
    assert!(cleanup_root.exists());
}

#[test]
fn workspace_status_parser_reuses_jj_commit_and_change_lines() {
    // Verifies: Status parsing preserves jj-rendered commit and file summary lines for reordering.
    let status = workspace_status_from_jj_status(
            "Working copy changes:\nM README.md\nM src/commands.rs\nWorking copy  (@) : kvxvwztp b9e8f888\nParent commit (@-): xskrmynn 6257dd5a main | parent\n",
            "Add status output".to_owned(),
        );

    assert_eq!(
        status,
        WorkspaceStatus {
            commit_lines: vec![
                "Working copy  (@) : kvxvwztp b9e8f888".to_owned(),
                "Parent commit (@-): xskrmynn 6257dd5a main | parent".to_owned(),
            ],
            description: "Add status output".to_owned(),
            change_lines: vec!["M README.md".to_owned(), "M src/commands.rs".to_owned()],
            extra_lines: Vec::new(),
        }
    );
}

#[test]
fn workspace_status_parser_keeps_no_change_summary() {
    // Verifies: Empty jj status output still produces a status line after the description.
    let status = workspace_status_from_jj_status(
            "The working copy has no changes.\nWorking copy  (@) : abcdef12 11112222\nParent commit (@-): 33334444 55556666 main | parent\n",
            "No local changes".to_owned(),
        );

    assert_eq!(
        status.change_lines,
        vec!["The working copy has no changes.".to_owned()]
    );
}

#[test]
fn workspace_status_parser_preserves_jj_colors() {
    // Verifies: Colored jj status lines survive parsing so jx status can render like jj.
    let status = workspace_status_from_jj_status(
            "Working copy changes:\n\x1b[38;5;6mM README.md\x1b[39m\nWorking copy  (@) : \x1b[1m\x1b[38;5;13mk\x1b[38;5;8mvxvwztp\x1b[0m\n",
            "Colored status".to_owned(),
        );

    assert_eq!(
        status.commit_lines,
        vec!["Working copy  (@) : \x1b[1m\x1b[38;5;13mk\x1b[38;5;8mvxvwztp\x1b[0m".to_owned()]
    );
    assert_eq!(
        status.change_lines,
        vec!["\x1b[38;5;6mM README.md\x1b[39m".to_owned()]
    );
}

#[test]
fn workspace_log_filters_other_workspace_heads() {
    // Verifies: Workspace log filters other workspace heads.
    let fixture = TestWorkspace::new("workspace-log");
    let settings = log_test_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current workspace change").await;
        let other = write_child(tx.repo_mut(), &trunk, "other workspace change").await;

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        tx.repo_mut()
            .set_wc_commit(WorkspaceNameBuf::from("other"), other.id().clone())
            .expect("set other working-copy change");

        let repo = tx
            .commit("arrange multi-workspace log")
            .await
            .expect("commit");
        (workspace, repo)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path())
        .expect("log renders");

    assert!(log.contains("current workspace change"), "{log}");
    assert!(log.contains("main trunk"), "{log}");
    assert!(!log.contains("other workspace change"), "{log}");
}

#[test]
fn facts_report_trunk_target_bookmarks_and_stack_index() {
    // Verifies: Facts report trunk, selected-change bookmarks, and stack index.
    let fixture = TestWorkspace::new("linear");
    let settings = user_settings().expect("settings");
    let (workspace, repo, trunk, ancestor, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let ancestor = write_child(tx.repo_mut(), &trunk, "ancestor change").await;
        let current = write_child(tx.repo_mut(), &ancestor, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "example-user/00-ancestor", ancestor.id());
        set_local_bookmark(tx.repo_mut(), "example-user/01-current", current.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange linear jj workspace")
            .await
            .expect("commit");
        (workspace, repo, trunk, ancestor, current)
    });
    let workspace_root = workspace.workspace_root().to_path_buf();
    let subject = JjWorkspace { workspace, repo };

    let facts = subject.facts().expect("load workspace facts");

    assert_eq!(facts.workspace_root, workspace_root);
    assert_eq!(facts.origin_branch, "main");
    assert_eq!(facts.trunk.branch, "main");
    assert_eq!(facts.trunk.commit_id, trunk.id().hex());
    assert_eq!(facts.trunk_git_commit_sha, trunk.id().hex());
    assert_eq!(facts.target_change.commit_id, current.id().hex());
    assert_eq!(facts.target_change.description, "current change");
    assert!(facts.target_change.is_empty);
    assert_eq!(facts.local_bookmarks_at_target, ["example-user/01-current"]);
    assert_eq!(
        facts.nearest_ancestor_bookmark.as_deref(),
        Some("example-user/00-ancestor")
    );
    assert_eq!(facts.stack_index, 1);
    assert!(facts.changed_files.is_empty());
    assert_eq!(
        facts.target_change.short_commit_id.len(),
        SHORT_COMMIT_ID_LEN
    );
    assert_eq!(facts.trunk.short_commit_id.len(), SHORT_COMMIT_ID_LEN);
    assert_ne!(ancestor.id(), current.id());
}

#[test]
fn facts_report_changed_files_for_selected_commit() {
    // Verifies: Facts expose changed files for reviewer ownership rule matching.
    let fixture = TestWorkspace::new("changed-files");
    let settings = user_settings().expect("settings");
    let (workspace, repo, ancestor, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let ancestor = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "selected ancestor",
            &[("README.md", b"readme\n".as_slice())],
        )
        .await;
        let current = write_child_with_files(
            tx.repo_mut(),
            &ancestor,
            "current change",
            &[
                ("README.md", b"updated readme\n".as_slice()),
                ("src/main.rs", b"fn main() {}\n".as_slice()),
            ],
        )
        .await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange changed-file jj workspace")
            .await
            .expect("commit");
        (workspace, repo, ancestor, current)
    });
    let subject = JjWorkspace { workspace, repo };

    let current_facts = subject.facts().expect("current facts load");
    let ancestor_facts = subject
        .facts_for_revision(Some(&ancestor.id().hex()))
        .expect("ancestor facts load");

    assert_eq!(current_facts.target_change.commit_id, current.id().hex());
    assert_eq!(
        current_facts.changed_files,
        vec!["README.md".to_owned(), "src/main.rs".to_owned()]
    );
    assert_eq!(ancestor_facts.changed_files, vec!["README.md".to_owned()]);
}

#[test]
fn facts_for_revision_reports_selected_commit_without_changing_workspace() {
    // Verifies: Facts for revision reports selected commit without changing workspace.
    let fixture = TestWorkspace::new("selected-revision");
    let settings = user_settings().expect("settings");
    let (workspace, repo, ancestor, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let ancestor = write_child(tx.repo_mut(), &trunk, "selected ancestor").await;
        let current = write_child(tx.repo_mut(), &ancestor, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange selectable revision workspace")
            .await
            .expect("commit");
        (workspace, repo, ancestor, current)
    });
    let selected_revision = ancestor.id().hex();
    let subject = JjWorkspace { workspace, repo };

    let facts = subject
        .facts_for_revision(Some(&selected_revision))
        .expect("selected revision facts load");

    assert_eq!(facts.target_change.commit_id, selected_revision);
    assert_eq!(facts.target_change.description, "selected ancestor");
    assert_eq!(facts.stack_index, 0);
    assert_eq!(
        subject.current_commit().expect("current commit").id(),
        current.id()
    );
}

#[test]
fn pull_request_candidates_include_linear_descendant_bookmarks() {
    // Verifies: Opening a PR from a lower stack change can use a bookmark on its direct descendant.
    let fixture = TestWorkspace::new("pr-candidate-descendant");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let selected = write_child(tx.repo_mut(), &trunk, "selected change").await;
        let descendant = write_child(tx.repo_mut(), &selected, "review head").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "review/descendant", descendant.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), selected.id().clone())
            .expect("set selected working-copy change");

        let repo = tx
            .commit("arrange descendant bookmark")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let candidates = subject
        .pull_request_candidate_bookmarks(None)
        .expect("candidate bookmarks load");

    assert_eq!(candidates, ["review/descendant"]);
}

#[test]
fn pull_request_candidates_order_selected_bookmarks_before_descendants() {
    // Verifies: A PR directly attached to the selected change wins over descendant fallback heads.
    let fixture = TestWorkspace::new("pr-candidate-order");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let selected = write_child(tx.repo_mut(), &trunk, "selected change").await;
        let descendant = write_child(tx.repo_mut(), &selected, "review head").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "review/selected", selected.id());
        set_local_bookmark(tx.repo_mut(), "review/descendant", descendant.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), selected.id().clone())
            .expect("set selected working-copy change");

        let repo = tx.commit("arrange bookmark order").await.expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let candidates = subject
        .pull_request_candidate_bookmarks(None)
        .expect("candidate bookmarks load");

    assert_eq!(candidates, ["review/selected", "review/descendant"]);
}

#[test]
fn push_facts_reuse_local_bookmark_without_origin_trunk() {
    // Verifies: Push planning can reuse a local bookmark before any origin bookmark exists.
    let fixture = TestWorkspace::new("push-facts-local-bookmark-no-trunk");
    let settings = user_settings().expect("settings");
    let (workspace, repo, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child(tx.repo_mut(), &root, "current change").await;

        set_local_bookmark(tx.repo_mut(), "main", current.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange local bookmark without origin trunk")
            .await
            .expect("commit");
        (workspace, repo, current)
    });
    let subject = JjWorkspace { workspace, repo };

    let facts = subject
        .push_facts_for_revision(None)
        .expect("push facts load from local bookmark");

    assert_eq!(facts.target_change.commit_id, current.id().hex());
    assert_eq!(facts.local_bookmarks_at_target, ["main"]);
    assert_eq!(facts.origin_branch, "main");
    assert_eq!(facts.stack_index, 0);
    assert_eq!(facts.nearest_ancestor_bookmark, None);
}

#[test]
fn facts_prefer_main_when_other_origin_bookmarks_are_also_ancestors() {
    // Verifies: Facts prefer main when other origin bookmarks are also ancestors.
    let fixture = TestWorkspace::new("main-preferred");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_origin_bookmark(tx.repo_mut(), "release", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange preferred trunk workspace")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let facts = subject.facts().expect("main trunk is preferred");

    assert_eq!(facts.origin_branch, "main");
}

#[test]
fn status_facts_reports_each_requested_remote_trunk() {
    // Verifies: Status facts resolve a local cached trunk for each requested remote.
    let fixture = TestWorkspace::new("status-remotes");
    let settings = user_settings().expect("settings");
    let (workspace, repo, origin_trunk, upstream_trunk) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let origin_trunk = write_child(tx.repo_mut(), &root, "origin trunk").await;
        let upstream_trunk = write_child_with_files(
            tx.repo_mut(),
            &origin_trunk,
            "upstream trunk",
            &[("upstream.txt", b"upstream")],
        )
        .await;
        let current = write_child_with_files(
            tx.repo_mut(),
            &upstream_trunk,
            "current change",
            &[("current.txt", b"current")],
        )
        .await;

        set_remote_bookmark(tx.repo_mut(), "origin", "main", origin_trunk.id());
        set_remote_bookmark(tx.repo_mut(), "upstream", "main", upstream_trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange status remotes workspace")
            .await
            .expect("commit");
        (workspace, repo, origin_trunk, upstream_trunk)
    });
    let subject = JjWorkspace { workspace, repo };

    let facts = subject
        .status_facts(["origin", "upstream"])
        .expect("status facts load");

    assert_eq!(facts.remotes.len(), 2);
    assert_eq!(facts.remotes[0].remote, "origin");
    assert_eq!(
        facts.remotes[0].trunk_git_commit_sha,
        origin_trunk.id().hex()
    );
    assert_eq!(facts.remotes[0].local_ahead_by, 2);
    assert_eq!(facts.remotes[1].remote, "upstream");
    assert_eq!(
        facts.remotes[1].trunk_git_commit_sha,
        upstream_trunk.id().hex()
    );
    assert_eq!(facts.remotes[1].local_ahead_by, 1);
}

#[test]
fn global_fetch_ready_accepts_empty_current_child_of_origin_trunk() {
    // Verifies: Global fetch may safely run when only jj's empty working-copy commit is local.
    let fixture = TestWorkspace::new("global-fetch-ready");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child_with_files(
            tx.repo_mut(),
            &root,
            "main trunk",
            &[("src/lib.rs", b"published")],
        )
        .await;
        let empty_current = write_child(tx.repo_mut(), &trunk, "").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                empty_current.id().clone(),
            )
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange global fetch ready workspace")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    assert!(subject
        .is_empty_working_copy_child_of_origin_trunk()
        .expect("readiness loads"));
}

#[test]
fn global_fetch_ready_rejects_changed_current() {
    // Verifies: Global fetch skips repos with local work above trunk.
    let fixture = TestWorkspace::new("global-fetch-local-work");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child_with_files(
            tx.repo_mut(),
            &root,
            "main trunk",
            &[("src/lib.rs", b"published")],
        )
        .await;
        let current = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "local work",
            &[("src/local.rs", b"local")],
        )
        .await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange global fetch local work workspace")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    assert!(!subject
        .is_empty_working_copy_child_of_origin_trunk()
        .expect("readiness loads"));
}

#[test]
fn status_facts_ignore_empty_working_copy_commit() {
    // Verifies: Remote status does not report an empty jj working-copy commit as unpublished work.
    let fixture = TestWorkspace::new("status-empty-current");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child_with_files(
            tx.repo_mut(),
            &root,
            "main trunk",
            &[("src/lib.rs", b"published")],
        )
        .await;
        let empty_current = write_child(tx.repo_mut(), &trunk, "").await;

        set_remote_bookmark(tx.repo_mut(), "origin", "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                empty_current.id().clone(),
            )
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange empty current status workspace")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let facts = subject.status_facts(["origin"]).expect("status facts load");

    assert_eq!(facts.remotes[0].local_ahead_by, 0);
}

#[test]
fn facts_reject_ambiguous_origin_trunk_candidates() {
    // Verifies: Facts reject ambiguous origin trunk candidates.
    let fixture = TestWorkspace::new("ambiguous-trunk");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "shared trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "release-a", trunk.id());
        set_origin_bookmark(tx.repo_mut(), "release-b", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange ambiguous trunk workspace")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let error = subject.facts().expect_err("trunk is ambiguous");

    assert!(matches!(
        error,
        JjError::AmbiguousTrunk { branches, .. }
            if branches == ["release-a".to_owned(), "release-b".to_owned()]
    ));
}

#[test]
fn facts_reject_non_linear_stack_paths() {
    // Verifies: Workspace facts reject non-linear stack paths.
    let fixture = TestWorkspace::new("non-linear");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let left = write_child(tx.repo_mut(), &trunk, "left branch").await;
        let right = write_child(tx.repo_mut(), &trunk, "right branch").await;
        let merge = tx
            .repo_mut()
            .new_commit(vec![left.id().clone(), right.id().clone()], left.tree())
            .set_description("merge current")
            .write()
            .await
            .expect("write merge commit");

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), merge.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange non-linear jj workspace")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let error = subject.facts().expect_err("non-linear stack is rejected");

    assert!(matches!(error, JjError::NonLinearStack { .. }));
    assert!(error.to_string().contains("has 2 parents"));
}

#[test]
fn select_trunk_candidate_reports_missing_and_conflicted_trunk() {
    // Verifies: Trunk selection reports missing and conflicted origin trunk state.
    let missing = select_trunk_candidate("origin", Vec::new(), Vec::new())
        .expect_err("missing trunk is rejected");
    let conflicted = select_trunk_candidate("origin", Vec::new(), vec!["main".to_owned()])
        .expect_err("conflicted trunk is rejected");

    assert!(matches!(missing, JjError::MissingTrunk { .. }));
    assert!(
        matches!(conflicted, JjError::ConflictedTrunk { branches, .. } if branches == ["main"])
    );
}

#[test]
fn ensure_bookmark_creates_reuses_and_rejects_other_change() {
    // Verifies: Bookmark mutation creates, reuses, and rejects bookmarks on other changes.
    let fixture = TestWorkspace::new("bookmark-mutation");
    let settings = user_settings().expect("settings");
    let (workspace, repo, trunk, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange bookmark mutation workspace")
            .await
            .expect("commit");
        (workspace, repo, trunk, current)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let selected = subject
        .ensure_bookmark("example-user/00-selected", &trunk.id().hex())
        .expect("bookmark is created for selected commit");
    let selected_target = subject
        .repo
        .view()
        .get_local_bookmark(RefName::new("example-user/00-selected"));
    assert_eq!(
        selected,
        BookmarkUpdate {
            branch: "example-user/00-selected".to_owned(),
            created: true,
        }
    );
    assert_eq!(selected_target.as_normal(), Some(trunk.id()));

    let created = subject
        .ensure_bookmark("example-user/00-current", &current.id().hex())
        .expect("bookmark is created");
    let target = subject
        .repo
        .view()
        .get_local_bookmark(RefName::new("example-user/00-current"));

    assert_eq!(
        created,
        BookmarkUpdate {
            branch: "example-user/00-current".to_owned(),
            created: true,
        }
    );
    assert_eq!(target.as_normal(), Some(current.id()));

    let reused = subject
        .ensure_bookmark("example-user/00-current", &current.id().hex())
        .expect("bookmark is reused");

    assert_eq!(
        reused,
        BookmarkUpdate {
            branch: "example-user/00-current".to_owned(),
            created: false,
        }
    );

    let mut tx = subject.repo.start_transaction();
    set_local_bookmark(tx.repo_mut(), "example-user/other", trunk.id());
    subject.repo = pollster::block_on(tx.commit("arrange bookmark conflict")).expect("commit");

    let error = subject
        .ensure_bookmark("example-user/other", &current.id().hex())
        .expect_err("bookmark on another change is rejected");

    assert!(matches!(
        error,
        JjError::BookmarkExistsOnDifferentChange { branch }
            if branch == "example-user/other"
    ));
}

#[test]
fn rebase_on_trunk_moves_current_to_latest_origin_trunk() {
    // Verifies: Rebase-on-trunk uses the latest origin trunk even when it is not an ancestor.
    let fixture = TestWorkspace::new("rebase-on-trunk-current");
    let settings = user_settings().expect("settings");
    let (workspace, repo, updated_trunk, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_trunk = write_child(tx.repo_mut(), &root, "old main trunk").await;
        let current = write_child(tx.repo_mut(), &old_trunk, "current change").await;
        let updated_trunk = write_child(tx.repo_mut(), &old_trunk, "updated main trunk").await;

        set_origin_bookmark(tx.repo_mut(), "main", updated_trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange rebase-on-trunk workspace")
            .await
            .expect("commit");
        (workspace, repo, updated_trunk, current)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .rebase_on_trunk(&[])
        .expect("current change is rebased onto updated trunk");
    let current_id = subject
        .repo
        .view()
        .get_wc_commit_id(subject.workspace.workspace_name())
        .expect("working-copy commit exists");
    let rebased_current = load_commit_from_repo(subject.repo.as_ref(), current_id)
        .expect("load rebased working-copy commit");

    assert_eq!(outcome.branch, "main");
    assert_eq!(
        outcome.source_short_commit_ids,
        vec![short_commit_id(current.id())]
    );
    assert_eq!(
        outcome.trunk_short_commit_id,
        short_commit_id(updated_trunk.id())
    );
    assert_eq!(outcome.rebased_commits, 1);
    assert_eq!(outcome.skipped_commits, 0);
    assert!(outcome.current_updated);
    assert_ne!(current_id, current.id());
    assert!(rebased_current.parent_ids().contains(updated_trunk.id()));
}

#[test]
fn rebase_on_trunk_moves_multiple_sources_to_latest_origin_trunk() {
    // Verifies: Rebase-on-trunk can move several independent source stacks in one transaction.
    let fixture = TestWorkspace::new("rebase-on-trunk-multiple-sources");
    let settings = user_settings().expect("settings");
    let (workspace, repo, updated_trunk, first, second) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_trunk = write_child(tx.repo_mut(), &root, "old main trunk").await;
        let first = write_child(tx.repo_mut(), &old_trunk, "first change").await;
        let second = write_child(tx.repo_mut(), &old_trunk, "second change").await;
        let updated_trunk = write_child(tx.repo_mut(), &old_trunk, "updated main trunk").await;

        set_origin_bookmark(tx.repo_mut(), "main", updated_trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), first.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange multi-source rebase-on-trunk workspace")
            .await
            .expect("commit");
        (workspace, repo, updated_trunk, first, second)
    });
    let mut subject = JjWorkspace { workspace, repo };
    let sources = vec![first.id().hex(), second.id().hex()];

    let outcome = subject
        .rebase_on_trunk(&sources)
        .expect("both sources are rebased onto updated trunk");
    let rebased_first_id = subject
        .repo
        .resolve_change_id(first.change_id())
        .expect("first change lookup succeeds")
        .expect("first change remains visible")
        .into_visible()
        .expect("first change has a visible commit")
        .into_iter()
        .next()
        .expect("first visible commit exists");
    let rebased_second_id = subject
        .repo
        .resolve_change_id(second.change_id())
        .expect("second change lookup succeeds")
        .expect("second change remains visible")
        .into_visible()
        .expect("second change has a visible commit")
        .into_iter()
        .next()
        .expect("second visible commit exists");
    let rebased_first = load_commit_from_repo(subject.repo.as_ref(), &rebased_first_id)
        .expect("load first rebased commit");
    let rebased_second = load_commit_from_repo(subject.repo.as_ref(), &rebased_second_id)
        .expect("load second rebased commit");

    assert_eq!(
        outcome.source_short_commit_ids,
        vec![short_commit_id(first.id()), short_commit_id(second.id())]
    );
    assert_eq!(outcome.rebased_commits, 2);
    assert_eq!(outcome.skipped_commits, 0);
    assert!(outcome.current_updated);
    assert_ne!(rebased_first.id(), first.id());
    assert_ne!(rebased_second.id(), second.id());
    assert!(rebased_first.parent_ids().contains(updated_trunk.id()));
    assert!(rebased_second.parent_ids().contains(updated_trunk.id()));
}

#[test]
fn fetch_origin_refs_selects_trunk_and_tracked_bookmarks_only() {
    // Verifies: Fetch avoids enumerating every remote bookmark in large repositories.
    let fixture = TestWorkspace::new("fetch-tracked-bookmarks");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let pr = write_child(tx.repo_mut(), &trunk, "tracked pr").await;
        let old = write_child(tx.repo_mut(), &trunk, "untracked old branch").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_origin_bookmark(tx.repo_mut(), "example-user/pr", pr.id());
        set_untracked_origin_bookmark(tx.repo_mut(), "old-branch", old.id());

        assert_eq!(
            tracked_origin_bookmarks(tx.repo_mut(), "main"),
            vec!["example-user/pr".to_owned(), "main".to_owned()]
        );
    });
}

#[test]
fn push_tracked_selects_tracked_updates_and_deletions() {
    // Verifies: Tracked push sends only tracked origin updates and includes deletions.
    let fixture = TestWorkspace::new("push-tracked-updates");
    let settings = user_settings().expect("settings");
    let (workspace, repo, trunk, current, old) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;
        let old = write_child(tx.repo_mut(), &trunk, "old branch").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", trunk.id());
        set_origin_bookmark(tx.repo_mut(), "feature", trunk.id());
        set_local_bookmark(tx.repo_mut(), "feature", current.id());
        set_origin_bookmark(tx.repo_mut(), "old", old.id());
        set_untracked_origin_bookmark(tx.repo_mut(), "untracked", old.id());

        let repo = tx
            .commit("arrange tracked push updates")
            .await
            .expect("commit");
        (workspace, repo, trunk, current, old)
    });
    let subject = JjWorkspace { workspace, repo };

    let updates = subject
        .tracked_origin_bookmark_updates()
        .expect("tracked updates classify");

    assert_eq!(
        updates
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["feature", "old"]
    );
    assert_eq!(updates[0].1.before.as_ref(), Some(trunk.id()));
    assert_eq!(updates[0].1.after.as_ref(), Some(current.id()));
    assert_eq!(updates[1].1.before.as_ref(), Some(old.id()));
    assert_eq!(updates[1].1.after, None);
}

#[test]
fn advance_trunk_for_sync_moves_main_to_current_and_creates_empty_child() {
    // Verifies: Sync preparation publishes current work locally and leaves the workspace ready.
    let fixture = TestWorkspace::new("advance-trunk-current");
    let settings = user_settings().expect("settings");
    let (workspace, repo, trunk, current) = pollster::block_on(async {
        let (mut workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "current change",
            &[("src/lib.rs", b"published")],
        )
        .await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange sync trunk advance")
            .await
            .expect("commit");
        workspace
            .check_out(repo.op_id().clone(), None, &current)
            .await
            .expect("checkout current working-copy tree");
        (workspace, repo, trunk, current)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .advance_trunk_for_sync()
        .expect("trunk advance succeeds");
    let main_target = subject.repo.view().get_local_bookmark(RefName::new("main"));
    let current_id = subject
        .repo
        .view()
        .get_wc_commit_id(subject.workspace.workspace_name())
        .expect("working-copy commit exists");
    let empty_child = load_commit_from_repo(subject.repo.as_ref(), current_id)
        .expect("load new working-copy commit");
    let empty_child_is_empty =
        pollster::block_on(empty_child.is_empty(subject.repo.as_ref())).expect("check empty child");

    assert_eq!(outcome.branch, "main");
    assert_eq!(outcome.old_short_commit_id, short_commit_id(trunk.id()));
    assert_eq!(outcome.new_short_commit_id, short_commit_id(current.id()));
    assert!(outcome.current_updated);
    assert_eq!(main_target.as_normal(), Some(current.id()));
    assert_ne!(current_id, current.id());
    assert!(empty_child.parent_ids().contains(current.id()));
    assert!(empty_child_is_empty);
    assert!(empty_child.description().trim().is_empty());
}

#[test]
fn advance_trunk_for_sync_reuses_existing_empty_child() {
    // Verifies: Re-running sync preparation keeps an existing empty undescribed child in place.
    let fixture = TestWorkspace::new("advance-trunk-empty-child");
    let settings = user_settings().expect("settings");
    let (workspace, repo, current, empty_child) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "current change",
            &[("src/lib.rs", b"published")],
        )
        .await;
        let empty_child = write_child(tx.repo_mut(), &current, "").await;

        set_origin_bookmark(tx.repo_mut(), "main", current.id());
        set_local_bookmark(tx.repo_mut(), "main", current.id());
        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                empty_child.id().clone(),
            )
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange existing sync empty child")
            .await
            .expect("commit");
        (workspace, repo, current, empty_child)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .advance_trunk_for_sync()
        .expect("trunk advance succeeds");
    let main_target = subject.repo.view().get_local_bookmark(RefName::new("main"));
    let current_id = subject
        .repo
        .view()
        .get_wc_commit_id(subject.workspace.workspace_name())
        .expect("working-copy commit exists");

    assert_eq!(outcome.old_short_commit_id, short_commit_id(current.id()));
    assert_eq!(outcome.new_short_commit_id, short_commit_id(current.id()));
    assert!(!outcome.current_updated);
    assert_eq!(main_target.as_normal(), Some(current.id()));
    assert_eq!(current_id, empty_child.id());
}

#[test]
fn advance_trunk_for_sync_stops_before_empty_described_tip() {
    // Verifies: Sync only advances trunk through commits with both changes and descriptions.
    let fixture = TestWorkspace::new("advance-trunk-empty-described-tip");
    let settings = user_settings().expect("settings");
    let (workspace, repo, trunk, published, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let published = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "published change",
            &[("src/lib.rs", b"published")],
        )
        .await;
        let current = write_child(tx.repo_mut(), &published, "next change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange described empty sync tip")
            .await
            .expect("commit");
        (workspace, repo, trunk, published, current)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .advance_trunk_for_sync()
        .expect("trunk advance succeeds");
    let main_target = subject.repo.view().get_local_bookmark(RefName::new("main"));
    let current_id = subject
        .repo
        .view()
        .get_wc_commit_id(subject.workspace.workspace_name())
        .expect("working-copy commit exists");

    assert_eq!(outcome.old_short_commit_id, short_commit_id(trunk.id()));
    assert_eq!(outcome.new_short_commit_id, short_commit_id(published.id()));
    assert!(!outcome.current_updated);
    assert_eq!(main_target.as_normal(), Some(published.id()));
    assert_eq!(current_id, current.id());
}

#[test]
fn advance_trunk_for_sync_stops_before_undescribed_changed_tip() {
    // Verifies: Sync keeps changed but undescribed work local instead of publishing it to trunk.
    let fixture = TestWorkspace::new("advance-trunk-undescribed-changed-tip");
    let settings = user_settings().expect("settings");
    let (workspace, repo, trunk, published, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let published = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "published change",
            &[("src/lib.rs", b"published")],
        )
        .await;
        let current = write_child_with_files(
            tx.repo_mut(),
            &published,
            "",
            &[("src/lib.rs", b"in progress")],
        )
        .await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange undescribed changed sync tip")
            .await
            .expect("commit");
        (workspace, repo, trunk, published, current)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .advance_trunk_for_sync()
        .expect("trunk advance succeeds");
    let main_target = subject.repo.view().get_local_bookmark(RefName::new("main"));
    let current_id = subject
        .repo
        .view()
        .get_wc_commit_id(subject.workspace.workspace_name())
        .expect("working-copy commit exists");

    assert_eq!(outcome.old_short_commit_id, short_commit_id(trunk.id()));
    assert_eq!(outcome.new_short_commit_id, short_commit_id(published.id()));
    assert!(!outcome.current_updated);
    assert_eq!(main_target.as_normal(), Some(published.id()));
    assert_eq!(current_id, current.id());
}

#[test]
fn export_git_refs_updates_backing_git_bookmark_view() {
    // Verifies: Git export keeps backing Git branch state aligned with jj bookmarks.
    let fixture = TestWorkspace::new("export-git-refs");
    let settings = user_settings().expect("settings");

    pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;

        set_local_bookmark(tx.repo_mut(), "main", trunk.id());
        export_git_refs(tx.repo_mut()).expect("export local bookmarks");
        let remote = tx
            .repo()
            .view()
            .get_remote_bookmark(
                RefName::new("main").to_remote_symbol(git::REMOTE_NAME_FOR_LOCAL_GIT_REPO),
            )
            .clone();

        assert_eq!(remote.target.as_normal(), Some(trunk.id()));
        assert!(remote.is_tracked());
    });
}

#[test]
fn prepare_initial_publish_target_describes_undescribed_root_child() {
    // Verifies: Repository bootstrap can publish a fresh jj repo whose first commit lacks text.
    let fixture = TestWorkspace::new("prepare-initial-description");
    let settings = user_settings().expect("settings");
    let (workspace, repo, initial) = pollster::block_on(async {
        let (mut workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let initial =
            write_child_with_files(tx.repo_mut(), &root, "", &[("README.md", b"hello\n")]).await;

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), initial.id().clone())
            .expect("set current working-copy change");
        let repo = tx
            .commit("arrange undescribed initial commit")
            .await
            .expect("commit");
        workspace
            .check_out(repo.op_id().clone(), None, &initial)
            .await
            .expect("checkout initial working-copy tree");
        (workspace, repo, initial)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let target = subject
        .initial_publish_target()
        .expect("initial publish target exists");
    let prepared = subject
        .prepare_initial_publish_target(&target)
        .expect("initial publish target is described");

    let current_id = subject
        .repo
        .view()
        .get_wc_commit_id(subject.workspace.workspace_name())
        .expect("working-copy commit exists");
    let current = load_commit_from_repo(subject.repo.as_ref(), current_id)
        .expect("load prepared working-copy commit");
    let current_is_empty = pollster::block_on(current.is_empty(subject.repo.as_ref()))
        .expect("check prepared working-copy tree");

    assert_eq!(target.commit_id, initial.id().hex());
    assert!(target.description.is_empty());
    assert_ne!(prepared.commit_id, target.commit_id);
    assert_eq!(prepared.description, "initial commit");
    assert_eq!(current_id.hex(), prepared.commit_id);
    assert_eq!(current.description(), "initial commit");
    assert!(!current_is_empty);
}

#[test]
fn push_bookmark_validates_local_and_remote_state_before_transport() {
    // Verifies: Bookmark push validates local and remote state before transport.
    let fixture = TestWorkspace::new("push-validation");
    let settings = user_settings().expect("settings");
    let (workspace, repo, trunk, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange push validation workspace")
            .await
            .expect("commit");
        (workspace, repo, trunk, current)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let missing = subject
        .push_bookmark("example-user/missing")
        .expect_err("missing local bookmark is rejected before transport");

    assert!(matches!(
        missing,
        JjError::MissingLocalBookmark { branch } if branch == "example-user/missing"
    ));

    let mut tx = subject.repo.start_transaction();
    set_local_bookmark(tx.repo_mut(), "example-user/current", current.id());
    tx.repo_mut().set_remote_bookmark(
        RefName::new("example-user/current").to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME)),
        RemoteRef {
            target: RefTarget::from_legacy_form([trunk.id().clone()], [current.id().clone()]),
            state: RemoteRefState::Tracked,
        },
    );
    subject.repo =
        pollster::block_on(tx.commit("arrange conflicted remote bookmark")).expect("commit");

    let conflicted = subject
        .push_bookmark("example-user/current")
        .expect_err("conflicted remote bookmark is rejected before transport");

    assert!(matches!(
        conflicted,
        JjError::ConflictedRemoteBookmark { branch, remote }
            if branch == "example-user/current" && remote == ORIGIN_REMOTE_NAME
    ));
}

#[test]
fn rebase_trunk_children_onto_updated_trunk_rewrites_surviving_children() {
    // Verifies: Fetch rebase rewrites surviving trunk children onto the updated trunk.
    let fixture = TestWorkspace::new("fetch-rebase-survivors");
    let settings = user_settings().expect("settings");

    pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let child = write_child(tx.repo_mut(), &trunk, "surviving trunk child").await;
        let _descendant = write_child(tx.repo_mut(), &child, "descendant change").await;
        let updated_trunk = write_child(tx.repo_mut(), &trunk, "updated trunk").await;

        let stats = rebase_trunk_children_onto_updated_trunk(
            tx.repo_mut(),
            &[child.id().clone()],
            &updated_trunk,
        )
        .await
        .expect("surviving trunk child is rebased");

        assert_eq!(stats.rebased_trunk_children, 1);
        assert_eq!(stats.rebased_descendants, 1);
        assert_eq!(stats.skipped_trunk_children, 0);
    });
}

#[test]
fn rebase_trunk_children_onto_updated_trunk_abandons_newly_empty_children() {
    // Verifies: Fetch drops local commits whose changes are already present upstream.
    let fixture = TestWorkspace::new("fetch-rebase-empty-child");
    let settings = user_settings().expect("settings");

    pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let child = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "landed local change",
            &[("src/landed.rs", b"landed\n")],
        )
        .await;
        let descendant = write_child_with_files(
            tx.repo_mut(),
            &child,
            "remaining local change",
            &[("src/remaining.rs", b"remaining\n")],
        )
        .await;
        let updated_trunk = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "updated trunk with landed change",
            &[
                ("src/landed.rs", b"landed\n"),
                ("src/upstream.rs", b"upstream\n"),
            ],
        )
        .await;

        let stats = rebase_trunk_children_onto_updated_trunk(
            tx.repo_mut(),
            &[child.id().clone()],
            &updated_trunk,
        )
        .await
        .expect("newly empty child is abandoned");
        let child_visible = tx
            .repo()
            .resolve_change_id(child.change_id())
            .expect("child change lookup succeeds")
            .and_then(|targets| targets.into_visible());
        let descendant_id = tx
            .repo()
            .resolve_change_id(descendant.change_id())
            .expect("descendant change lookup succeeds")
            .expect("descendant remains indexed")
            .into_visible()
            .expect("descendant remains visible")
            .into_iter()
            .next()
            .expect("descendant visible commit exists");
        let rebased_descendant =
            load_commit_from_repo(tx.repo(), &descendant_id).expect("load rebased descendant");

        assert_eq!(stats.rebased_trunk_children, 0);
        assert_eq!(stats.rebased_descendants, 1);
        assert_eq!(stats.abandoned_empty_commits, 1);
        assert!(child_visible.is_none());
        assert_eq!(
            rebased_descendant.parent_ids(),
            [updated_trunk.id().clone()]
        );
    });
}

#[test]
fn rebase_trunk_children_onto_updated_trunk_skips_landed_children() {
    // Verifies: Fetch rebase skips trunk children already contained in the updated trunk.
    let fixture = TestWorkspace::new("fetch-rebase-landed");
    let settings = user_settings().expect("settings");

    pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let child = write_child(tx.repo_mut(), &trunk, "landed trunk child").await;
        let updated_trunk = write_child(tx.repo_mut(), &child, "updated trunk").await;

        let stats = rebase_trunk_children_onto_updated_trunk(
            tx.repo_mut(),
            &[child.id().clone()],
            &updated_trunk,
        )
        .await
        .expect("landed trunk child is skipped");

        assert_eq!(stats.rebased_trunk_children, 0);
        assert_eq!(stats.rebased_descendants, 0);
        assert_eq!(stats.skipped_trunk_children, 1);
    });
}

#[test]
fn repair_immutable_working_copy_checks_out_updated_trunk_when_current_landed() {
    // Verifies: Fetch repair checks out updated trunk when the working-copy change landed.
    let fixture = TestWorkspace::new("fetch-repair-current");
    let settings = user_settings().expect("settings");

    pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let workspace_name = workspace.workspace_name().to_owned();
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let updated_trunk = write_child(tx.repo_mut(), &trunk, "updated trunk").await;
        tx.repo_mut()
            .set_wc_commit(workspace_name.clone(), trunk.id().clone())
            .expect("set current working-copy change");

        let stats = repair_immutable_working_copy(
            tx.repo_mut(),
            workspace_name.clone(),
            trunk.id(),
            trunk.id(),
            &updated_trunk,
        )
        .await
        .expect("working copy is repaired");
        let current_id = tx
            .repo()
            .view()
            .get_wc_commit_id(&workspace_name)
            .expect("working-copy commit exists");
        let current = load_commit_from_repo(tx.repo(), current_id).expect("load new wc commit");

        assert!(stats.repaired);
        assert_eq!(stats.rebased_descendants, 0);
        assert_ne!(current_id, trunk.id());
        assert!(current.parent_ids().contains(updated_trunk.id()));
    });
}

fn log_test_settings() -> Result<UserSettings, JjError> {
    let mut config = StackedConfig::with_defaults();
    config.extend_layers(default_config_layers());
    jj_lib::config::migrate(&mut config, &default_config_migrations()).map_err(log_error)?;
    UserSettings::from_config(config).map_err(|error| JjError::Settings {
        message: error.to_string(),
    })
}

async fn write_child(repo: &mut MutableRepo, parent: &Commit, description: &str) -> Commit {
    repo.new_commit(vec![parent.id().clone()], parent.tree())
        .set_description(description)
        .write()
        .await
        .expect("write child commit")
}

async fn write_child_with_files(
    repo: &mut MutableRepo,
    parent: &Commit,
    description: &str,
    files: &[(&str, &[u8])],
) -> Commit {
    let mut tree_builder = MergedTreeBuilder::new(parent.tree());
    for (path, contents) in files {
        let path = RepoPathBuf::from_internal_string(*path).expect("valid repo path");
        let id = repo
            .store()
            .write_file(&path, &mut &contents[..])
            .await
            .expect("write file contents");
        tree_builder.set_or_remove(
            path,
            Merge::normal(TreeValue::File {
                id,
                executable: false,
                copy_id: CopyId::placeholder(),
            }),
        );
    }
    let tree = tree_builder.write_tree().await.expect("write tree");

    repo.new_commit(vec![parent.id().clone()], tree)
        .set_description(description)
        .write()
        .await
        .expect("write child commit with files")
}

fn set_origin_bookmark(repo: &mut MutableRepo, branch: &str, commit_id: &CommitId) {
    set_remote_bookmark(repo, ORIGIN_REMOTE_NAME, branch, commit_id);
}

fn set_remote_bookmark(repo: &mut MutableRepo, remote: &str, branch: &str, commit_id: &CommitId) {
    set_remote_bookmark_with_state(repo, remote, branch, commit_id, RemoteRefState::Tracked);
}

fn set_untracked_origin_bookmark(repo: &mut MutableRepo, branch: &str, commit_id: &CommitId) {
    set_remote_bookmark_with_state(
        repo,
        ORIGIN_REMOTE_NAME,
        branch,
        commit_id,
        RemoteRefState::New,
    );
}

fn set_remote_bookmark_with_state(
    repo: &mut MutableRepo,
    remote: &str,
    branch: &str,
    commit_id: &CommitId,
    state: RemoteRefState,
) {
    repo.set_remote_bookmark(
        RefName::new(branch).to_remote_symbol(RemoteName::new(remote)),
        RemoteRef {
            target: RefTarget::normal(commit_id.clone()),
            state,
        },
    );
}

fn set_local_bookmark(repo: &mut MutableRepo, bookmark: &str, commit_id: &CommitId) {
    repo.set_local_bookmark_target(RefName::new(bookmark), RefTarget::normal(commit_id.clone()));
}

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "jx-jj-test-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create test workspace");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
