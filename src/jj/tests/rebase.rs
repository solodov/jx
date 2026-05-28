use super::*;

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
