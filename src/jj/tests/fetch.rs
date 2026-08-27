use super::*;

#[test]
fn parse_remote_default_branch_reads_head_symref() {
    // Verifies: Live remote HEAD parsing extracts only the advertised branch symref.
    let output = "ref: refs/heads/master\tHEAD\n094718641101af6b44bbe4a54ac156040fe6de2c\tHEAD\n";

    assert_eq!(
        parse_remote_default_branch(output).as_deref(),
        Some("master")
    );
}

#[test]
fn fetch_trunk_uses_live_default_branch_to_break_cached_main_master_ambiguity() {
    // Verifies: Networked fetch can use live remote HEAD when stale cached main/master refs conflict.
    let fixture = TestWorkspace::new("fetch-main-master-default");
    let settings = user_settings().expect("settings");
    let (workspace, repo, master_id) = pollster::block_on(async {
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

        let master_id = master.id().clone();
        let repo = tx
            .commit("arrange fetch main master default workspace")
            .await
            .expect("commit");
        (workspace, repo, master_id)
    });
    let subject = JjWorkspace { workspace, repo };
    let target = subject.current_commit().expect("current commit loads");

    let selection = subject
        .resolve_fetch_trunk_with_default_branch(&target, |_| Some("master".to_owned()))
        .expect("live default branch breaks fetch trunk tie");

    assert_eq!(selection.branch, "master");
    assert_eq!(selection.commit.id(), &master_id);
    assert_eq!(
        selection.refresh_bookmarks,
        ["main".to_owned(), "master".to_owned()]
    );
}

#[test]
fn fetch_trunk_uses_live_default_branch_after_sideways_trunk_rewrite() {
    // Verifies: Fetch can still select origin trunk after local main was rewritten sideways.
    let fixture = TestWorkspace::new("fetch-sideways-main-default");
    let settings = user_settings().expect("settings");
    let (workspace, repo, remote_main_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_base = write_child(tx.repo_mut(), &root, "old base").await;
        let remote_main = write_child(tx.repo_mut(), &old_base, "main before local rewrite").await;
        let new_base = write_child(tx.repo_mut(), &root, "new base").await;
        let local_main = tx
            .repo_mut()
            .new_commit(vec![new_base.id().clone()], new_base.tree())
            .set_change_id(remote_main.change_id().clone())
            .set_description("main after local rewrite")
            .write()
            .await
            .expect("write sideways local main rewrite");

        set_origin_bookmark(tx.repo_mut(), "main", remote_main.id());
        set_local_bookmark(tx.repo_mut(), "main", local_main.id());
        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                local_main.id().clone(),
            )
            .expect("set current working-copy change");

        let remote_main_id = remote_main.id().clone();
        let repo = tx
            .commit("arrange sideways main default workspace")
            .await
            .expect("commit");
        (workspace, repo, remote_main_id)
    });
    let subject = JjWorkspace { workspace, repo };
    let target = subject.current_commit().expect("current commit loads");

    let selection = subject
        .resolve_fetch_trunk_with_default_branch(&target, |_| Some("main".to_owned()))
        .expect("live default branch recovers cached sideways trunk");

    assert_eq!(selection.branch, "main");
    assert_eq!(selection.commit.id(), &remote_main_id);
    assert!(selection.refresh_bookmarks.is_empty());
}

#[test]
fn fetch_trunk_uses_origin_trunk_when_protecting_historical_stack() {
    // Verifies: protected sync fetches semantic trunk even when PR refs are the only ancestors.
    let fixture = TestWorkspace::new("fetch-protected-default-not-ancestor");
    let settings = user_settings().expect("settings");
    let (workspace, repo, main_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let stale_pr = write_child(tx.repo_mut(), &root, "stale landed PR").await;
        let historical_base = write_child(tx.repo_mut(), &stale_pr, "historical trunk base").await;
        let current = write_child(tx.repo_mut(), &historical_base, "protected PR head").await;
        let main = write_child(tx.repo_mut(), &historical_base, "current main trunk").await;

        set_origin_bookmark(tx.repo_mut(), "main", main.id());
        set_origin_bookmark(tx.repo_mut(), "topic/current", current.id());
        set_origin_bookmark(tx.repo_mut(), "topic/stale", stale_pr.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let main_id = main.id().clone();
        let repo = tx
            .commit("arrange protected fetch default non-ancestor workspace")
            .await
            .expect("commit");
        (workspace, repo, main_id)
    });
    let subject = JjWorkspace { workspace, repo };
    let target = subject.current_commit().expect("current commit loads");

    let selection = subject
        .resolve_fetch_trunk(
            &target,
            &FetchOptions {
                protected_rebase_roots: vec!["topic/current".to_owned()],
            },
        )
        .expect("protected fetch chooses semantic trunk");

    assert_eq!(selection.branch, "main");
    assert_eq!(selection.commit.id(), &main_id);
    assert!(selection.refresh_bookmarks.is_empty());
}

