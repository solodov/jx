use super::*;

#[test]
fn previous_commit_moves_and_renders_surrounding_chain() {
    // Verifies: Previous commit navigation shows the moved-to commit with its direct line context.
    let fixture = TestWorkspace::new("previous-navigation");
    let settings = user_settings().expect("settings");
    let (workspace, repo, middle) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let base = write_child(tx.repo_mut(), &root, "shared base").await;
        let bottom = write_child(tx.repo_mut(), &base, "bottom change").await;
        let middle = write_child(tx.repo_mut(), &bottom, "middle change").await;
        let top = write_child(tx.repo_mut(), &middle, "top change").await;

        set_origin_bookmark(tx.repo_mut(), "main", base.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), top.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange previous navigation")
            .await
            .expect("commit");
        (workspace, repo, middle)
    });
    let mut subject = JjWorkspace { workspace, repo };

    subject
        .move_to_previous_commit()
        .expect("move to previous commit");
    let log = subject
        .render_navigation_log(fixture.path())
        .expect("navigation log renders");

    assert_eq!(
        subject.current_commit().expect("current commit").id(),
        middle.id()
    );
    assert!(log.contains("top change"), "{log}");
    assert!(log.contains("middle change"), "{log}");
    assert!(log.contains("bottom change"), "{log}");
    assert!(log.contains("shared base"), "{log}");
    assert!(!log.contains("Working copy"), "{log}");
}

#[test]
fn next_commit_moves_to_single_child() {
    // Verifies: Next commit navigation moves along the single direct child chain.
    let fixture = TestWorkspace::new("next-navigation");
    let settings = user_settings().expect("settings");
    let (workspace, repo, middle) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let base = write_child(tx.repo_mut(), &root, "shared base").await;
        let bottom = write_child(tx.repo_mut(), &base, "bottom change").await;
        let middle = write_child(tx.repo_mut(), &bottom, "middle change").await;
        let _top = write_child(tx.repo_mut(), &middle, "top change").await;

        set_origin_bookmark(tx.repo_mut(), "main", base.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), bottom.id().clone())
            .expect("set current working-copy change");

        let repo = tx.commit("arrange next navigation").await.expect("commit");
        (workspace, repo, middle)
    });
    let mut subject = JjWorkspace { workspace, repo };

    subject.move_to_next_commit().expect("move to next commit");
    let log = subject
        .render_navigation_log(fixture.path())
        .expect("navigation log renders");

    assert_eq!(
        subject.current_commit().expect("current commit").id(),
        middle.id()
    );
    assert!(log.contains("top change"), "{log}");
    assert!(log.contains("middle change"), "{log}");
    assert!(log.contains("bottom change"), "{log}");
    assert!(log.contains("shared base"), "{log}");
}

#[test]
fn previous_commit_rejects_immutable_parent() {
    // Verifies: Previous navigation stops at the editable chain boundary instead of entering trunk.
    let fixture = TestWorkspace::new("previous-navigation-immutable");
    let settings = user_settings().expect("settings");
    let (workspace, repo, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let base = write_child(tx.repo_mut(), &root, "shared base").await;
        let current = write_child(tx.repo_mut(), &base, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", base.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange immutable previous navigation")
            .await
            .expect("commit");
        (workspace, repo, current)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let error = subject
        .move_to_previous_commit()
        .expect_err("immutable parent is not editable");

    assert!(matches!(error, JjError::NoPreviousCommit));
    assert_eq!(
        subject.current_commit().expect("current commit").id(),
        current.id()
    );
}

#[test]
fn next_commit_rejects_branching_children() {
    // Verifies: Next commit navigation avoids guessing when the current commit branches.
    let fixture = TestWorkspace::new("next-navigation-branching");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let base = write_child(tx.repo_mut(), &root, "shared base").await;
        let current = write_child(tx.repo_mut(), &base, "current change").await;
        let _left = write_child(tx.repo_mut(), &current, "left child").await;
        let _right = write_child(tx.repo_mut(), &current, "right child").await;

        set_origin_bookmark(tx.repo_mut(), "main", base.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange branching next navigation")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let error = subject
        .move_to_next_commit()
        .expect_err("branching next commits are ambiguous");

    assert!(matches!(error, JjError::AmbiguousNextCommit { count: 2 }));
}
