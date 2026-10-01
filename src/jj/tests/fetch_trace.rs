use super::*;
use crate::jj::fetch::trace::{record_import_details, record_root_snapshot};

#[test]
fn fetch_trace_identifies_empty_drop_and_clean_descendant() {
    let fixture = TestWorkspace::new("fetch-trace-empty-drop");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("workspace");
        let mut tx = repo.start_transaction();
        let base = write_child(tx.repo_mut(), &repo.store().root_commit(), "base").await;
        let topic =
            write_child_with_files(tx.repo_mut(), &base, "topic", &[("landed.txt", b"landed")])
                .await;
        let follow_up =
            write_child_with_files(tx.repo_mut(), &topic, "follow-up", &[("new.txt", b"new")])
                .await;
        let trunk = write_child_with_files(
            tx.repo_mut(),
            &base,
            "squash merge",
            &[("landed.txt", b"landed")],
        )
        .await;
        let roots = vec![root_change(&topic)];
        let mut steps = Vec::new();
        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &roots,
            &trunk,
            &RevsetExpression::none(),
            &BTreeMap::new(),
            &HashSet::new(),
            &mut |step| steps.push(step),
        )
        .await
        .expect("rebase");

        assert_eq!(stats.abandoned_empty_commits, 1);
        assert_eq!(stats.rebased_descendants, 1);
        let dropped = result_for(&steps, &topic);
        assert_attr(dropped, "outcome", "abandoned");
        assert_attr(dropped, "empty_policy", "abandon_newly_empty");
        assert_attr(dropped, "replacement_parent", trunk.id().hex());
        assert_attr(dropped, "old_parents", base.id().hex());
        let descendant = result_for(&steps, &follow_up);
        assert_attr(descendant, "phase", "descendant");
        assert_attr(descendant, "new_parents", trunk.id().hex());
        assert_attr(descendant, "conflict_before", false);
        assert_attr(descendant, "conflict_after", false);
    });
}

#[test]
fn fetch_trace_identifies_new_propagated_and_preexisting_conflicts() {
    let fixture = TestWorkspace::new("fetch-trace-conflicts");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("workspace");
        let mut tx = repo.start_transaction();
        let base = write_child_with_files(
            tx.repo_mut(),
            &repo.store().root_commit(),
            "base",
            &[("conflict.txt", b"base\n")],
        )
        .await;
        let topic = write_child_with_files(
            tx.repo_mut(),
            &base,
            "topic",
            &[("conflict.txt", b"local\n")],
        )
        .await;
        let follow_up =
            write_child_with_files(tx.repo_mut(), &topic, "follow-up", &[("new.txt", b"new")])
                .await;
        let trunk = write_child_with_files(
            tx.repo_mut(),
            &base,
            "upstream",
            &[("conflict.txt", b"upstream\n")],
        )
        .await;
        let mut steps = Vec::new();
        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &[root_change(&topic)],
            &trunk,
            &RevsetExpression::none(),
            &BTreeMap::new(),
            &HashSet::new(),
            &mut |step| steps.push(step),
        )
        .await
        .expect("rebase");
        let root_result = result_for(&steps, &topic);
        assert_attr(root_result, "phase", "root");
        assert_attr(root_result, "conflict_before", false);
        assert_attr(root_result, "conflict_after", true);
        assert_attr(root_result, "new_conflict", true);
        assert_attr(root_result, "conflict_paths", "[\"conflict.txt\"]");
        assert_attr(root_result, "conflicted_parents", "");
        let conflicted = stats
            .rebased_commits
            .iter()
            .find(|record| record.short_change_id == short_change_id(&topic))
            .expect("root result");
        let conflicted_id = CommitId::try_from_hex(&conflicted.new_commit_id).expect("commit id");
        assert_attr(
            result_for(&steps, &follow_up),
            "conflicted_parents",
            conflicted_id.hex(),
        );

        let prior_conflict =
            load_commit_from_repo(tx.repo(), &conflicted_id).expect("conflicted root");
        let next_trunk = write_child_with_files(
            tx.repo_mut(),
            &trunk,
            "later upstream",
            &[("unrelated.txt", b"new")],
        )
        .await;
        steps.clear();
        rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &[root_change(&prior_conflict)],
            &next_trunk,
            &RevsetExpression::none(),
            &BTreeMap::new(),
            &HashSet::new(),
            &mut |step| steps.push(step),
        )
        .await
        .expect("rebase existing conflict");
        let result = result_for(&steps, &prior_conflict);
        assert_attr(result, "conflict_before", true);
        assert_attr(result, "conflict_after", true);
        assert_attr(result, "new_conflict", false);
    });
}

