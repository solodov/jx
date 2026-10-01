use super::*;

#[test]
fn fetch_rebases_unbookmarked_children_when_import_abandons_stack_root() {
    let fixture = TestWorkspace::new("fetch-import-abandoned-root");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize workspace");
        let mut tx = repo.start_transaction();
        let old_trunk = write_child_with_files(
            tx.repo_mut(),
            &repo.store().root_commit(),
            "old trunk",
            &[("query.txt", b"old query\n")],
        )
        .await;
        let landed = write_child_with_files(
            tx.repo_mut(),
            &old_trunk,
            "migrate query",
            &[("query.txt", b"migrated query\n")],
        )
        .await;
        let local_child = write_child_with_files(
            tx.repo_mut(),
            &landed,
            "rename criterion",
            &[("query.txt", b"restricted migrated query\n")],
        )
        .await;
        let follow_up = write_child_with_files(
            tx.repo_mut(),
            &local_child,
            "follow-up",
            &[("follow-up.txt", b"local work\n")],
        )
        .await;
        set_local_bookmark(tx.repo_mut(), "topic/root", landed.id());
        tx.repo_mut()
            .set_wc_commit(
                workspace.workspace_name().to_owned(),
                follow_up.id().clone(),
            )
            .expect("set current workspace");
        let immutable = ResolvedRevsetExpression::commit(old_trunk.id().clone()).ancestors();
        let roots = collect_trunk_child_changes(tx.repo(), old_trunk.id(), &immutable)
            .expect("snapshot roots before fetch");
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].commit_id, *landed.id());

        let updated_trunk = write_child_with_files(
            tx.repo_mut(),
            &old_trunk,
            "squash merge and other upstream work",
            &[
                ("query.txt", b"migrated query\n"),
                ("upstream.txt", b"upstream work\n"),
            ],
        )
        .await;
        tx.repo_mut()
            .set_local_bookmark_target(RefName::new("topic/root"), RefTarget::absent());
        tx.repo_mut().record_abandoned_commit(&landed);
        let mut steps = Vec::new();
        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &roots,
            &updated_trunk,
            &immutable,
            &BTreeMap::new(),
            &HashSet::from([landed.id().clone()]),
            &mut |step| steps.push(step),
        )
        .await
        .expect("repair fetched stack");

        assert!(steps.iter().any(|step| {
            step.attrs
                .contains(&fetch_trace_attr("decision", "import_abandoned_onto_trunk"))
                && step
                    .attrs
                    .contains(&fetch_trace_attr("original_commit", landed.id().hex()))
        }));
        let child = visible_import_commit(tx.repo(), &local_child);
        let descendant = visible_import_commit(tx.repo(), &follow_up);
        assert_eq!(child.parent_ids(), &[updated_trunk.id().clone()]);
        assert_eq!(descendant.parent_ids(), &[child.id().clone()]);
        assert!(!child.has_conflict());
        assert!(!descendant.has_conflict());
        assert!(stats
            .rebased_commits
            .iter()
            .all(|record| !record.has_conflict));
        assert_eq!(
            tx.repo()
                .view()
                .get_wc_commit_id(workspace.workspace_name()),
            Some(descendant.id())
        );
        assert!(!tx
            .repo()
            .view()
            .get_local_bookmark(RefName::new("topic/root"))
            .is_present());
        assert!(tx
            .repo()
            .resolve_change_id(landed.change_id())
            .expect("resolve landed change")
            .and_then(|targets| targets.into_visible())
            .is_none());

        let expected = write_child_with_files(
            tx.repo_mut(),
            &updated_trunk,
            "expected child",
            &[("query.txt", b"restricted migrated query\n")],
        )
        .await;
        assert_eq!(child.tree_ids(), expected.tree_ids());
        let expected_descendant = write_child_with_files(
            tx.repo_mut(),
            &expected,
            "expected follow-up",
            &[("follow-up.txt", b"local work\n")],
        )
        .await;
        assert_eq!(descendant.tree_ids(), expected_descendant.tree_ids());
        tx.commit("repair import-abandoned stack")
            .await
            .expect("commit repairs");
    });
}