#[test]
fn fetch_rebase_uses_jj_rewrite_mapping_before_trunk_repair() {
    // Verifies: fetch lets jj apply remote rewrite mappings, then resolves trunk children by change id.
    let fixture = TestWorkspace::new("fetch-change-id-trunk-child");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_trunk = write_child(tx.repo_mut(), &root, "old main trunk").await;
        let local_child = write_child(tx.repo_mut(), &old_trunk, "local child").await;
        let updated_trunk = tx
            .repo_mut()
            .new_commit(vec![root.id().clone()], root.tree())
            .set_change_id(old_trunk.change_id().clone())
            .set_description("updated main trunk")
            .write()
            .await
            .expect("write updated trunk with preserved change id");
        let trunk_children = collect_trunk_child_changes(tx.repo(), old_trunk.id())
            .expect("collect trunk child changes");

        tx.repo_mut()
            .set_rewritten_commit(old_trunk.id().clone(), updated_trunk.id().clone());
        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &trunk_children,
            &updated_trunk,
            &RevsetExpression::none(),
            &BTreeMap::new(),
        )
        .await
        .expect("jj rewrite mapping is applied before trunk repair");

        assert_eq!(stats.rebased_descendants, 1);
        assert_eq!(stats.rebased_trunk_children, 0);
        assert_eq!(stats.skipped_trunk_children, 1);
        let visible = tx
            .repo()
            .resolve_change_id(local_child.change_id())
            .expect("change id resolves")
            .expect("change remains visible")
            .into_visible()
            .expect("visible child commit");
        assert_eq!(visible.len(), 1);
        let rebased_child = load_commit_from_repo(tx.repo(), &visible[0]).expect("load child");
        assert_eq!(rebased_child.parent_ids(), &[updated_trunk.id().clone()]);
    });
}

#[test]
fn fetch_rebase_skips_protected_trunk_child_subtree() {
    // Verifies: PR-aware sync can preserve a green trunk-child PR and its descendants.
    let fixture = TestWorkspace::new("fetch-protected-trunk-child");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_trunk = write_child(tx.repo_mut(), &root, "old main trunk").await;
        let protected_child = write_child(tx.repo_mut(), &old_trunk, "protected child").await;
        let protected_descendant =
            write_child(tx.repo_mut(), &protected_child, "protected descendant").await;
        let updated_trunk = write_child(tx.repo_mut(), &root, "updated main trunk").await;
        let trunk_children = collect_trunk_child_changes(tx.repo(), old_trunk.id())
            .expect("collect trunk child changes");
        let protected_rebase_roots =
            BTreeMap::from([(protected_child.change_id().clone(), "topic/root".to_owned())]);

        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &trunk_children,
            &updated_trunk,
            &RevsetExpression::none(),
            &protected_rebase_roots,
        )
        .await
        .expect("protected child is skipped");

        assert_eq!(stats.rebased_trunk_children, 0);
        assert_eq!(stats.rebased_descendants, 0);
        assert_eq!(stats.skipped_trunk_children, 1);
        let visible_child = tx
            .repo()
            .resolve_change_id(protected_child.change_id())
            .expect("change id resolves")
            .expect("protected child remains visible")
            .into_visible()
            .expect("visible child commit");
        let visible_descendant = tx
            .repo()
            .resolve_change_id(protected_descendant.change_id())
            .expect("change id resolves")
            .expect("protected descendant remains visible")
            .into_visible()
            .expect("visible descendant commit");
        let child = load_commit_from_repo(tx.repo(), &visible_child[0]).expect("load child");
        let descendant =
            load_commit_from_repo(tx.repo(), &visible_descendant[0]).expect("load descendant");
        assert_eq!(child.parent_ids(), &[old_trunk.id().clone()]);
        assert_eq!(descendant.parent_ids(), &[child.id().clone()]);
    });
}

