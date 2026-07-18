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

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");

    assert!(log.contains("current workspace change"), "{log}");
    assert!(log.contains("main trunk"), "{log}");
    assert!(!log.contains("other workspace change"), "{log}");
}

#[test]
fn workspace_log_omits_commit_ids_from_jx_default_header() {
    // Verifies: jx's default log header prioritizes the operator-facing change id.
    let fixture = TestWorkspace::new("workspace-log-compact-header");
    let settings = log_test_settings().expect("settings");
    let (workspace, repo, current_change_id, current_commit_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child(tx.repo_mut(), &root, "current workspace change").await;
        let current_change_id = short_change_id(&current);
        let current_commit_id = short_commit_id(current.id());

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx.commit("arrange compact log").await.expect("commit");
        (workspace, repo, current_change_id, current_commit_id)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");

    assert!(log.contains(&current_change_id), "{log}");
    assert!(!log.contains(&current_commit_id), "{log}");
}

#[test]
fn workspace_log_links_pull_request_annotations_for_matching_bookmarks() {
    // Verifies: log annotations attach PR links to commits via local bookmark targets.
    let fixture = TestWorkspace::new("workspace-log-pr-links");
    let settings = log_test_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child(tx.repo_mut(), &root, "current workspace change").await;

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        set_local_bookmark(tx.repo_mut(), "topic/current", current.id());

        let repo = tx.commit("arrange annotated log").await.expect("commit");
        (workspace, repo)
    });
    let annotations = [LogBookmarkAnnotation {
        bookmark: "topic/current".to_owned(),
        label: "#42".to_owned(),
        url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
    }];

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &annotations)
        .expect("log renders");

    assert!(log.contains("topic/current"), "{log}");
    assert!(
        log.contains(
            "\x1b]8;;https://github.com/example-owner/example-repo/pull/42\x1b\\#42\x1b]8;;\x1b\\"
        ),
        "{log}"
    );
}
