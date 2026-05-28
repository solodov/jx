use super::*;

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
fn sync_bookmark_selection_requires_one_bookmark_unless_named() {
    // Verifies: Single-target sync requires an unambiguous bookmark on the selected commit.
    let fixture = TestWorkspace::new("sync-bookmark-selection");
    let settings = user_settings().expect("settings");
    let (workspace, repo, ambiguous_commit_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let unbookmarked = write_child(tx.repo_mut(), &trunk, "unbookmarked change").await;
        let ambiguous = write_child(tx.repo_mut(), &trunk, "ambiguous change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "example-user/one", ambiguous.id());
        set_local_bookmark(tx.repo_mut(), "example-user/two", ambiguous.id());
        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                unbookmarked.id().clone(),
            )
            .expect("set working-copy commit");

        let repo = tx
            .commit("arrange sync bookmark selection")
            .await
            .expect("commit");
        (workspace, repo, ambiguous.id().hex())
    });
    let subject = JjWorkspace { workspace, repo };

    let missing = subject
        .sync_bookmark_selection_for_revision(None)
        .expect_err("unbookmarked current change is rejected");
    let ambiguous = subject
        .sync_bookmark_selection_for_revision(Some(&ambiguous_commit_id))
        .expect_err("ambiguous selected change is rejected");
    let named = subject
        .sync_bookmark_selection_for_revision(Some("example-user/two"))
        .expect("exact bookmark selects one branch");

    assert!(matches!(missing, JjError::MissingSyncBookmark));
    assert!(matches!(ambiguous, JjError::AmbiguousSyncBookmark { .. }));
    assert_eq!(named.branch, "example-user/two");
}