#[test]
fn fetch_rebase_moves_descendants_of_landed_trunk_child_to_updated_trunk() {
    // Verifies: once a previously protected root lands, local children move onto current trunk.
    let fixture = TestWorkspace::new("fetch-landed-trunk-child-descendants");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_trunk = write_child(tx.repo_mut(), &root, "old hypothetical trunk").await;
        let landed_child = write_child(tx.repo_mut(), &old_trunk, "landed hypothetical root").await;
        let empty_default = write_child(tx.repo_mut(), &landed_child, "").await;
        let follow_up = write_child(tx.repo_mut(), &empty_default, "hypothetical follow-up").await;
        let updated_trunk =
            write_child(tx.repo_mut(), &landed_child, "updated hypothetical trunk").await;
        let trunk_children = collect_trunk_child_changes(tx.repo(), old_trunk.id())
            .expect("collect trunk child changes");
        let protected_rebase_roots = BTreeMap::from([(
            landed_child.change_id().clone(),
            "hypothetical/root".to_owned(),
        )]);
        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                empty_default.id().clone(),
            )
            .expect("set working-copy commit");

        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &trunk_children,
            &updated_trunk,
            &RevsetExpression::none(),
            &protected_rebase_roots,
        )
        .await
        .expect("landed descendants rebase");

        assert_eq!(stats.skipped_trunk_children, 1);
        assert_eq!(stats.rebased_trunk_children, 1);
        assert_eq!(stats.rebased_descendants, 1);
        let visible_default = tx
            .repo()
            .resolve_change_id(empty_default.change_id())
            .expect("default change id resolves")
            .expect("default remains visible")
            .into_visible()
            .expect("visible default commit");
        let visible_follow_up = tx
            .repo()
            .resolve_change_id(follow_up.change_id())
            .expect("follow-up change id resolves")
            .expect("follow-up remains visible")
            .into_visible()
            .expect("visible follow-up commit");
        let default = load_commit_from_repo(tx.repo(), &visible_default[0]).expect("load default");
        let follow_up =
            load_commit_from_repo(tx.repo(), &visible_follow_up[0]).expect("load follow-up");
        assert_eq!(default.parent_ids(), &[updated_trunk.id().clone()]);
        assert_eq!(follow_up.parent_ids(), &[default.id().clone()]);
        assert_eq!(
            tx.repo()
                .view()
                .get_wc_commit_id(workspace.workspace_name()),
            Some(default.id())
        );
    });
}

