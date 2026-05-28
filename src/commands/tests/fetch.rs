use super::*;

#[test]
fn fetch_renders_rebased_commit_details_like_sync() {
    // Verifies: Fetch uses the same rebased-commit detail section as sync.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "fetch"], &environment, &services)
        .expect("fetch succeeds");

    assert_eq!(
            result.stdout,
            "Fetched: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n\nRebased on origin/main:\n  default@  aaaabbbb -> ccccdddd  example change\n  default@  eeeeffff -> 12345678  follow-up change\n"
        );
}

#[test]
fn fetch_omits_rebase_section_when_no_commits_move() {
    // Verifies: Fetch omits the rebase detail section when no local jj work moved.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: false,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "fetch"], &environment, &services)
        .expect("fetch succeeds");

    assert_eq!(
            result.stdout,
            "Fetched: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n"
        );
}

#[test]
fn fetch_omits_rebase_section_when_repair_has_no_commit_detail() {
    // Verifies: Fetch matches sync by omitting details when repair produced no visible row.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 1,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: true,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "fetch"], &environment, &services)
        .expect("fetch succeeds");

    assert_eq!(
            result.stdout,
            "Fetched: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n"
        );
}

#[test]
fn f_alias_runs_fetch() {
    // Verifies: The short alias keeps the common fetch workflow quick to invoke.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "f"], &environment, &services)
        .expect("fetch alias succeeds");

    assert!(result.stdout.starts_with("Fetched: origin/main "));
}

#[test]
fn global_fetch_renderer_sorts_entries_by_directory() {
    // Verifies: Global fetch output follows filesystem order rather than discovery order.
    let entries = vec![
        GlobalFetchEntry {
            root: PathBuf::from("/workspace/src/beta"),
            display_root: "beta".to_owned(),
            result: Ok(()),
        },
        GlobalFetchEntry {
            root: PathBuf::from("/workspace/projects/alpha"),
            display_root: "alpha".to_owned(),
            result: Err("alpha failed".to_owned()),
        },
    ];

    let output = render_global_fetch(&entries, Path::new("/workspace"), false)
        .expect("global fetch renders");

    assert_eq!(
        output,
        "Fetched:\n  beta\n\nErrors:\n  alpha  alpha failed\n"
    );
}

#[test]
fn fetch_all_only_fetches_global_ready_repositories() {
    // Verifies: Global fetch mutates only repos whose working copy is safe to auto-fetch.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let ready = workspace.create_jj_workspace("projects/ready");
    let local_work = workspace.create_jj_workspace("projects/local-work");
    TestWorkspace::write_git_config_at(
        &ready,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/ready.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &local_work,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/local-work.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        global_fetch_ready_roots: Some(BTreeSet::from([ready.clone()])),
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: false,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let progress = RecordingProgress::default();
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };
    let result = run_with_args_and_progress(
        ["jx", "fetch", "--all"],
        &environment,
        &services,
        &progress,
        prompts,
        OutputMode::plain(),
    )
    .expect("global fetch succeeds");

    assert_eq!(
        progress.messages(),
        [
            "  0% Fetching local-work…",
            " 50% Fetching local-work…",
            " 50% Fetching ready…",
            "100% Fetching ready…",
        ]
    );
    assert!(progress.finished.get());
    assert_eq!(result.stdout, "Fetched:\n  ~/projects/ready\n");
    assert_eq!(services.fetch_origin_roots.borrow().as_slice(), [ready]);
}

#[test]
fn fetch_accepts_specific_repository_argument() {
    // Verifies: A positional project key fetches that repository with normal single-repo behavior.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let target = workspace.create_jj_workspace("projects/target");
    TestWorkspace::write_git_config_at(
        &target,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/target.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: false,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "fetch", "target"], &environment, &services)
        .expect("specific fetch succeeds");

    assert_eq!(services.fetch_origin_roots.borrow().as_slice(), [target]);
    assert_eq!(
        result.stdout,
        "Fetched: origin/main (\x1b]8;;https://github.com/example-owner/target/tree/main\x1b\\ssh://git@github.com/example-owner/target.git\x1b]8;;\x1b\\)\n"
    );
}

#[test]
fn f_alias_accepts_all_flag() {
    // Verifies: The short fetch alias supports global fetch ergonomics.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let ready = workspace.create_jj_workspace("projects/ready");
    TestWorkspace::write_git_config_at(
        &ready,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/ready.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 0,
            current_repaired: false,
            rebased_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "f", "-a"], &environment, &services)
        .expect("global fetch alias succeeds");

    assert_eq!(result.stdout, "Fetched:\n  ~/projects/ready\n");
}

#[test]
fn fetch_rejects_non_github_origin_with_actionable_error() {
    // Verifies: Fetch rejects non-GitHub origin URLs with an actionable error.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://example.invalid/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let error =
        run_with_args(["jx", "fetch"], &environment).expect_err("github origin is required");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::OriginNotGitHub { .. })
    ));
    assert!(error.to_string().contains("not a GitHub repository URL"));
}