#[test]
fn fetch_redirects_abandoned_prefixes_without_moving_protected_or_unrelated_stacks() {
    let fixture = TestWorkspace::new("fetch-import-abandoned-prefix");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("workspace");
        let mut tx = repo.start_transaction();
        let root = repo.store().root_commit();
        let base = write_child(tx.repo_mut(), &root, "old trunk").await;
        let first = write_child_with_files(
            tx.repo_mut(),
            &base,
            "first merged PR",
            &[("first.txt", b"first")],
        )
        .await;
        let second = write_child_with_files(
            tx.repo_mut(),
            &first,
            "second merged PR",
            &[("second.txt", b"second")],
        )
        .await;
        let child = write_child_with_files(
            tx.repo_mut(),
            &second,
            "local child",
            &[("local.txt", b"local")],
        )
        .await;
        let protected = write_child_with_files(
            tx.repo_mut(),
            &base,
            "green sibling",
            &[("protected.txt", b"protected")],
        )
        .await;
        let unrelated = write_child_with_files(
            tx.repo_mut(),
            &root,
            "other history",
            &[("unrelated.txt", b"unrelated")],
        )
        .await;
        let unrelated_child = write_child_with_files(
            tx.repo_mut(),
            &unrelated,
            "other history child",
            &[("other.txt", b"other")],
        )
        .await;
        let immutable = ResolvedRevsetExpression::commit(base.id().clone()).ancestors();
        let roots =
            collect_trunk_child_changes(tx.repo(), base.id(), &immutable).expect("snapshot roots");
        assert_eq!(roots.len(), 2);
        let trunk = write_child_with_files(
            tx.repo_mut(),
            &base,
            "merged trunk",
            &[("first.txt", b"first"), ("second.txt", b"second")],
        )
        .await;
        for abandoned in [&first, &second, &unrelated] {
            tx.repo_mut().record_abandoned_commit(abandoned);
        }
        let abandoned = HashSet::from([
            first.id().clone(),
            second.id().clone(),
            unrelated.id().clone(),
        ]);
        let protected_roots =
            BTreeMap::from([(protected.change_id().clone(), "topic/green".to_owned())]);
        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &roots,
            &trunk,
            &immutable,
            &protected_roots,
            &abandoned,
            &mut |_| {},
        )
        .await
        .expect("repair imported roots");
        let child = visible_import_commit(tx.repo(), &child);
        assert_eq!(child.parent_ids(), &[trunk.id().clone()]);
        assert!(!child.has_conflict());
        assert_eq!(
            visible_import_commit(tx.repo(), &protected).id(),
            protected.id()
        );
        assert_eq!(
            visible_import_commit(tx.repo(), &unrelated_child).parent_ids(),
            &[root.id().clone()]
        );
        assert!(stats
            .rebased_commits
            .iter()
            .all(|record| !record.has_conflict));
        let expected = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "expected child",
            &[("local.txt", b"local")],
        )
        .await;
        assert_eq!(child.tree_ids(), expected.tree_ids());
        tx.commit("repair abandoned prefix")
            .await
            .expect("commit repairs");
    });
}

#[test]
fn fetch_recovers_previously_stranded_children_without_new_import_abandonments() {
    let fixture = TestWorkspace::new("fetch-import-stranded-child");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("workspace");
        let mut tx = repo.start_transaction();
        let base = write_child_with_files(
            tx.repo_mut(),
            &repo.store().root_commit(),
            "old trunk",
            &[("query.txt", b"old\n")],
        )
        .await;
        let landed = write_child_with_files(
            tx.repo_mut(),
            &base,
            "merged PR",
            &[("query.txt", b"migrated\n")],
        )
        .await;
        let child = write_child_with_files(
            tx.repo_mut(),
            &landed,
            "local rename",
            &[("query.txt", b"renamed migrated\n")],
        )
        .await;
        let trunk = write_child_with_files(
            tx.repo_mut(),
            &base,
            "squash merge",
            &[("query.txt", b"migrated\n")],
        )
        .await;
        tx.repo_mut().record_abandoned_commit(&landed);
        tx.repo_mut()
            .rebase_descendants()
            .await
            .expect("simulate previous stale-parent replay");
        let stranded = visible_import_commit(tx.repo(), &child);
        assert_eq!(stranded.parent_ids(), &[base.id().clone()]);
        assert!(stranded.has_conflict());
        let immutable = ResolvedRevsetExpression::commit(trunk.id().clone()).ancestors();
        let roots = collect_trunk_child_changes(tx.repo(), trunk.id(), &immutable)
            .expect("rediscover stranded root");
        assert_eq!(roots.len(), 1);
        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &roots,
            &trunk,
            &immutable,
            &BTreeMap::new(),
            &HashSet::new(),
            &mut |_| {},
        )
        .await
        .expect("repair without another fetch change");
        let repaired = visible_import_commit(tx.repo(), &child);
        assert_eq!(repaired.parent_ids(), &[trunk.id().clone()]);
        assert!(!repaired.has_conflict());
        assert!(stats
            .rebased_commits
            .iter()
            .all(|record| !record.has_conflict));
        let expected = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "expected",
            &[("query.txt", b"renamed migrated\n")],
        )
        .await;
        assert_eq!(repaired.tree_ids(), expected.tree_ids());
        tx.commit("repair stranded child")
            .await
            .expect("commit repairs");
    });
}

/// Resolves a local change after import and trunk repair have rewritten it.
fn visible_import_commit(repo: &dyn jj_lib::repo::Repo, old: &Commit) -> Commit {
    let ids = repo
        .resolve_change_id(old.change_id())
        .expect("resolve change")
        .expect("change remains indexed")
        .into_visible()
        .expect("change remains visible");
    assert_eq!(ids.len(), 1);
    load_commit_from_repo(repo, &ids[0]).expect("load rewritten change")
}
