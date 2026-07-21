use super::*;

#[test]
fn push_tracked_selects_tracked_updates_and_deletions() {
    // Verifies: Tracked push sends only tracked origin updates and includes deletions.
    let fixture = TestWorkspace::new("push-tracked-updates");
    let settings = user_settings().expect("settings");
    let (workspace, repo, trunk, current, old) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current change").await;
        let old = write_child(tx.repo_mut(), &trunk, "old branch").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", trunk.id());
        set_origin_bookmark(tx.repo_mut(), "feature", trunk.id());
        set_local_bookmark(tx.repo_mut(), "feature", current.id());
        set_origin_bookmark(tx.repo_mut(), "old", old.id());
        set_untracked_origin_bookmark(tx.repo_mut(), "untracked", old.id());

        let repo = tx
            .commit("arrange tracked push updates")
            .await
            .expect("commit");
        (workspace, repo, trunk, current, old)
    });
    let subject = JjWorkspace { workspace, repo };

    let updates = subject
        .tracked_origin_bookmark_updates()
        .expect("tracked updates classify");

    assert_eq!(
        updates
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["feature", "old"]
    );
    assert_eq!(updates[0].1.before.as_ref(), Some(trunk.id()));
    assert_eq!(updates[0].1.after.as_ref(), Some(current.id()));
    assert_eq!(updates[1].1.before.as_ref(), Some(old.id()));
    assert_eq!(updates[1].1.after, None);
}

#[test]
fn syncable_tracked_push_skips_bookmarks_with_conflicted_commits() {
    // Verifies: Sync filters conflicted bookmark updates while retaining clean pushes and deletions.
    let fixture = TestWorkspace::new("push-syncable-conflicts");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old_trunk = write_child_with_files(
            tx.repo_mut(),
            &root,
            "old main trunk",
            &[("conflict.txt", b"base\n")],
        )
        .await;
        let local_conflict = write_child_with_files(
            tx.repo_mut(),
            &old_trunk,
            "conflicting branch",
            &[("conflict.txt", b"local\n")],
        )
        .await;
        let updated_trunk = write_child_with_files(
            tx.repo_mut(),
            &old_trunk,
            "updated main trunk",
            &[("conflict.txt", b"upstream\n")],
        )
        .await;
        let clean = write_child_with_files(
            tx.repo_mut(),
            &updated_trunk,
            "clean branch",
            &[("clean.txt", b"clean\n")],
        )
        .await;
        let deleted = write_child(tx.repo_mut(), &updated_trunk, "deleted branch").await;

        let trunk_children = vec![TrunkChildChange {
            commit_id: local_conflict.id().clone(),
            change_id: local_conflict.change_id().clone(),
        }];
        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &trunk_children,
            &updated_trunk,
            &RevsetExpression::none(),
            &BTreeMap::new(),
        )
        .await
        .expect("conflicting child is rebased");
        let conflicted_id = CommitId::try_from_hex(&stats.rebased_commits[0].new_commit_id)
            .expect("rebased commit id is valid");
        let conflicted = load_commit_from_repo(tx.repo(), &conflicted_id)
            .expect("load conflicted rebased commit");
        assert!(conflicted.has_conflict());

        set_origin_bookmark(tx.repo_mut(), "main", updated_trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", updated_trunk.id());
        set_origin_bookmark(tx.repo_mut(), "clean", updated_trunk.id());
        set_local_bookmark(tx.repo_mut(), "clean", clean.id());
        set_origin_bookmark(tx.repo_mut(), "conflicted", updated_trunk.id());
        set_local_bookmark(tx.repo_mut(), "conflicted", conflicted.id());
        set_origin_bookmark(tx.repo_mut(), "deleted", deleted.id());

        let repo = tx
            .commit("arrange syncable tracked push")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let subject = JjWorkspace { workspace, repo };
    let updates = subject
        .tracked_origin_bookmark_updates()
        .expect("tracked updates classify");

    let split = subject
        .split_conflicted_tracked_bookmark_updates(updates)
        .expect("conflicted updates split");

    assert_eq!(
        split
            .pushable
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["clean", "deleted"]
    );
    assert_eq!(split.skipped_conflicted.len(), 1);
    assert_eq!(split.skipped_conflicted[0].branch, "conflicted");
    assert_eq!(split.skipped_conflicted[0].conflicted_commits.len(), 1);
    assert_eq!(
        split.skipped_conflicted[0].conflicted_commits[0].description,
        "conflicting branch"
    );
}

