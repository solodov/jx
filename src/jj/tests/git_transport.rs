use super::*;

#[test]
fn export_git_refs_updates_backing_git_bookmark_view() {
    // Verifies: Git export keeps backing Git branch state aligned with jj bookmarks.
    let fixture = TestWorkspace::new("export-git-refs");
    let settings = user_settings().expect("settings");

    pollster::block_on(async {
        let (_workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;

        set_local_bookmark(tx.repo_mut(), "main", trunk.id());
        export_git_refs(tx.repo_mut()).expect("export local bookmarks");
        let remote = tx
            .repo()
            .view()
            .get_remote_bookmark(
                RefName::new("main").to_remote_symbol(git::REMOTE_NAME_FOR_LOCAL_GIT_REPO),
            )
            .clone();

        assert_eq!(remote.target.as_normal(), Some(trunk.id()));
        assert!(remote.is_tracked());
    });
}

#[test]
fn prepare_initial_publish_target_describes_undescribed_root_child() {
    // Verifies: Repository bootstrap can publish a fresh jj repo whose first commit lacks text.
    let fixture = TestWorkspace::new("prepare-initial-description");
    let settings = user_settings().expect("settings");
    let (workspace, repo, initial) = pollster::block_on(async {
        let (mut workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let initial =
            write_child_with_files(tx.repo_mut(), &root, "", &[("README.md", b"hello\n")]).await;

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), initial.id().clone())
            .expect("set current working-copy change");
        let repo = tx
            .commit("arrange undescribed initial commit")
            .await
            .expect("commit");
        workspace
            .check_out(repo.op_id().clone(), None, &initial)
            .await
            .expect("checkout initial working-copy tree");
        (workspace, repo, initial)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let target = subject
        .initial_publish_target()
        .expect("initial publish target exists");
    let prepared = subject
        .prepare_initial_publish_target(&target)
        .expect("initial publish target is described");

    let current_id = subject
        .repo
        .view()
        .get_wc_commit_id(subject.workspace.workspace_name())
        .expect("working-copy commit exists");
    let current = load_commit_from_repo(subject.repo.as_ref(), current_id)
        .expect("load prepared working-copy commit");
    let current_is_empty = pollster::block_on(current.is_empty(subject.repo.as_ref()))
        .expect("check prepared working-copy tree");

    assert_eq!(target.commit_id, initial.id().hex());
    assert!(target.description.is_empty());
    assert_ne!(prepared.commit_id, target.commit_id);
    assert_eq!(prepared.description, "initial commit");
    assert_eq!(current_id.hex(), prepared.commit_id);
    assert_eq!(current.description(), "initial commit");
    assert!(!current_is_empty);
}

#[test]
fn prepare_initial_publish_target_snapshots_fresh_repo_files() {
    // Verifies: Initial repository bootstrap includes files that existed before jj init.
    let fixture = TestWorkspace::new("prepare-initial-files");
    fs::write(fixture.path().join("README.md"), b"hello\n").expect("write working-copy file");
    let settings = user_settings().expect("settings");
    let (workspace, repo, initial_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let current_id = repo
            .view()
            .get_wc_commit_id(workspace.workspace_name())
            .expect("working-copy commit exists")
            .clone();
        (workspace, repo, current_id)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let target = subject
        .initial_publish_target()
        .expect("initial publish target exists before snapshot");
    let prepared = subject
        .prepare_initial_publish_target(&target)
        .expect("initial publish target is snapshotted and described");

    let prepared_commit = subject
        .load_commit(&CommitId::try_from_hex(&prepared.commit_id).expect("valid commit id"))
        .expect("load prepared initial commit");
    let readme_path = RepoPathBuf::from_internal_string("README.md").expect("valid repo path");
    let readme_value = pollster::block_on(prepared_commit.tree().path_value(&readme_path))
        .expect("read prepared tree path");

    assert_eq!(target.commit_id, initial_id.hex());
    assert!(target.description.is_empty());
    assert_ne!(prepared.commit_id, target.commit_id);
    assert_eq!(prepared.description, "initial commit");
    assert!(matches!(
        readme_value.as_normal(),
        Some(TreeValue::File { .. })
    ));
}

#[test]
fn prepare_initial_publish_target_describes_empty_fresh_repo_commit() {
    // Verifies: Repository bootstrap can publish a newly initialized repo before files are added.
    let fixture = TestWorkspace::new("prepare-empty-initial-description");
    let settings = user_settings().expect("settings");
    let (workspace, repo, initial_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let current_id = repo
            .view()
            .get_wc_commit_id(workspace.workspace_name())
            .expect("working-copy commit exists")
            .clone();
        (workspace, repo, current_id)
    });
    let mut subject = JjWorkspace { workspace, repo };

    let target = subject
        .initial_publish_target()
        .expect("empty initial publish target exists");
    let prepared = subject
        .prepare_initial_publish_target(&target)
        .expect("empty initial publish target is described");

    let prepared_commit = subject
        .load_commit(&CommitId::try_from_hex(&prepared.commit_id).expect("valid commit id"))
        .expect("load prepared initial commit");
    let prepared_is_empty = pollster::block_on(prepared_commit.is_empty(subject.repo.as_ref()))
        .expect("check prepared tree");

    assert_eq!(target.commit_id, initial_id.hex());
    assert!(target.description.is_empty());
    assert_ne!(prepared.commit_id, target.commit_id);
    assert_eq!(prepared.description, "initial commit");
    assert!(prepared_is_empty);
}
