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
    pollster::block_on(Workspace::init_internal_git(
        &settings,
        fixture.path(),
        gix::hash::Kind::Sha1,
    ))
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
fn workspace_removal_allows_recreating_colocated_workspace() {
    assert_workspace_removal_allows_recreation(true);
}

#[test]
fn workspace_removal_allows_recreating_non_colocated_workspace() {
    assert_workspace_removal_allows_recreation(false);
}

/// Exercises deletion through the real jj CLI, including a sibling that must survive.
fn assert_workspace_removal_allows_recreation(colocated: bool) {
    let fixture = TestWorkspace::new("workspace-removal-recreation");
    let primary = fixture.path().join("primary");
    fs::create_dir_all(&primary).expect("create primary workspace");
    let settings = user_settings().expect("settings");
    if colocated {
        pollster::block_on(Workspace::init_colocated_git(
            &settings,
            &primary,
            gix::hash::Kind::Sha1,
        ))
        .expect("initialize colocated workspace");
    } else {
        pollster::block_on(Workspace::init_internal_git(
            &settings,
            &primary,
            gix::hash::Kind::Sha1,
        ))
        .expect("initialize non-colocated workspace");
    }
    fs::write(
        primary.join(".jj/repo/config.toml"),
        "[git]\ncolocate = true\n",
    )
    .expect("enable colocation for colocated primary");
    let cleanup_root = fixture.path().join(".work");
    let options = WorkspaceAddOptions {
        name: "on-duty".to_owned(),
        destination: cleanup_root.join("jx/on-duty"),
        revision: None,
        shared_paths: Vec::new(),
    };
    run_jj_workspace_add(&primary, &options).expect("add workspace");
    let sibling = WorkspaceAddOptions {
        name: "other".to_owned(),
        destination: cleanup_root.join("jx/other"),
        ..options.clone()
    };
    run_jj_workspace_add(&primary, &sibling).expect("add sibling workspace");
    fs::write(sibling.destination.join("keep.txt"), "sibling work\n").expect("write sibling work");
    assert_eq!(options.destination.join(".git").is_file(), colocated);

    let original_dir = std::env::current_dir().expect("current directory");
    let result = remove_jj_workspace(
        &primary,
        &WorkspaceRemoveOptions {
            name: options.name.clone(),
            root: options.destination.clone(),
            cleanup_root,
        },
    );
    std::env::set_current_dir(original_dir).expect("restore test working directory");
    result.expect("remove workspace");

    assert!(!options.destination.exists());
    assert!(!primary.join(".git/worktrees/on-duty").exists());
    assert_eq!(
        fs::read_to_string(sibling.destination.join("keep.txt")).expect("read sibling work"),
        "sibling work\n",
    );
    if colocated {
        assert!(primary.join(".git/worktrees/other").is_dir());
    }
    run_jj_workspace_add(&primary, &options).expect("recreate workspace at the same path");
    assert_eq!(options.destination.join(".git").is_file(), colocated);
}

