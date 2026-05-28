use super::*;

#[test]
fn stack_help_describes_cached_display_and_live_refresh() {
    // Verifies: Stack help distinguishes local cached display from GitHub-backed refresh.
    let help = help_output(["jx", "stack", "--help"]);

    assert!(help.contains("Show or refresh repo-local pull request stack state"));
    assert!(help.contains(".jx/stack.toml"));
    assert!(help.contains("without contacting GitHub"));
    assert!(help.contains("open GitHub PRs authored by you"));
    assert!(help.contains("show"));
    assert!(help.contains("refresh"));
    assert!(!help.contains("track"));
    assert!(!help.contains("reset"));
}

#[test]
fn stack_subcommand_help_explains_effects() {
    // Verifies: Stack subcommand help names data sources and non-mutating GitHub behavior.
    let show_help = help_output(["jx", "stack", "show", "--help"]);
    assert!(show_help.contains("Show stored pull request stack state"));
    assert!(show_help.contains("without contacting GitHub"));
    assert!(show_help.contains("default when no stack subcommand"));

    let refresh_help = help_output(["jx", "stack", "refresh", "--help"]);
    assert!(refresh_help.contains("Rebuild repo-local stack state"));
    assert!(refresh_help.contains("local PR bookmark heads"));
    assert!(refresh_help.contains("writes .jx/stack.toml"));
    assert!(refresh_help.contains("does not push branches"));
    assert!(refresh_help.contains("create, update, close, or delete pull requests"));
}

#[test]
fn stack_interactive_opens_selected_cached_pull_request() {
    // Verifies: Stack selection opens cached PR metadata without querying GitHub.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "example-user/old".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Older change".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "example-user/chosen".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(11),
                    parent_pull_request: None,
                    title: "Chosen change".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/11".to_owned()),
                    draft: true,
                    merged: false,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = RecordingPullRequestSelector::new(1);

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack open succeeds");

    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/example-repo/pull/11\n"
    );
    assert_eq!(
        selector.labels.borrow().as_slice(),
        &[vec![
            "◯ #10     Older change".to_owned(),
            "\x1b[2m\x1b[38;2;150;142;132m◌ #11     Chosen change\x1b[0m".to_owned(),
        ]]
    );
    assert_eq!(
        services.opened_urls.borrow().as_slice(),
        ["https://github.com/example-owner/example-repo/pull/11"]
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [None]
    );
    assert_eq!(services.pull_request_bookmark_calls.get(), 0);
    assert!(services
        .authored_open_pull_request_head_calls
        .borrow()
        .is_empty());
    assert!(services.pull_request_head_calls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_interactive_can_be_cancelled() {
    // Verifies: Quitting the stack selector stops without opening a browser.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "example-user/change".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Change".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = CancellingPullRequestSelector;

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
    assert!(services.opened_urls.borrow().is_empty());
}

#[test]
fn stack_interactive_shows_full_cached_stack_with_draft_rows() {
    // Verifies: Stack selection keeps draft PRs visible even when another branch is current.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "topic/root".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Root".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                    draft: false,
                    merged: true,
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/11".to_owned()),
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "topic/draft".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(12),
                    parent_pull_request: None,
                    title: "Draft".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/12".to_owned()),
                    draft: true,
                    merged: false,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        ..Default::default()
    };
    let selector = RecordingPullRequestSelector::new(1);

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack open succeeds");

    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/example-repo/pull/11\n"
    );
    assert_eq!(
        selector.labels.borrow().as_slice(),
        &[vec![
            "✓ #10     Root".to_owned(),
            "└─ ◉ #11     Child".to_owned(),
            "\x1b[2m\x1b[38;2;150;142;132m◌ #12     Draft\x1b[0m".to_owned(),
        ]]
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [None]
    );
    assert_eq!(services.pull_request_bookmark_calls.get(), 0);
    assert!(services
        .authored_open_pull_request_head_calls
        .borrow()
        .is_empty());
    assert!(services.pull_request_head_calls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_show_colored_rows_match_interactive_selector_labels() {
    // Verifies: Non-interactive stack output and interactive choices share row rendering.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "topic/ready".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Ready".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "topic/draft".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(11),
                    parent_pull_request: None,
                    title: "Draft".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/11".to_owned()),
                    draft: true,
                    merged: false,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
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

    let stack = run_with_args_and_progress(
        ["jx", "stack"],
        &environment,
        &FakeServices::default(),
        &NoProgress,
        prompts,
        OutputMode { color: true },
    )
    .expect("stack show succeeds");
    let selector = RecordingPullRequestSelector::new(0);
    run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i", "--print"],
        &environment,
        &FakeServices::default(),
        &selector,
    )
    .expect("interactive stack open succeeds");

    let stack_rows = stack.stdout.lines().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(selector.labels.borrow().as_slice(), &[stack_rows]);
}

