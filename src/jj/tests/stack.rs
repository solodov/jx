use super::*;

#[test]
fn stack_move_resolves_bookmark_fragment_after_revision_lookup() {
    // Verifies: `jx stack --onto` target resolution falls back from jj ids to bookmark fragments.
    let fixture = TestWorkspace::new("stack-move-bookmark-fragment");
    let settings = user_settings().expect("settings");
    let (workspace, repo, base_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let base = write_child(tx.repo_mut(), &trunk, "base change").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "topic/base-target", base.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx.commit("arrange stack move").await.expect("commit");
        (workspace, repo, base.id().clone())
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .move_current_stack(StackMoveTarget::Onto("base-target".to_owned()))
        .expect("stack move succeeds");

    let current = subject.current_commit().expect("current commit loads");
    assert_eq!(current.parent_ids(), vec![base_id]);
    assert_eq!(outcome.rebased_commits, 1);
    assert!(outcome.current_updated);
}

#[test]
fn local_stack_branches_reflect_nearest_bookmarked_parent() {
    // Verifies: Local stack metadata repair derives PR bases from jj ancestry, not GitHub state.
    let fixture = TestWorkspace::new("local-stack-branches");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let parent = write_child(tx.repo_mut(), &trunk, "parent change").await;
        let child = write_child(tx.repo_mut(), &parent, "child change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "topic/parent", parent.id());
        set_local_bookmark(tx.repo_mut(), "topic/child", child.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), child.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange local stack branches")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let branches = subject
        .local_stack_branches()
        .expect("local stack branches load");

    assert_eq!(
        branches,
        vec![
            LocalStackBranch {
                branch: "topic/child".to_owned(),
                base_branch: "topic/parent".to_owned(),
                parent_branch: Some("topic/parent".to_owned()),
                title: "child change".to_owned(),
            },
            LocalStackBranch {
                branch: "topic/parent".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                title: "parent change".to_owned(),
            },
        ]
    );
}

#[test]
fn stack_move_reports_ambiguous_bookmark_fragments() {
    // Verifies: Bookmark fragment fallback fails safely when the best fragment match is ambiguous.
    let fixture = TestWorkspace::new("stack-move-ambiguous-fragment");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let left = write_child(tx.repo_mut(), &trunk, "left base").await;
        let right = write_child(tx.repo_mut(), &trunk, "right base").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "topic/base-left", left.id());
        set_local_bookmark(tx.repo_mut(), "topic/base-right", right.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange ambiguous stack move")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let error = subject
        .move_current_stack(StackMoveTarget::Onto("base".to_owned()))
        .expect_err("ambiguous bookmark fragment is rejected");

    assert!(matches!(
        error,
        JjError::StackTargetAmbiguous { target, matches }
            if target == "base" && matches == vec!["topic/base-left", "topic/base-right"]
    ));
}
