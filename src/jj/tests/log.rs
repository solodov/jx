use super::*;

#[test]
fn short_commit_ids_are_eight_hex_characters() {
    // Verifies: Short commit IDs are eight hex characters.
    let commit_id = CommitId::from_hex("0123456789abcdef");

    assert_eq!(short_commit_id(&commit_id), "01234567");
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
