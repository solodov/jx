use super::*;

#[test]
fn workspace_list_parser_extracts_workspace_names() {
    // Verifies: Workspace list parsing uses jj's stable `name:` prefix and ignores blank lines.
    let names =
        workspace_names_from_jj_list("default: abcdef12 (empty) primary\nfix: 12345678 work\n\n");

    assert_eq!(names, vec!["default", "fix"]);
}

#[test]
fn workspace_entries_use_repository_root_for_current_workspace() {
    // Verifies: Current workspace removal can resolve the active root even when jj has no recorded path for it.
    let fixture = TestWorkspace::new("workspace-entries-current-root");
    let settings = user_settings().expect("settings");
    pollster::block_on(Workspace::init_internal_git(&settings, fixture.path()))
        .expect("initialize jj workspace");

    let entries = jj_workspace_entries(fixture.path()).expect("workspace entries load");

    assert_eq!(
        entries,
        vec![WorkspaceEntry {
            name: "default".to_owned(),
            root: fixture.path().to_path_buf(),
            is_current: true,
        }]
    );
}

#[test]
fn workspace_root_missing_path_error_is_skippable() {
    // Verifies: Work list can ignore stale jj workspace records that have no usable path.
    let error = JjError::WorkspaceRootFailed {
        name: "stale".to_owned(),
        status: "exit code 1: Workspace has no recorded path: stale".to_owned(),
    };

    assert!(workspace_root_is_missing_recorded_path(&error));
}

#[test]
fn workspace_cleanup_removes_empty_managed_parents() {
    // Verifies: Workspace removal prunes the repo layout directory and `.work` when both are empty.
    let fixture = TestWorkspace::new("workspace-cleanup-empty");
    let cleanup_root = fixture.path().join(".work");
    let workspace_root = cleanup_root.join("jx/fix");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::remove_dir_all(&workspace_root).expect("delete workspace root");

    remove_empty_workspace_dirs(&workspace_root, &cleanup_root).expect("cleanup succeeds");

    assert!(!cleanup_root.exists());
}

#[test]
fn workspace_cleanup_keeps_non_empty_managed_parents() {
    // Verifies: Workspace cleanup stops before directories that still contain other work.
    let fixture = TestWorkspace::new("workspace-cleanup-non-empty");
    let cleanup_root = fixture.path().join(".work");
    let workspace_root = cleanup_root.join("jx/fix");
    let sibling = cleanup_root.join("jx/other");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&sibling).expect("create sibling workspace");
    fs::remove_dir_all(&workspace_root).expect("delete workspace root");

    remove_empty_workspace_dirs(&workspace_root, &cleanup_root).expect("cleanup succeeds");

    assert!(cleanup_root.join("jx").exists());
    assert!(cleanup_root.exists());
}

#[test]
fn workspace_add_preflight_rejects_existing_destination() {
    // Verifies: Workspace-add preflight rejects an existing destination before invoking jj.
    let fixture = TestWorkspace::new("workspace-add-existing-destination");
    let settings = user_settings().expect("settings");
    pollster::block_on(Workspace::init_internal_git(&settings, fixture.path()))
        .expect("initialize jj workspace");
    let destination = fixture.path().join("existing-workspace");
    fs::create_dir_all(&destination).expect("create existing destination");

    let error = run_jj_workspace_add(
        fixture.path(),
        &WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: destination.clone(),
            revision: None,
            shared_paths: Vec::new(),
        },
    )
    .expect_err("existing destination is rejected");

    assert!(matches!(error, JjError::WorkspacePathExists { path } if path == destination));
}

#[test]
fn workspace_shared_path_preflight_rejects_tracked_exact_path_only() {
    // Verifies: Shared path preflight checks the exact configured path, not tracked parents.
    let fixture = TestWorkspace::new("shared-path-tracked-exact");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child_with_files(
            tx.repo_mut(),
            &root,
            "current tracks local-looking paths",
            &[
                (".pi/config.toml", b"pi state\n".as_slice()),
                (".foo/bar/other.txt", b"tracked parent\n".as_slice()),
            ],
        )
        .await;
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        tx.commit("arrange tracked shared paths")
            .await
            .expect("commit arrangement");
    });

    let error =
        validate_workspace_shared_paths_untracked(fixture.path(), None, &[".pi".to_owned()])
            .expect_err("tracked exact shared path is rejected");

    assert!(matches!(
        error,
        JjError::WorkspaceSharedPathsTracked { paths } if paths == vec![".pi"]
    ));
    validate_workspace_shared_paths_untracked(fixture.path(), None, &[".foo/bar/baz".to_owned()])
        .expect("tracked parent directories do not reject an untracked configured tail");
}

#[test]
fn workspace_shared_path_preflight_uses_selected_revision() {
    // Verifies: Shared path preflight checks the checkout selected for `jj workspace add -r`.
    let fixture = TestWorkspace::new("shared-path-selected-revision");
    let settings = user_settings().expect("settings");
    let (ancestor_id, current_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let ancestor = write_child(tx.repo_mut(), &root, "ancestor without shared path").await;
        let current = write_child_with_files(
            tx.repo_mut(),
            &ancestor,
            "current tracks shared path",
            &[(".pi/config.toml", b"pi state\n".as_slice())],
        )
        .await;
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        tx.commit("arrange selected revision shared paths")
            .await
            .expect("commit arrangement");
        (ancestor.id().hex(), current.id().hex())
    });

    validate_workspace_shared_paths_untracked(
        fixture.path(),
        Some(&ancestor_id),
        &[".pi".to_owned()],
    )
    .expect("untracked path at selected revision passes");
    let error = validate_workspace_shared_paths_untracked(
        fixture.path(),
        Some(&current_id),
        &[".pi".to_owned()],
    )
    .expect_err("tracked path at selected revision is rejected");

    assert!(matches!(
        error,
        JjError::WorkspaceSharedPathsTracked { paths } if paths == vec![".pi"]
    ));
}