#[test]
fn syncable_tracked_push_includes_unchanged_bookmark_pr_metadata() {
    // Verifies: repository sync can repair PR metadata even when bookmark refs already match origin.
    let fixture = TestWorkspace::new("push-syncable-unchanged-pr-metadata");
    let settings = user_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let parent = write_child(tx.repo_mut(), &trunk, "Parent PR").await;
        let child = write_child(tx.repo_mut(), &parent, "Child PR\n\nChild body").await;

        set_origin_bookmark(tx.repo_mut(), "main", trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", trunk.id());
        set_origin_bookmark(tx.repo_mut(), "example-user/parent", parent.id());
        set_local_bookmark(tx.repo_mut(), "example-user/parent", parent.id());
        set_origin_bookmark(tx.repo_mut(), "example-user/child", child.id());
        set_local_bookmark(tx.repo_mut(), "example-user/child", child.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), child.id().clone())
            .expect("set working-copy commit");

        let repo = tx
            .commit("arrange unchanged syncable bookmarks")
            .await
            .expect("commit");
        (workspace, repo)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .push_syncable_tracked(SyncPushOptions::default())
        .expect("syncable tracked push succeeds");

    assert_eq!(outcome.pushed.pushed_refs, 0);
    assert_eq!(
        outcome
            .pushed
            .bookmarks
            .iter()
            .map(|bookmark| (
                bookmark.branch.as_str(),
                bookmark.pull_request_description.as_deref(),
                bookmark.pull_request_base.as_deref(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "example-user/child",
                Some("Child PR\n\nChild body"),
                Some("example-user/parent")
            ),
            ("example-user/parent", Some("Parent PR"), Some("main")),
        ]
    );
}

#[test]
fn experimental_syncable_tracked_push_skips_same_tree_update_and_adopts_non_current_bookmark() {
    // Verifies: The experimental mode preserves a green remote PR head when local code is identical and the PR bookmark is not current.
    let fixture = TestWorkspace::new("push-syncable-same-tree-adopt");
    let settings = user_settings().expect("settings");
    let (workspace, repo, remote, local) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let remote = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "Remote PR title\n\nRemote body",
            &[("same.txt", b"same\n")],
        )
        .await;
        let new_trunk = write_child(tx.repo_mut(), &trunk, "updated main trunk").await;
        let local = tx
            .repo_mut()
            .new_commit(vec![new_trunk.id().clone()], remote.tree())
            .set_description("Local PR title\n\nLocal body")
            .write()
            .await
            .expect("write same-tree local commit");

        set_origin_bookmark(tx.repo_mut(), "main", new_trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", new_trunk.id());
        set_origin_bookmark(tx.repo_mut(), "example-user/topic", remote.id());
        set_local_bookmark(tx.repo_mut(), "example-user/topic", local.id());
        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                new_trunk.id().clone(),
            )
            .expect("set working-copy commit");

        let repo = tx
            .commit("arrange same-tree tracked push")
            .await
            .expect("commit");
        (workspace, repo, remote, local)
    });
    let mut subject = JjWorkspace { workspace, repo };
    assert_eq!(
        subject
            .tracked_bookmark_sync_status_lines()
            .expect("sync status lines render")
            .into_iter()
            .filter(|line| line.contains("example-user/topic"))
            .collect::<Vec<_>>(),
        vec![format!(
            "  ≈ example-user/topic local code {} matches GitHub {}; sync will update remote head",
            short_commit_id(local.id()),
            short_commit_id(remote.id())
        )]
    );

    let outcome = subject
        .push_syncable_tracked(SyncPushOptions {
            skip_same_tree_pushes: true,
        })
        .expect("syncable tracked push succeeds");

    assert_eq!(outcome.pushed.pushed_refs, 0);
    assert!(outcome.pushed.pushed_commits.is_empty());
    assert_eq!(outcome.skipped_same_tree_bookmarks.len(), 1);
    assert_eq!(
        outcome.skipped_same_tree_bookmarks[0].branch,
        "example-user/topic"
    );
    assert!(outcome.skipped_same_tree_bookmarks[0].adopted_remote_head);
    let bookmark = outcome
        .pushed
        .bookmarks
        .iter()
        .find(|bookmark| bookmark.branch == "example-user/topic")
        .expect("same-tree bookmark is still available for PR metadata sync");
    assert_eq!(
        bookmark.pull_request_description.as_deref(),
        Some("Local PR title\n\nLocal body")
    );
    assert_eq!(
        subject
            .repo
            .view()
            .get_local_bookmark(RefName::new("example-user/topic"))
            .as_normal(),
        Some(remote.id())
    );
    assert_ne!(remote.id(), local.id());
}