#[test]
fn fetch_repair_moves_abandoned_other_workspace_to_updated_trunk() {
    // Verifies: if Git import abandons another workspace's landed head, fetch reparents
    // jj's empty replacement workspace commit from old trunk to the updated trunk.
    let fixture = TestWorkspace::new("fetch-repair-abandoned-other-workspace");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_trunk = write_child(tx.repo_mut(), &root, "old main trunk").await;
        let runner_current = write_child(tx.repo_mut(), &old_trunk, "running workspace head").await;
        let abandoned_current = write_child_with_files(
            tx.repo_mut(),
            &old_trunk,
            "landed other workspace head",
            &[("feature.txt", b"landed")],
        )
        .await;
        let updated_trunk = write_child(tx.repo_mut(), &old_trunk, "updated main trunk").await;
        let other_workspace = WorkspaceNameBuf::from("other-workspace");

        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                runner_current.id().clone(),
            )
            .expect("set running working-copy commit");
        tx.repo_mut()
            .set_wc_commit(other_workspace.clone(), abandoned_current.id().clone())
            .expect("set other working-copy commit");
        let workspaces_before = collect_workspace_current_commits(tx.repo());

        tx.repo_mut().record_abandoned_commit(&abandoned_current);
        tx.repo_mut()
            .rebase_descendants_with_options(
                &RevsetExpression::none(),
                &RebaseOptions::default(),
                |_, _| {},
            )
            .await
            .expect("import rewrite creates replacement workspace commit");
        let replacement_id = tx
            .repo()
            .view()
            .get_wc_commit_id(&other_workspace)
            .expect("other workspace has replacement commit")
            .clone();
        let replacement =
            load_commit_from_repo(tx.repo(), &replacement_id).expect("load replacement");
        assert_eq!(replacement.parent_ids(), &[old_trunk.id().clone()]);
        assert!(replacement
            .is_discardable(tx.repo())
            .await
            .expect("replacement is discardable"));

        let stats = repair_fetch_working_copies(
            tx.repo_mut(),
            &workspaces_before,
            workspace.workspace_name(),
            old_trunk.id(),
            &updated_trunk,
            &HashSet::from([abandoned_current.id().clone()]),
        )
        .await
        .expect("repair abandoned workspace replacement");

        assert_eq!(stats.repaired_workspaces, 1);
        assert!(!stats.current_repaired);
        let repaired_id = tx
            .repo()
            .view()
            .get_wc_commit_id(&other_workspace)
            .expect("other workspace remains present");
        let repaired = load_commit_from_repo(tx.repo(), repaired_id).expect("load repaired");
        assert_eq!(repaired.parent_ids(), &[updated_trunk.id().clone()]);
        tx.commit("repair abandoned workspace replacement")
            .await
            .expect("repair transaction commits cleanly");
    });
}

#[test]
fn fetch_repair_moves_abandoned_current_workspace_to_updated_trunk() {
    // Verifies: when the running workspace's landed head is abandoned by Git import,
    // fetch repairs jj's empty replacement and reports the current workspace changed.
    let fixture = TestWorkspace::new("fetch-repair-abandoned-current-workspace");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_trunk = write_child(tx.repo_mut(), &root, "old main trunk").await;
        let abandoned_current = write_child_with_files(
            tx.repo_mut(),
            &old_trunk,
            "landed current workspace head",
            &[("feature.txt", b"landed")],
        )
        .await;
        let updated_trunk = write_child(tx.repo_mut(), &old_trunk, "updated main trunk").await;

        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                abandoned_current.id().clone(),
            )
            .expect("set current working-copy commit");
        let workspaces_before = collect_workspace_current_commits(tx.repo());

        tx.repo_mut().record_abandoned_commit(&abandoned_current);
        tx.repo_mut()
            .rebase_descendants_with_options(
                &RevsetExpression::none(),
                &RebaseOptions::default(),
                |_, _| {},
            )
            .await
            .expect("import rewrite creates replacement workspace commit");

        let stats = repair_fetch_working_copies(
            tx.repo_mut(),
            &workspaces_before,
            workspace.workspace_name(),
            old_trunk.id(),
            &updated_trunk,
            &HashSet::from([abandoned_current.id().clone()]),
        )
        .await
        .expect("repair abandoned current workspace replacement");

        assert_eq!(stats.repaired_workspaces, 1);
        assert!(stats.current_repaired);
        let repaired_id = tx
            .repo()
            .view()
            .get_wc_commit_id(workspace.workspace_name())
            .expect("current workspace remains present");
        let repaired = load_commit_from_repo(tx.repo(), repaired_id).expect("load repaired");
        assert_eq!(repaired.parent_ids(), &[updated_trunk.id().clone()]);
        tx.commit("repair abandoned current workspace replacement")
            .await
            .expect("repair transaction commits cleanly");
    });
}

#[test]
fn fetch_origin_refs_selects_trunk_tracked_and_refresh_bookmarks_only() {
    // Verifies: Fetch avoids enumerating every remote bookmark while still pruning stale candidates.
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
            tracked_origin_bookmarks(tx.repo_mut(), "main", &["stale-main".to_owned()]),
            vec![
                "example-user/pr".to_owned(),
                "main".to_owned(),
                "stale-main".to_owned(),
            ]
        );
    });
}