#[test]
fn stack_interactive_prints_selected_pull_request_url() {
    // Verifies: --print suppresses browser launch after cached stack selection.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "example-user/selected".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(12),
                parent_pull_request: None,
                title: "Selected change".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = FixedPullRequestSelector { selected: 0 };

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i", "--print"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack open succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo/pull/12\n"
    );
    assert!(services.opened_urls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_interactive_opens_historical_cached_pull_requests() {
    // Verifies: Cached stack opening trusts stored PR identity instead of live authored filters.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "example-user/reused".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(40568),
                parent_pull_request: None,
                title: "Merged or unowned historical PR".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/40568".to_owned()),
                draft: false,
                merged: true,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = FixedPullRequestSelector { selected: 0 };

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack open succeeds");

    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/example-repo/pull/40568\n"
    );
    assert_eq!(
        services.opened_urls.borrow().as_slice(),
        ["https://github.com/example-owner/example-repo/pull/40568"]
    );
    assert_eq!(services.pull_request_bookmark_calls.get(), 0);
    assert!(services
        .authored_open_pull_request_head_calls
        .borrow()
        .is_empty());
    assert!(services.pull_request_head_calls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_interactive_reports_missing_stack_state() {
    // Verifies: Interactive stack opening is cache-only and reports empty metadata directly.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = FixedPullRequestSelector { selected: 0 };

    let error = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect_err("missing stack state is reported");

    assert!(matches!(
        error,
        CommandError::Workflow(WorkflowError::MissingLocalBookmarkPullRequests { repository })
            if repository == "example-owner/example-repo"
    ));
    assert!(services.open_pull_request_selectors.borrow().is_empty());
    assert_eq!(services.pull_request_bookmark_calls.get(), 0);
    assert!(services.pull_request_head_calls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_refresh_persists_hierarchy_and_ignore_rules() {
    // Verifies: stack refresh records PR hierarchy in repo-local ignored metadata.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec![
            "topic/child".to_owned(),
            "topic/root".to_owned(),
            "topic/draft".to_owned(),
        ],
        authored_open_pull_requests_by_head: BTreeMap::from([
            (
                "topic/root".to_owned(),
                pull_request_choice_record(10, "Root", "topic/root", "main", false),
            ),
            (
                "topic/child".to_owned(),
                pull_request_choice_record(11, "Child", "topic/child", "topic/root", false),
            ),
            (
                "topic/draft".to_owned(),
                pull_request_choice_record(12, "Draft", "topic/draft", "topic/root", true),
            ),
        ]),
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
        ["jx", "stack", "refresh"],
        &environment,
        &services,
        &progress,
        prompts,
        OutputMode::plain(),
    )
    .expect("stack refresh succeeds");

    assert_eq!(progress.messages(), ["Refreshing pull request stack…"]);
    assert!(progress.finished.get());
    assert_eq!(
        result.stdout,
        "◯ #10     Root\n├─ ◯ #11     Child\n└─ ◌ #12     Draft\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join(".jx/.gitignore")).expect("read gitignore"),
        "/.gitignore\n/workspace.toml\n/stack.toml\n"
    );
    let stack_file =
        fs::read_to_string(workspace.path().join(".jx/stack.toml")).expect("read stack state");
    assert!(stack_file.contains("pull_request = 10"));
    assert!(stack_file.contains("parent_branch = \"topic/root\""));
}

#[test]
fn stack_without_subcommand_shows_state() {
    // Verifies: bare `jx stack` uses the safe read-only stack view.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result =
        run_with_args_and_services(["jx", "stack"], &environment, &FakeServices::default())
            .expect("stack show succeeds");

    assert_eq!(result.stdout, "◯ #10     Root\n");
}

#[test]
fn stack_show_reads_primary_checkout_state_from_managed_workspace() {
    // Verifies: stack state is repo-local even when the command runs from a managed workspace.
    let workspace = TestWorkspace::new_under("projects/.work/jx/current");
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
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let primary = workspace.home.join("projects/jx");
    write_stack_metadata(
        &primary,
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let result = run_with_args_and_services(
        ["jx", "stack", "show"],
        &environment,
        &FakeServices::default(),
    )
    .expect("stack show succeeds");

    assert_eq!(result.stdout, "◯ #10     Root\n");
    assert!(!workspace.path().join(".jx/stack.toml").exists());
}

#[test]
fn stack_tracking_retains_missing_stored_ancestors() {
    // Verifies: disappeared parents remain in stack state while children are still tracked.
    let existing = StackMetadata {
        version: 1,
        nodes: vec![
            StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: true,
            },
            StackMetadataNode {
                branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                parent_branch: Some("topic/root".to_owned()),
                pull_request: Some(11),
                parent_pull_request: Some(10),
                title: "Old child".to_owned(),
                url: None,
                draft: false,
                merged: false,
            },
        ],
    };
    let child = pull_request_choice_record(11, "Child", "topic/child", "main", false);

    let metadata = stack_metadata_from_pull_requests(&[child], &existing);

    assert_eq!(
        stack_metadata_rows(&metadata.nodes),
        vec!["✓ #10     Root", "└─ ◯ #11     Child"]
    );
}

#[test]
fn stack_refresh_updates_missing_stored_ancestor_by_pull_request_number() {
    // Verifies: stack refresh updates disappeared parent metadata from its durable PR number.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Stale root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let root = PullRequestRecord {
        number: 10,
        title: "Merged root".to_owned(),
        body: None,
        head_branch: "deleted/root".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
        draft: false,
        merged: true,
    };
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/child".to_owned()],
        authored_open_pull_requests_by_head: BTreeMap::from([(
            "topic/child".to_owned(),
            pull_request_choice_record(11, "Child", "topic/child", "topic/root", false),
        )]),
        pull_requests_by_number: BTreeMap::from([(10, root)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "refresh"], &environment, &services)
        .expect("stack refresh succeeds");

    assert_eq!(result.stdout, "✓ #10     Merged root\n└─ ◯ #11     Child\n");
    assert_eq!(services.pull_request_number_calls.borrow().as_slice(), [10]);
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes[0].branch, "topic/root");
    assert_eq!(metadata.nodes[0].title, "Merged root");
    assert!(metadata.nodes[0].merged);
}

fn help_output<const N: usize>(args: [&str; N]) -> String {
    let error = cli()
        .try_get_matches_from(args)
        .expect_err("help exits before command execution");
    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
    error.to_string()
}