#[test]
fn experimental_syncable_tracked_push_skips_same_tree_current_bookmark_without_adopting() {
    // Verifies: The experimental mode preserves remote CI without moving the bookmark away from the current working copy.
    let fixture = TestWorkspace::new("push-syncable-same-tree-current");
    let settings = user_settings().expect("settings");
    let (workspace, repo, remote, local) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let remote = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "Remote PR title",
            &[("same.txt", b"same\n")],
        )
        .await;
        let new_trunk = write_child(tx.repo_mut(), &trunk, "updated main trunk").await;
        let local = tx
            .repo_mut()
            .new_commit(vec![new_trunk.id().clone()], remote.tree())
            .set_description("Local PR title")
            .write()
            .await
            .expect("write same-tree current commit");

        set_origin_bookmark(tx.repo_mut(), "main", new_trunk.id());
        set_local_bookmark(tx.repo_mut(), "main", new_trunk.id());
        set_origin_bookmark(tx.repo_mut(), "example-user/topic", remote.id());
        set_local_bookmark(tx.repo_mut(), "example-user/topic", local.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), local.id().clone())
            .expect("set working-copy commit");

        let repo = tx
            .commit("arrange current same-tree tracked push")
            .await
            .expect("commit");
        (workspace, repo, remote, local)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let outcome = subject
        .push_syncable_tracked(SyncPushOptions {
            skip_same_tree_pushes: true,
        })
        .expect("syncable tracked push succeeds");

    assert_eq!(outcome.pushed.pushed_refs, 0);
    assert_eq!(outcome.skipped_same_tree_bookmarks.len(), 1);
    assert!(!outcome.skipped_same_tree_bookmarks[0].adopted_remote_head);
    assert_eq!(
        subject
            .repo
            .view()
            .get_local_bookmark(RefName::new("example-user/topic"))
            .as_normal(),
        Some(local.id())
    );
    assert_ne!(remote.id(), local.id());
}

#[test]
fn bookmark_pull_request_description_uses_first_stack_commit() {
    // Verifies: Sync PR text follows the PR-opening stack root, not later review-fix commits.
    let fixture = TestWorkspace::new("bookmark-pr-description-root");
    let settings = user_settings().expect("settings");
    let (repo, trunk, tip) = pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let first = write_child(tx.repo_mut(), &trunk, "PR title\n\nPR body").await;
        let tip = write_child(tx.repo_mut(), &first, "address review comments").await;
        let repo = tx
            .commit("arrange bookmark PR description stack")
            .await
            .expect("commit");
        (repo, trunk, tip)
    });

    let trunk = TrackedPushTrunk {
        branch: "main".to_owned(),
        id: trunk.id().clone(),
    };

    let description =
        bookmark_pull_request_description(repo.as_ref(), Some(tip.id()), Some(&trunk))
            .expect("description resolves");

    assert_eq!(description.as_deref(), Some("PR title\n\nPR body"));
}