#[test]
fn fetch_trace_explains_skipped_roots_and_import_rewrites() {
    let fixture = TestWorkspace::new("fetch-trace-skips");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("workspace");
        let mut tx = repo.start_transaction();
        let base = write_child(tx.repo_mut(), &repo.store().root_commit(), "base").await;
        let landed = write_child(tx.repo_mut(), &base, "landed").await;
        let trunk = write_child(tx.repo_mut(), &landed, "trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current").await;
        let protected = write_child(tx.repo_mut(), &base, "protected").await;
        let missing = write_child(tx.repo_mut(), &base, "abandoned").await;
        tx.repo_mut().record_abandoned_commit(&missing);
        let mut steps = Vec::new();
        let stats = rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &[
                root_change(&current),
                root_change(&protected),
                root_change(&landed),
                root_change(&missing),
            ],
            &trunk,
            &RevsetExpression::none(),
            &BTreeMap::from([(protected.change_id().clone(), "topic/protected".to_owned())]),
            &HashSet::new(),
            &mut |step| steps.push(step),
        )
        .await
        .expect("skip roots");
        assert_eq!(stats.skipped_trunk_children, 4);
        for (commit, decision) in [
            (&current, "already_on_trunk"),
            (&protected, "protected"),
            (&landed, "landed_by_ancestry"),
            (&missing, "unresolved"),
        ] {
            let step = steps
                .iter()
                .find(|step| {
                    step.name == "rebase_decision"
                        && step
                            .attrs
                            .contains(&fetch_trace_attr("change_id", commit.change_id().hex()))
                })
                .expect("root decision");
            assert_attr(step, "decision", decision);
        }

        let child = write_child(tx.repo_mut(), &base, "child of rewritten base").await;
        let replacement = tx
            .repo_mut()
            .new_commit(vec![repo.store().root_commit_id().clone()], base.tree())
            .set_change_id(base.change_id().clone())
            .set_description("replacement base")
            .write()
            .await
            .expect("replacement");
        tx.repo_mut()
            .set_rewritten_commit(base.id().clone(), replacement.id().clone());
        steps.clear();
        rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &[root_change(&child)],
            &replacement,
            &RevsetExpression::none(),
            &BTreeMap::new(),
            &HashSet::new(),
            &mut |step| steps.push(step),
        )
        .await
        .expect("import rewrite");
        assert_attr(result_for(&steps, &child), "phase", "import_rewrite");
        assert_attr(result_for(&steps, &child), "empty_policy", "keep");
    });
}

