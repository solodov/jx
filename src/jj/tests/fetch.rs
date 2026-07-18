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