#[test]
fn workspace_add_preflight_rejects_existing_destination() {
    // Verifies: Workspace-add preflight rejects an existing destination before invoking jj.
    let fixture = TestWorkspace::new("workspace-add-existing-destination");
    let settings = user_settings().expect("settings");
    pollster::block_on(Workspace::init_internal_git(
        &settings,
        fixture.path(),
        gix::hash::Kind::Sha1,
    ))
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
fn workspace_add_recovers_missing_registered_git_worktree() {
    assert_workspace_add_recovers_stale_git_worktree(false);
}

#[test]
fn workspace_add_recovers_empty_registered_git_worktree() {
    assert_workspace_add_recovers_stale_git_worktree(true);
}

#[test]
fn workspace_add_preserves_locked_stale_git_worktree() {
    let (_fixture, primary, options) = stale_git_worktree_fixture();
    let registration = primary.join(".git/worktrees/on-duty");
    fs::write(registration.join("locked"), "keep this worktree\n").expect("lock worktree");
    fs::create_dir(&options.destination).expect("leave empty directory from failed add");

    let error = run_jj_workspace_add(&primary, &options).expect_err("locked worktree rejected");

    assert!(error.to_string().contains("locked"), "{error}");
    assert!(registration.join("locked").is_file());
    assert!(options.destination.is_dir());
}

#[test]
fn workspace_add_preserves_nonempty_stale_git_worktree_directory() {
    let (_fixture, primary, options) = stale_git_worktree_fixture();
    fs::create_dir(&options.destination).expect("create destination");
    fs::write(options.destination.join("keep.txt"), "local work\n").expect("write local work");

    let error = run_jj_workspace_add(&primary, &options).expect_err("nonempty directory rejected");

    assert!(matches!(error, JjError::WorkspacePathExists { .. }));
    assert_eq!(
        fs::read_to_string(options.destination.join("keep.txt")).expect("read local work"),
        "local work\n",
    );
    assert!(primary.join(".git/worktrees/on-duty").is_dir());
}

#[cfg(unix)]
#[test]
fn workspace_add_preserves_symlink_at_stale_git_worktree_destination() {
    let (_fixture, primary, options) = stale_git_worktree_fixture();
    std::os::unix::fs::symlink("missing-target", &options.destination).expect("create symlink");

    let error = run_jj_workspace_add(&primary, &options).expect_err("symlink rejected");

    assert!(matches!(error, JjError::WorkspacePathExists { .. }));
    assert_eq!(
        fs::read_link(&options.destination).expect("read symlink"),
        PathBuf::from("missing-target"),
    );
    assert!(primary.join(".git/worktrees/on-duty").is_dir());
}

#[test]
fn workspace_add_validates_shared_paths_before_recovering_stale_registration() {
    let (_fixture, primary, mut options) = stale_git_worktree_fixture();
    fs::write(primary.join("tracked.txt"), "tracked\n").expect("write tracked file");
    JjWorkspace::load_after_working_copy_snapshot(&primary).expect("snapshot primary workspace");
    options.shared_paths = vec!["tracked.txt".to_owned()];

    let error = run_jj_workspace_add(&primary, &options).expect_err("tracked shared path rejected");

    assert!(matches!(error, JjError::WorkspaceSharedPathsTracked { .. }));
    assert!(primary.join(".git/worktrees/on-duty").is_dir());
    assert!(!options.destination.exists());
}

#[test]
fn workspace_add_does_not_recover_workspace_still_tracked_by_jj() {
    let (_fixture, primary, options) = stale_git_worktree_fixture();
    run_jj_workspace_add(&primary, &options).expect("add workspace");
    fs::remove_dir_all(&options.destination).expect("delete workspace files");

    let error =
        run_jj_workspace_add(&primary, &options).expect_err("duplicate jj workspace rejected");

    assert!(matches!(error, JjError::WorkspaceAddFailed { .. }));
    assert!(primary.join(".git/worktrees/on-duty").is_dir());
    assert!(!options.destination.exists());
}

#[test]
fn workspace_add_cleans_empty_directory_after_git_failure() {
    let (_fixture, primary, options) = stale_git_worktree_fixture();
    run_jj_workspace_add(&primary, &options).expect("recover registration and add workspace");
    let other = WorkspaceAddOptions {
        name: "other".to_owned(),
        destination: options.destination.with_file_name("other"),
        ..options
    };
    fs::write(
        primary.join(".jj/repo/config.toml"),
        "[git]\ncolocate = true\nexecutable-path = '/nonexistent/jx-test-git'\n",
    )
    .expect("configure failing Git executable");

    let error = run_jj_workspace_add(&primary, &other).expect_err("Git executable fails");

    assert!(matches!(error, JjError::WorkspaceAddFailed { .. }));
    assert!(!other.destination.exists());
}

/// Recovers only the selected destination, leaving another stale registration intact.
fn assert_workspace_add_recovers_stale_git_worktree(existing_empty_directory: bool) {
    let (_fixture, primary, mut options) = stale_git_worktree_fixture();
    let unrelated = options.destination.with_file_name("unrelated");
    create_stale_git_worktree(&primary, &unrelated);
    if existing_empty_directory {
        fs::create_dir(&options.destination).expect("leave empty directory from failed add");
    }
    // Match Git's recorded path even when the request uses a different spelling.
    options.destination = primary.join("../.work/backend/on-duty");

    run_jj_workspace_add(&primary, &options).expect("recover stale registration and add workspace");

    assert!(options.destination.join(".git").is_file());
    assert!(options.destination.join(".jj").is_dir());
    assert!(primary.join(".git/worktrees/unrelated").is_dir());
    assert!(!unrelated.exists());
}

/// Initializes a colocated repository with a Git registration but no destination directory.
fn stale_git_worktree_fixture() -> (TestWorkspace, PathBuf, WorkspaceAddOptions) {
    let fixture = TestWorkspace::new("workspace-add-stale-registration");
    let primary = fixture.path().join("primary");
    fs::create_dir_all(&primary).expect("create primary workspace");
    let settings = user_settings().expect("settings");
    pollster::block_on(Workspace::init_colocated_git(
        &settings,
        &primary,
        gix::hash::Kind::Sha1,
    ))
    .expect("initialize colocated workspace");
    fs::write(
        primary.join(".jj/repo/config.toml"),
        "[git]\ncolocate = true\n",
    )
    .expect("enable colocation");
    let options = WorkspaceAddOptions {
        name: "on-duty".to_owned(),
        destination: fixture.path().join(".work/backend/on-duty"),
        revision: None,
        shared_paths: Vec::new(),
    };
    create_stale_git_worktree(&primary, &options.destination);
    (fixture, primary, options)
}

/// Simulates a Git worktree whose files were deleted without unregistering it.
fn create_stale_git_worktree(primary: &Path, destination: &Path) {
    fs::create_dir_all(destination.parent().expect("worktree parent")).expect("create parent");
    let workspace = JjWorkspace::load(primary).expect("load primary workspace");
    let subprocess_options =
        git::GitSubprocessOptions::from_settings(workspace.workspace.settings())
            .expect("Git settings");
    git::create_worktree(workspace.repo.store(), subprocess_options, destination)
        .expect("create Git worktree");
    fs::remove_dir_all(destination).expect("delete worktree files without unregistering");
}

#[test]
fn workspace_shared_path_preflight_rejects_tracked_exact_path_only() {
    // Verifies: Shared path preflight checks the exact configured path, not tracked parents.
    let fixture = TestWorkspace::new("shared-path-tracked-exact");
    let settings = user_settings().expect("settings");
    pollster::block_on(async {
        let (workspace, repo) =
            Workspace::init_internal_git(&settings, fixture.path(), gix::hash::Kind::Sha1)
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
        let (workspace, repo) =
            Workspace::init_internal_git(&settings, fixture.path(), gix::hash::Kind::Sha1)
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
