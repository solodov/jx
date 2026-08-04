use super::*;

#[test]
fn advance_trunk_skips_current_change_that_is_not_based_on_current_trunk() {
    // Verifies: sync trunk advancement does not choose PR refs as trunk for protected historical stacks.
    let fixture = TestWorkspace::new("advance-trunk-historical-pr");
    let settings = user_settings().expect("settings");
    let (workspace, repo, main_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let stale_pr = write_child_with_files(
            tx.repo_mut(),
            &root,
            "stale landed PR",
            &[("stale.txt", b"stale")],
        )
        .await;
        let historical_base = write_child_with_files(
            tx.repo_mut(),
            &stale_pr,
            "historical trunk base",
            &[("base.txt", b"base")],
        )
        .await;
        let current = write_child_with_files(
            tx.repo_mut(),
            &historical_base,
            "protected PR head",
            &[("current.txt", b"current")],
        )
        .await;
        let main = write_child_with_files(
            tx.repo_mut(),
            &historical_base,
            "current main trunk",
            &[("main.txt", b"main")],
        )
        .await;

        set_origin_bookmark(tx.repo_mut(), "main", main.id());
        set_local_bookmark(tx.repo_mut(), "main", main.id());
        set_origin_bookmark(tx.repo_mut(), "topic/current", current.id());
        set_origin_bookmark(tx.repo_mut(), "topic/stale", stale_pr.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let main_id = main.id().clone();
        let repo = tx
            .commit("arrange historical PR trunk advance workspace")
            .await
            .expect("commit");
        (workspace, repo, main_id)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .advance_trunk_for_sync()
        .expect("off-trunk protected PR does not block sync");

    assert_eq!(outcome.branch, "main");
    assert_eq!(outcome.old_short_commit_id, short_commit_id(&main_id));
    assert_eq!(outcome.new_short_commit_id, short_commit_id(&main_id));
    assert_eq!(outcome.trunk, None);
    assert!(!outcome.current_updated);
}
