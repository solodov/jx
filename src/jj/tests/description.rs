use super::*;

#[test]
fn rewrite_commit_description_updates_selected_commit_and_descendants() {
    // Verifies: Description rewrites return the replacement commit id for PR planning.
    let fixture = TestWorkspace::new("rewrite-description");
    let settings = user_settings().expect("settings");
    let (workspace, repo, current) = pollster::block_on(async {
        let (workspace, repo) =
            Workspace::init_internal_git(&settings, fixture.path(), gix::hash::Kind::Sha1)
                .await
                .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child(tx.repo_mut(), &root, "old title").await;
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        let repo = tx.commit("arrange described commit").await.expect("commit");
        (workspace, repo, current)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let rewrite = subject
        .rewrite_commit_description(&current.id().hex(), "new title")
        .expect("description rewrites");
    let rewritten = subject
        .load_commit(&CommitId::try_from_hex(&rewrite.commit_id).expect("valid commit id"))
        .expect("rewritten commit loads");

    assert!(rewrite.changed);
    assert_ne!(rewrite.commit_id, current.id().hex());
    assert_eq!(rewritten.description(), "new title");
}

#[test]
fn a_cached_workspace_cannot_rewrite_a_superseded_commit() {
    let fixture = TestWorkspace::new("rewrite-description-stale");
    let settings = user_settings().expect("settings");
    pollster::block_on(Workspace::init_internal_git(
        &settings,
        fixture.path(),
        gix::hash::Kind::Sha1,
    ))
    .expect("initialize jj workspace");
    let mut cached = JjWorkspace::load(fixture.path()).unwrap();
    let original = cached.current_commit().unwrap().id().hex();
    let mut other = JjWorkspace::load(fixture.path()).unwrap();
    let updated = other
        .rewrite_commit_description(&original, "fresh description")
        .unwrap();
    let error = cached
        .rewrite_commit_description(&original, "stale description")
        .unwrap_err();
    assert!(matches!(error, JjError::StaleChange { actual, .. } if actual == updated.commit_id));
    assert_eq!(
        cached.current_commit().unwrap().description(),
        "fresh description"
    );
}
