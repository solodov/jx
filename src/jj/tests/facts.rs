use super::*;

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
    assert_eq!(
        facts.target_change.change_id,
        current.change_id().reverse_hex()
    );
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
fn facts_exclude_trunk_bookmarks_from_nearest_stack_ancestor() {
    // Verifies: Root PR base selection uses trunk, not another local bookmark on trunk.
    let fixture = TestWorkspace::new("trunk-bookmarks-not-stack-base");
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
        set_local_bookmark(tx.repo_mut(), "green", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange trunk bookmark workspace")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let facts = subject.facts().expect("workspace facts load");

    assert_eq!(facts.trunk.branch, "main");
    assert_eq!(facts.nearest_ancestor_bookmark, None);
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
fn status_facts_prefers_trunk_branch_without_scanning_all_remote_bookmarks() {
    // Verifies: remote-status and stack-status avoid broad ancestry scans when origin/main is usable.
    let fixture = TestWorkspace::new("status-trunk-fast-path");
    let settings = user_settings().expect("settings");
    let (workspace, repo, trunk) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        for index in 0..25 {
            set_origin_bookmark(tx.repo_mut(), &format!("feature-{index}"), current.id());
        }
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange status fast-path workspace")
            .await
            .expect("commit");
        (workspace, repo, trunk)
    });
    let subject = JjWorkspace { workspace, repo };

    let facts = subject
        .status_facts_with_metrics(["origin"])
        .expect("status facts load");

    assert_eq!(facts.facts.remotes[0].branch, "main");
    assert_eq!(
        facts.facts.remotes[0].trunk_git_commit_sha,
        trunk.id().hex()
    );
    let trunk_metrics = &facts.metrics.remotes[0].trunk;
    assert!(trunk_metrics.fast_path);
    assert_eq!(trunk_metrics.remote_bookmark_count, 1);
    assert_eq!(trunk_metrics.ancestor_check_count, 1);
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
fn facts_keep_main_master_trunk_ambiguity_for_read_only_checks() {
    // Verifies: Read-only facts still reject equally preferred main/master cached trunk candidates.
    let fixture = TestWorkspace::new("main-master-ambiguous-trunk");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let main = write_child(tx.repo_mut(), &root, "main trunk").await;
        let master = write_child(tx.repo_mut(), &main, "master trunk").await;
        let current = write_child(tx.repo_mut(), &master, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", main.id());
        set_origin_bookmark(tx.repo_mut(), "master", master.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange main master ambiguous trunk workspace")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let error = subject.facts().expect_err("trunk is ambiguous");

    assert!(matches!(
        error,
        JjError::AmbiguousTrunk { branches, .. }
            if branches == ["main".to_owned(), "master".to_owned()]
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
