use super::*;

#[test]
fn fork_sync_plan_identifies_rebase_stack_root() {
    // Verifies: fork sync plans the fork-only stack relative to the fetched upstream branch.
    let fixture = TestWorkspace::new("fork-sync-plan-rebase");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let base = write_child(tx.repo_mut(), &root, "old source").await;
        let upstream = write_child(tx.repo_mut(), &base, "new source").await;
        let fork_root = write_child(tx.repo_mut(), &base, "fork root").await;
        let fork_head = write_child(tx.repo_mut(), &fork_root, "fork head").await;

        set_local_bookmark(tx.repo_mut(), "main", fork_head.id());
        set_origin_bookmark(tx.repo_mut(), "main", fork_head.id());
        set_remote_bookmark(tx.repo_mut(), "upstream", "main", upstream.id());

        let repo = tx.commit("arrange fork rebase plan").await.expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let plan = subject
        .fork_sync_branch_plan("main", "upstream", "main")
        .expect("fork sync plan succeeds");

    assert_eq!(plan.branch, "main");
    assert!(!plan.push_needed);
    assert_eq!(plan.upstream_short_commit_id.len(), 8);
    assert!(matches!(
        plan.operation,
        ForkSyncBranchOperation::Rebase {
            commit_count: 2,
            ..
        }
    ));
}

#[test]
fn fork_sync_rebases_stack_and_moves_branch() {
    // Verifies: applying a fork sync rebase moves the local branch stack onto upstream.
    let fixture = TestWorkspace::new("fork-sync-apply-rebase");
    let settings = user_settings().expect("settings");
    let (workspace, repo, upstream_id, old_head_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let base = write_child(tx.repo_mut(), &root, "old source").await;
        let upstream = write_child(tx.repo_mut(), &base, "new source").await;
        let fork_root = write_child(tx.repo_mut(), &base, "fork root").await;
        let fork_head = write_child(tx.repo_mut(), &fork_root, "fork head").await;

        set_local_bookmark(tx.repo_mut(), "main", fork_head.id());
        set_origin_bookmark(tx.repo_mut(), "main", fork_head.id());
        set_remote_bookmark(tx.repo_mut(), "upstream", "main", upstream.id());
        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                fork_head.id().clone(),
            )
            .expect("set current working-copy change");

        let repo = tx.commit("arrange fork rebase").await.expect("commit");
        (
            workspace,
            repo,
            upstream.id().clone(),
            fork_head.id().clone(),
        )
    });
    let mut subject = JjWorkspace { workspace, repo };
    let plan = subject
        .fork_sync_branch_plan("main", "upstream", "main")
        .expect("fork sync plan succeeds");

    let outcome = subject
        .apply_fork_sync_branch_plan(&plan)
        .expect("fork sync rebase succeeds");

    let new_head_id = subject
        .repo
        .view()
        .get_local_bookmark(RefName::new("main"))
        .as_normal()
        .cloned()
        .expect("main bookmark remains");
    assert_ne!(new_head_id, old_head_id);
    let upstream = subject
        .load_commit(&upstream_id)
        .expect("upstream commit loads");
    let new_head = subject
        .load_commit(&new_head_id)
        .expect("new main commit loads");
    assert_eq!(
        subject
            .linear_stack_path(&upstream, &new_head)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(outcome.rebased_commits.len(), 2);
    assert!(matches!(
        outcome.operation,
        ForkSyncBranchOutcomeKind::Rebased {
            commit_count: 2,
            ..
        }
    ));
}

#[test]
fn fork_sync_fast_forwards_branch_and_rebases_local_children() {
    // Verifies: a fork branch that is only behind upstream fast-forwards and keeps local children on top.
    let fixture = TestWorkspace::new("fork-sync-fast-forward");
    let settings = user_settings().expect("settings");
    let (workspace, repo, upstream_id, current_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_main = write_child(tx.repo_mut(), &root, "old source").await;
        let upstream = write_child(tx.repo_mut(), &old_main, "new source").await;
        let current = write_child(tx.repo_mut(), &old_main, "local work").await;

        set_local_bookmark(tx.repo_mut(), "main", old_main.id());
        set_origin_bookmark(tx.repo_mut(), "main", old_main.id());
        set_remote_bookmark(tx.repo_mut(), "upstream", "main", upstream.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange fork fast-forward")
            .await
            .expect("commit");
        (workspace, repo, upstream.id().clone(), current.id().clone())
    });
    let mut subject = JjWorkspace { workspace, repo };
    let plan = subject
        .fork_sync_branch_plan("main", "upstream", "main")
        .expect("fork sync plan succeeds");
    assert_eq!(plan.operation, ForkSyncBranchOperation::FastForward);

    let outcome = subject
        .apply_fork_sync_branch_plan(&plan)
        .expect("fork sync fast-forward succeeds");

    let new_main_id = subject
        .repo
        .view()
        .get_local_bookmark(RefName::new("main"))
        .as_normal()
        .cloned()
        .expect("main bookmark remains");
    assert_eq!(new_main_id, upstream_id);
    let current = subject.current_commit().expect("current commit loads");
    assert_ne!(current.id(), &current_id);
    assert_eq!(current.parent_ids(), vec![upstream_id]);
    assert!(matches!(
        outcome.operation,
        ForkSyncBranchOutcomeKind::FastForward
    ));
    assert!(outcome.current_updated);
}
