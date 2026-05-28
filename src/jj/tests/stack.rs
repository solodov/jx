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
fn stack_trunk_moves_current_to_latest_origin_trunk() {
    // Verifies: `jx sk --trunk` matches the old trunk-rebase behavior by targeting latest origin trunk.
    let fixture = TestWorkspace::new("stack-trunk-latest-origin");
    let settings = user_settings().expect("settings");
    let (workspace, repo, updated_trunk_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_trunk = write_child(tx.repo_mut(), &root, "old trunk").await;
        let updated_trunk = write_child(tx.repo_mut(), &old_trunk, "updated trunk").await;
        let current = write_child(tx.repo_mut(), &old_trunk, "current change").await;

        set_origin_bookmark(tx.repo_mut(), "main", updated_trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx.commit("arrange stack trunk move").await.expect("commit");
        (workspace, repo, updated_trunk.id().clone())
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .move_current_stack(StackMoveTarget::Trunk)
        .expect("current change is moved onto updated trunk");

    let current = subject.current_commit().expect("current commit loads");
    assert_eq!(current.parent_ids(), vec![updated_trunk_id]);
    assert_eq!(outcome.rebased_commits, 1);
    assert!(outcome.current_updated);
}

#[test]
fn stack_trunk_accepts_current_that_already_landed_on_trunk() {
    // Verifies: moving onto trunk is a no-op when the current change is already in trunk history.
    let fixture = TestWorkspace::new("stack-trunk-current-landed");
    let settings = user_settings().expect("settings");
    let (workspace, repo, updated_trunk_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child(tx.repo_mut(), &root, "current change").await;
        let updated_trunk = write_child(tx.repo_mut(), &current, "updated trunk").await;

        set_origin_bookmark(tx.repo_mut(), "main", updated_trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange landed stack trunk move")
            .await
            .expect("commit");
        (workspace, repo, updated_trunk.id().clone())
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .move_current_stack(StackMoveTarget::Trunk)
        .expect("landed current is accepted as up to date");

    let current = subject.current_commit().expect("current commit loads");
    assert!(subject
        .is_ancestor_or_equal(current.id(), &updated_trunk_id)
        .expect("ancestor query succeeds"));
    assert_eq!(outcome.rebased_commits, 0);
    assert_eq!(outcome.skipped_commits, 1);
    assert!(!outcome.current_updated);
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

    let facts = subject
        .local_stack_branch_facts()
        .expect("local stack branch facts load");

    assert_eq!(
        facts.branches,
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
    assert_eq!(facts.metrics.branch_count, 2);
    assert_eq!(facts.metrics.local_bookmark_count, 2);
    assert_eq!(facts.metrics.normal_bookmark_count, 2);
    assert_eq!(facts.metrics.resolved_trunk_count, 1);
    assert_eq!(facts.metrics.stack_path_count, 2);
}

#[test]
fn stack_plan_facts_include_branching_neighbourhood() {
    // Verifies: stack plan allows sibling branches under a shared stack root.
    let fixture = TestWorkspace::new("stack-plan-neighbourhood");
    let settings = user_settings().expect("settings");
    let (workspace, repo, left_id, right_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let stack_root = write_child(tx.repo_mut(), &trunk, "root change").await;
        let left = write_child(tx.repo_mut(), &stack_root, "left change").await;
        let right = write_child(tx.repo_mut(), &stack_root, "right change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), left.id().clone())
            .expect("set current working-copy change");

        let left_id = left.id().hex();
        let right_id = right.id().hex();
        let repo = tx
            .commit("arrange stack plan neighbourhood")
            .await
            .expect("commit");
        (workspace, repo, left_id, right_id)
    });
    let subject = JjWorkspace { workspace, repo };

    let facts = subject
        .stack_plan_facts(&StackPlanSelection::ExplicitRevisions {
            revisions: vec![format!("{left_id} | {right_id}")],
        })
        .expect("stack plan facts load");

    assert_eq!(facts.selected_indexes, vec![1, 2]);
    assert_eq!(facts.nodes[0].parent_index, None);
    assert_eq!(facts.nodes[1].parent_index, Some(0));
    assert_eq!(facts.nodes[2].parent_index, Some(0));
    assert_eq!(
        facts.nodes[0].workspace.target_change.description,
        "root change"
    );
    assert_eq!(
        facts.nodes[1].workspace.target_change.description,
        "left change"
    );
    assert_eq!(
        facts.nodes[2].workspace.target_change.description,
        "right change"
    );
}

#[test]
fn stack_plan_facts_reject_multiple_selected_roots() {
    // Verifies: explicit stack plans must still target one common root.
    let fixture = TestWorkspace::new("stack-plan-multiple-roots");
    let settings = user_settings().expect("settings");
    let (workspace, repo, left_id, right_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let left = write_child(tx.repo_mut(), &trunk, "left change").await;
        let right = write_child(tx.repo_mut(), &trunk, "right change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), left.id().clone())
            .expect("set current working-copy change");

        let left_id = left.id().hex();
        let right_id = right.id().hex();
        let repo = tx
            .commit("arrange stack plan multiple roots")
            .await
            .expect("commit");
        (workspace, repo, left_id, right_id)
    });
    let subject = JjWorkspace { workspace, repo };

    let error = subject
        .stack_plan_facts(&StackPlanSelection::ExplicitRevisions {
            revisions: vec![left_id, right_id],
        })
        .expect_err("multi-root plan is rejected");

    assert!(matches!(error, JjError::StackPublishMultipleStacks));
}

#[test]
fn stack_publish_facts_infer_full_linear_stack_around_current() {
    // Verifies: stack publish without -r expands ancestors and descendants around the working copy.
    let fixture = TestWorkspace::new("stack-publish-inferred");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let parent = write_child(tx.repo_mut(), &trunk, "parent change").await;
        let current = write_child(tx.repo_mut(), &parent, "current change").await;
        let _child = write_child(tx.repo_mut(), &current, "child change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange inferred stack publish")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };

    let facts = subject
        .stack_publish_facts(&StackPublishSelection::InferredStack { anchor: None })
        .expect("stack publish facts load");

    assert_eq!(facts.publish_indexes, vec![0, 1, 2]);
    assert_eq!(facts.anchor_index, Some(1));
    assert_eq!(facts.nodes[0].parent_index, None);
    assert_eq!(facts.nodes[1].parent_index, Some(0));
    assert_eq!(facts.nodes[2].parent_index, Some(1));
    assert_eq!(
        facts.nodes[0].workspace.target_change.description,
        "parent change"
    );
    assert_eq!(
        facts.nodes[1].workspace.target_change.description,
        "current change"
    );
    assert_eq!(
        facts.nodes[2].workspace.target_change.description,
        "child change"
    );
    assert_eq!(facts.metrics.target_resolution_count, 1);
    assert_eq!(facts.metrics.resolved_trunk_count, 1);
    assert_eq!(facts.metrics.stack_path_count, 1);
    assert_eq!(facts.metrics.collected_child_count, 1);
    assert_eq!(facts.metrics.loaded_child_count, 1);
    assert_eq!(facts.metrics.workspace_fact_count, 3);
}

#[test]
fn stack_publish_facts_keep_explicit_revset_subset() {
    // Verifies: explicit revsets select the publish subset without filling stack gaps.
    let fixture = TestWorkspace::new("stack-publish-explicit");
    let settings = user_settings().expect("settings");
    let (workspace, repo, parent_id, child_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let parent = write_child(tx.repo_mut(), &trunk, "parent change").await;
        let middle = write_child(tx.repo_mut(), &parent, "middle change").await;
        let child = write_child(tx.repo_mut(), &middle, "child change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), child.id().clone())
            .expect("set current working-copy change");

        let parent_id = parent.id().hex();
        let child_id = child.id().hex();
        let repo = tx
            .commit("arrange explicit stack publish")
            .await
            .expect("commit");
        (workspace, repo, parent_id, child_id)
    });
    let subject = JjWorkspace { workspace, repo };

    let facts = subject
        .stack_publish_facts(&StackPublishSelection::ExplicitRevisions {
            revisions: vec![format!("{parent_id} | {child_id}")],
        })
        .expect("stack publish facts load");

    assert_eq!(facts.publish_indexes, vec![0, 2]);
    assert_eq!(facts.anchor_index, None);
    assert_eq!(
        facts.nodes[0].workspace.target_change.description,
        "parent change"
    );
    assert_eq!(
        facts.nodes[1].workspace.target_change.description,
        "middle change"
    );
    assert_eq!(
        facts.nodes[2].workspace.target_change.description,
        "child change"
    );
    assert_eq!(facts.metrics.resolved_revision_count, 2);
    assert_eq!(facts.metrics.resolved_trunk_count, 2);
    assert_eq!(facts.metrics.stack_path_count, 2);
    assert_eq!(facts.metrics.workspace_fact_count, 3);
}

#[test]
fn stack_publish_facts_reject_multiple_selected_stacks() {
    // Verifies: one publish invocation cannot span unrelated stack roots yet.
    let fixture = TestWorkspace::new("stack-publish-multiple-stacks");
    let settings = user_settings().expect("settings");
    let (workspace, repo, left_id, right_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let left = write_child(tx.repo_mut(), &trunk, "left change").await;
        let right = write_child(tx.repo_mut(), &trunk, "right change").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), left.id().clone())
            .expect("set current working-copy change");

        let left_id = left.id().hex();
        let right_id = right.id().hex();
        let repo = tx
            .commit("arrange multi-stack publish")
            .await
            .expect("commit");
        (workspace, repo, left_id, right_id)
    });
    let subject = JjWorkspace { workspace, repo };

    let error = subject
        .stack_publish_facts(&StackPublishSelection::ExplicitRevisions {
            revisions: vec![left_id, right_id],
        })
        .expect_err("multi-stack publish is rejected");

    assert!(matches!(error, JjError::StackPublishMultipleStacks));
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