#[test]
fn fetch_trace_preserves_bookmark_identity_before_deletion() {
    let fixture = TestWorkspace::new("fetch-trace-import");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("workspace");
        let mut tx = repo.start_transaction();
        let topic = write_child(tx.repo_mut(), &repo.store().root_commit(), "topic").await;
        set_local_bookmark(tx.repo_mut(), "topic/root", topic.id());
        set_origin_bookmark(tx.repo_mut(), "topic/root", topic.id());
        let mut steps = Vec::new();
        record_root_snapshot(tx.repo(), &root_change(&topic), &mut |step| {
            steps.push(step)
        });
        assert_attr(&steps[0], "bookmarks", "topic/root");
        let old_remote = tx
            .repo()
            .view()
            .get_remote_bookmark(
                RefName::new("topic/root").to_remote_symbol(RemoteName::new("origin")),
            )
            .clone();
        let before = BTreeMap::from([(
            "topic/root".to_owned(),
            RefTarget::normal(topic.id().clone()),
        )]);
        tx.repo_mut()
            .set_local_bookmark_target(RefName::new("topic/root"), RefTarget::absent());
        let stats = git::GitImportStats {
            changed_remote_bookmarks: vec![(
                RefName::new("topic/root")
                    .to_remote_symbol(RemoteName::new("origin"))
                    .to_owned(),
                (old_remote, RefTarget::absent()),
            )],
            abandoned_commits: vec![topic.clone()],
            rewritten_commit_ids: HashSet::from([topic.id().clone()]),
            ..Default::default()
        };
        record_import_details(tx.repo(), &before, &stats, &mut |step| steps.push(step));
        let changed = steps
            .iter()
            .find(|step| step.name == "import_bookmark")
            .expect("bookmark diagnostic");
        assert_attr(changed, "deleted", true);
        assert_attr(changed, "old_remote_target", topic.id().hex());
        assert_attr(changed, "old_local_target", topic.id().hex());
        assert_attr(changed, "new_local_target", "");
        assert!(steps.iter().any(|step| step.name == "import_abandoned"));
        assert!(steps.iter().any(|step| step.name == "import_rewritten"));
    });
}

#[test]
fn fetch_trace_bounds_conflict_paths_without_recording_file_contents() {
    let fixture = TestWorkspace::new("fetch-trace-many-conflicts");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (_, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("workspace");
        let mut tx = repo.start_transaction();
        let base = write_child(tx.repo_mut(), &repo.store().root_commit(), "base").await;
        let paths = (0..25)
            .map(|index| format!("file-{index:02}.txt"))
            .collect::<Vec<_>>();
        let local_files = paths
            .iter()
            .map(|path| (path.as_str(), b"private-local-content\n".as_slice()))
            .collect::<Vec<_>>();
        let upstream_files = paths
            .iter()
            .map(|path| (path.as_str(), b"private-upstream-content\n".as_slice()))
            .collect::<Vec<_>>();
        let topic = write_child_with_files(tx.repo_mut(), &base, "topic", &local_files).await;
        let trunk = write_child_with_files(tx.repo_mut(), &base, "trunk", &upstream_files).await;
        let mut steps = Vec::new();
        rebase_trunk_child_changes_onto_updated_trunk(
            tx.repo_mut(),
            &[root_change(&topic)],
            &trunk,
            &RevsetExpression::none(),
            &BTreeMap::new(),
            &HashSet::new(),
            &mut |step| steps.push(step),
        )
        .await
        .expect("rebase");
        let result = result_for(&steps, &topic);
        assert_attr(result, "conflict_paths_truncated", true);
        let paths = result
            .attrs
            .iter()
            .find(|attr| attr.key == "conflict_paths")
            .expect("paths");
        let FetchTraceValue::String(paths) = &paths.value else {
            panic!("paths are a JSON string")
        };
        assert_eq!(
            serde_json::from_str::<Vec<String>>(paths)
                .expect("JSON paths")
                .len(),
            20
        );
        assert!(!format!("{steps:?}").contains("private-local-content"));
        assert!(!format!("{steps:?}").contains("private-upstream-content"));
    });
}

fn root_change(commit: &Commit) -> TrunkChildChange {
    TrunkChildChange {
        commit_id: commit.id().clone(),
        change_id: commit.change_id().clone(),
    }
}

fn result_for<'a>(steps: &'a [FetchTraceStep], commit: &Commit) -> &'a FetchTraceStep {
    steps
        .iter()
        .find(|step| {
            step.name == "rebase_result"
                && step
                    .attrs
                    .contains(&fetch_trace_attr("old_commit", commit.id().hex()))
        })
        .expect("rebase result")
}

fn assert_attr(step: &FetchTraceStep, key: &str, value: impl Into<FetchTraceValue>) {
    assert!(
        step.attrs.contains(&fetch_trace_attr(key, value)),
        "missing {key} in {step:?}"
    );
}
