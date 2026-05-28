use super::*;

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
fn pull_request_bookmarks_include_all_local_bookmark_heads() {
    // Verifies: Stack tracking sees every normal local bookmark, not just the current chain.
    let fixture = TestWorkspace::new("pr-bookmarks-all-local");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;
        let sibling = write_child(tx.repo_mut(), &trunk, "sibling change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "review/current", current.id());
        set_local_bookmark(tx.repo_mut(), "review/sibling", sibling.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange local PR bookmarks")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let bookmarks = subject
        .pull_request_bookmarks()
        .expect("pull request bookmarks load");

    assert_eq!(bookmarks, ["review/current", "review/sibling"]);
}

#[test]
fn pull_request_candidates_resolve_explicit_commit_prefix() {
    // Verifies: Open PR selectors keep jj's single-commit prefix behavior before bookmark fallback.
    let fixture = TestWorkspace::new("pr-candidate-commit-prefix");
    let settings = user_settings().expect("settings");
    let (workspace, repo, selected_prefix) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let selected = write_child(tx.repo_mut(), &trunk, "selected change").await;
        let descendant = write_child(tx.repo_mut(), &selected, "review head").await;
        let selected_prefix = selected.id().hex()[..8].to_owned();

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "review/descendant", descendant.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), trunk.id().clone())
            .expect("set current working-copy change away from selected commit");

        let repo = tx
            .commit("arrange explicit commit prefix")
            .await
            .expect("commit");
        (workspace, repo, selected_prefix)
    });
    let subject = JjWorkspace { workspace, repo };

    let candidates = subject
        .pull_request_candidate_bookmarks(Some(&selected_prefix))
        .expect("candidate bookmarks load");

    assert_eq!(candidates, ["review/descendant"]);
}

#[test]
fn pull_request_candidates_fall_back_to_exact_local_bookmark() {
    // Verifies: Slash bookmark names can be used directly even when they are not jj revsets.
    let fixture = TestWorkspace::new("pr-candidate-bookmark-selector");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child(tx.repo_mut(), &root, "current change").await;
        let selected = write_child(tx.repo_mut(), &root, "review head").await;

        set_local_bookmark(tx.repo_mut(), "example-user/00-1977d9cd", selected.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change away from bookmark");

        let repo = tx
            .commit("arrange slash bookmark selector")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let candidates = subject
        .pull_request_candidate_bookmarks(Some("example-user/00-1977d9cd"))
        .expect("candidate bookmarks load");

    assert_eq!(candidates, ["example-user/00-1977d9cd"]);
}

#[test]
fn pull_request_candidates_keep_ambiguous_revision_errors() {
    // Verifies: Ambiguous commit selectors do not fall through to bookmark lookup.
    let fixture = TestWorkspace::new("pr-candidate-ambiguous-selector");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let first = write_child(tx.repo_mut(), &root, "first change").await;
        let second = write_child(tx.repo_mut(), &root, "second change").await;

        set_local_bookmark(tx.repo_mut(), "ambiguous/bookmark", first.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), second.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange ambiguous selector")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let error = subject
        .pull_request_candidate_bookmarks(Some("all()"))
        .expect_err("ambiguous revisions stay ambiguous");

    assert!(matches!(
        error,
        JjError::AmbiguousRevision { revision } if revision == "all()"
    ));
}
