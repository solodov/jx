use super::*;

#[test]
fn pr_action_revision_requires_exact_local_head_and_does_not_follow_rewritten_changes() {
    let fixture = TestWorkspace::new("pr-action-revision");
    let settings = user_settings().unwrap();
    let (workspace, repo, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .unwrap();
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child(tx.repo_mut(), &root, "PR head").await;
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .unwrap();
        let repo = tx.commit("arrange action head").await.unwrap();
        (workspace, repo, current)
    });
    let mut subject = JjWorkspace { workspace, repo };
    for invalid in [
        "@",
        "main",
        "abc",
        "0000000000000000000000000000000000000001",
        "x000000000000000000000000000000000000000",
    ] {
        assert!(subject.pr_action_revision(invalid).is_none());
    }
    let resolved = subject.pr_action_revision(&current.id().hex()).unwrap();
    assert_eq!(resolved.commit_id, current.id().hex());
    assert_eq!(resolved.change_id, Some(current.change_id().reverse_hex()));
    let rewritten = subject
        .rewrite_commit_description(&current.id().hex(), "changed locally")
        .unwrap();
    let old = subject.pr_action_revision(&current.id().hex()).unwrap();
    assert_eq!(old.commit_id, current.id().hex());
    assert_eq!(old.change_id, None);
    let new = subject.pr_action_revision(&rewritten.commit_id).unwrap();
    assert_eq!(new.commit_id, rewritten.commit_id);
    assert_eq!(new.change_id, Some(current.change_id().reverse_hex()));
}