#[test]
fn bookmark_pull_request_description_uses_first_commit_after_parent_bookmark() {
    // Verifies: child PR text follows its own stack root when trunk has not advanced.
    let fixture = TestWorkspace::new("bookmark-pr-description-parent");
    let settings = user_settings().expect("settings");
    let (repo, trunk, tip) = pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let parent = write_child(tx.repo_mut(), &trunk, "Parent PR").await;
        let child = write_child(tx.repo_mut(), &parent, "Child PR\n\nChild body").await;
        let tip = write_child(tx.repo_mut(), &child, "address child review comments").await;
        set_local_bookmark(tx.repo_mut(), "example-user/parent", parent.id());
        let repo = tx
            .commit("arrange child bookmark PR description stack")
            .await
            .expect("commit");
        (repo, trunk, tip)
    });
    let trunk = TrackedPushTrunk {
        branch: "main".to_owned(),
        id: trunk.id().clone(),
    };

    let description =
        bookmark_pull_request_description(repo.as_ref(), Some(tip.id()), Some(&trunk))
            .expect("description resolves");

    assert_eq!(description.as_deref(), Some("Child PR\n\nChild body"));
}

#[test]
fn bookmark_pull_request_base_uses_nearest_stack_bookmark() {
    // Verifies: Sync retargets stacked PRs to the nearest bookmarked ancestor.
    let fixture = TestWorkspace::new("bookmark-pr-base-parent");
    let settings = user_settings().expect("settings");
    let (repo, trunk, tip) = pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let parent = write_child(tx.repo_mut(), &trunk, "parent PR").await;
        let tip = write_child(tx.repo_mut(), &parent, "child PR").await;
        set_local_bookmark(tx.repo_mut(), "example-user/parent", parent.id());
        let repo = tx.commit("arrange bookmark PR base").await.expect("commit");
        (repo, trunk, tip)
    });
    let trunk = TrackedPushTrunk {
        branch: "main".to_owned(),
        id: trunk.id().clone(),
    };

    let base = bookmark_pull_request_base(repo.as_ref(), Some(tip.id()), Some(&trunk))
        .expect("base resolves");

    assert_eq!(base.as_deref(), Some("example-user/parent"));
}

#[test]
fn bookmark_pull_request_base_uses_trunk_for_stack_root() {
    // Verifies: root PRs keep targeting the resolved trunk branch.
    let fixture = TestWorkspace::new("bookmark-pr-base-trunk");
    let settings = user_settings().expect("settings");
    let (repo, trunk, target) = pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let target = write_child(tx.repo_mut(), &trunk, "root PR").await;
        let repo = tx.commit("arrange root PR base").await.expect("commit");
        (repo, trunk, target)
    });
    let trunk = TrackedPushTrunk {
        branch: "main".to_owned(),
        id: trunk.id().clone(),
    };

    let base = bookmark_pull_request_base(repo.as_ref(), Some(target.id()), Some(&trunk))
        .expect("base resolves");

    assert_eq!(base.as_deref(), Some("main"));
}

#[test]
fn push_bookmark_validates_local_and_remote_state_before_transport() {
    // Verifies: Bookmark push validates local and remote state before transport.
    let fixture = TestWorkspace::new("push-validation");
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
            .commit("arrange push validation workspace")
            .await
            .expect("commit");
        (workspace, repo, trunk, current)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let missing = subject
        .push_bookmark("example-user/missing")
        .expect_err("missing local bookmark is rejected before transport");

    assert!(matches!(
        missing,
        JjError::MissingLocalBookmark { branch } if branch == "example-user/missing"
    ));

    let mut tx = subject.repo.start_transaction();
    set_local_bookmark(tx.repo_mut(), "example-user/current", current.id());
    tx.repo_mut().set_remote_bookmark(
        RefName::new("example-user/current").to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME)),
        RemoteRef {
            target: RefTarget::from_legacy_form([trunk.id().clone()], [current.id().clone()]),
            state: RemoteRefState::Tracked,
        },
    );
    subject.repo =
        pollster::block_on(tx.commit("arrange conflicted remote bookmark")).expect("commit");

    let conflicted = subject
        .push_bookmark("example-user/current")
        .expect_err("conflicted remote bookmark is rejected before transport");

    assert!(matches!(
        conflicted,
        JjError::ConflictedRemoteBookmark { branch, remote }
            if branch == "example-user/current" && remote == ORIGIN_REMOTE_NAME
    ));
}
