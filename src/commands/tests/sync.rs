use super::*;

fn blank_sync_pull_request_cell() -> String {
    " ".repeat(7)
}

fn sync_pull_request_cell(number: u64) -> String {
    let target = format!("#{number}");
    format!(
        "{}{}",
        example_pull_request_link(number),
        " ".repeat(7_usize.saturating_sub(target.chars().count()))
    )
}

fn sync_pull_request_status(
    number: u64,
    head_branch: &str,
    latest_commit_oid: &str,
) -> PullRequestStatusRecord {
    PullRequestStatusRecord {
        number,
        title: "Ready PR".to_owned(),
        url: Some(format!(
            "https://github.com/example-owner/example-repo/pull/{number}"
        )),
        created_at: None,
        head_branch: head_branch.to_owned(),
        base_branch: "main".to_owned(),
        default_branch: Some("main".to_owned()),
        author: Some("example-user".to_owned()),
        draft: false,
        merged: false,
        closed: false,
        merged_at: None,
        closed_at: None,
        check_status: PullRequestCheckStatus::Passing,
        checks: Vec::new(),
        merge_status: PullRequestMergeStatus::Mergeable,
        review_status: PullRequestReviewStatus::Approved,
        auto_merge_status: PullRequestAutoMergeStatus::NotConfigured,
        requested_reviewers: ReviewerSelection::default(),
        suggested_reviewers: Vec::new(),
        approved_reviewers: Vec::new(),
        changes_requested_reviewers: Vec::new(),
        commented_reviewers: Vec::new(),
        addressed_reviewers: Vec::new(),
        reviewer_responses: Vec::new(),
        reviewer_mentions: Vec::new(),
        dismissed_reviewers: Vec::new(),
        review_activity: Vec::new(),
        timeline_events: Vec::new(),
        labels: Vec::new(),
        latest_commit_oid: Some(latest_commit_oid.to_owned()),
    }
}

#[test]
fn global_sync_renderer_sorts_each_section_by_directory() {
    // Verifies: Global sync output keeps each status section in stable filesystem order.
    let entries = vec![
        GlobalSyncEntry {
            root: PathBuf::from("/workspace/src/zeta"),
            display_root: "zeta".to_owned(),
            outcome: GlobalSyncOutcome::Synced,
        },
        GlobalSyncEntry {
            root: PathBuf::from("/workspace/src/read-only"),
            display_root: "read-only".to_owned(),
            outcome: GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::ReadOnlyOrigin),
        },
        GlobalSyncEntry {
            root: PathBuf::from("/workspace/projects/alpha"),
            display_root: "alpha".to_owned(),
            outcome: GlobalSyncOutcome::Synced,
        },
        GlobalSyncEntry {
            root: PathBuf::from("/workspace/projects/read-only"),
            display_root: "projects-read-only".to_owned(),
            outcome: GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::ReadOnlyOrigin),
        },
    ];

    let output =
        render_global_sync(&entries, Path::new("/workspace"), false).expect("global sync renders");

    assert_eq!(
        output,
        "Synced:\n  alpha\n  zeta\n\nSkipped: read-only origin\n  projects-read-only\n  read-only\n"
    );
}

#[test]
fn global_sync_renderer_does_not_require_current_workspace_for_color_output() {
    // Verifies: All-repository output can render after running from a non-repository directory.
    let entries = vec![GlobalSyncEntry {
        root: PathBuf::from("/workspace/projects/alpha"),
        display_root: "alpha".to_owned(),
        outcome: GlobalSyncOutcome::Synced,
    }];

    let output = render_global_sync(&entries, Path::new("/not-a-workspace"), true)
        .expect("global sync renders without current workspace");

    assert_eq!(output, "Synced:\n  alpha\n");
}

#[test]
fn sync_pushes_same_tree_heads_by_default_unless_experimental_flag_is_set() {
    // Verifies: the same-tree push shortcut is opt-in so normal sync updates GitHub heads.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect("default sync succeeds");

    assert_eq!(
        services.sync_push_options.borrow().as_slice(),
        &[SyncPushOptions {
            skip_same_tree_pushes: false,
        }]
    );

    let services = FakeServices::default();
    run_with_args_and_services(
        ["jx", "sync", "--experimental-skip-same-tree-push"],
        &environment,
        &services,
    )
    .expect("experimental sync succeeds");

    assert_eq!(
        services.sync_push_options.borrow().as_slice(),
        &[SyncPushOptions {
            skip_same_tree_pushes: true,
        }]
    );
}

#[test]
fn sync_all_shorthand_syncs_writable_repositories_when_origin_does_not_need_pulling() {
    // Verifies: -a selects global sync and can push tracked state even with local jj work present.
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
    let local_work = workspace.create_jj_workspace("projects/local-work");
    let _missing_origin = workspace.create_jj_workspace("projects/missing-origin");
    let pull_needed = workspace.create_jj_workspace("projects/pull-needed");
    let read_only = workspace.create_jj_workspace("projects/read-only");
    let up_to_date = workspace.create_jj_workspace("projects/up-to-date");
    let writable = workspace.create_jj_workspace("projects/writable");
    for (root, name) in [
        (&local_work, "local-work"),
        (&pull_needed, "pull-needed"),
        (&read_only, "read-only"),
        (&up_to_date, "up-to-date"),
        (&writable, "writable"),
    ] {
        TestWorkspace::write_git_config_at(
            root,
            &format!(
                r#"
[remote "origin"]
    url = https://github.com/example-owner/{name}.git
"#,
            ),
        );
    }
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        global_fetch_ready_roots: Some(BTreeSet::new()),
        origin_push_access_roots: Some(BTreeSet::from([
            local_work.clone(),
            pull_needed.clone(),
            up_to_date.clone(),
            writable.clone(),
        ])),
        up_to_date_sync_roots: BTreeSet::from([up_to_date.clone()]),
        clean_status_repos: vec![
            "local-work".to_owned(),
            "up-to-date".to_owned(),
            "writable".to_owned(),
        ],
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            trunk: None,
            changed_remote_bookmarks: 0,
            changed_remote_tags: 0,
            abandoned_commits: 0,
            rebased_trunk_children: 0,
            rebased_descendants: 0,
            skipped_trunk_children: 1,
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
        ["jx", "sync", "-a"],
        &environment,
        &services,
        &progress,
        prompts,
        OutputMode::plain(),
    )
    .expect("global sync succeeds");

    assert_eq!(
        progress.messages(),
        [
            "  0% Syncing local-work…",
            " 16% Syncing local-work…",
            " 16% Syncing missing-origin…",
            " 33% Syncing missing-origin…",
            " 33% Syncing pull-needed…",
            " 50% Syncing pull-needed…",
            " 50% Syncing read-only…",
            " 66% Syncing read-only…",
            " 66% Syncing up-to-date…",
            " 83% Syncing up-to-date…",
            " 83% Syncing writable…",
            "100% Syncing writable…",
        ]
    );
    assert!(progress.finished.get());
    assert_eq!(
        services.fetch_origin_roots.borrow().as_slice(),
        [local_work.clone(), up_to_date.clone(), writable.clone()]
    );
    assert_eq!(
        services.push_tracked_roots.borrow().as_slice(),
        [local_work.clone(), up_to_date, writable]
    );
    assert_eq!(
        result.stdout,
        "Synced:\n  ~/projects/local-work\n  ~/projects/writable\n\nSkipped: up to date\n  ~/projects/up-to-date\n\nSkipped: pull needed\n  ~/projects/pull-needed  GitHub has 3 new commits\n\nSkipped: read-only origin\n  ~/projects/read-only\n\nSetup needed:\n  ~/projects/missing-origin  The fixed `origin` remote is missing. Add an `origin` GitHub remote before running `jx`.\n"
    );
}

#[test]
fn sync_all_retries_transient_fetch_failures_before_reporting_error() {
    // Verifies: Global sync rides out brief origin fetch transport failures before surfacing an error.
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
    let retrying = workspace.create_jj_workspace("projects/retrying");
    TestWorkspace::write_git_config_at(
        &retrying,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/retrying.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        origin_push_access_roots: Some(BTreeSet::from([retrying.clone()])),
        fetch_origin_failures_before_success: std::cell::Cell::new(2),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync", "--all"], &environment, &services)
        .expect("global sync succeeds after fetch retries");

    assert_eq!(
        services.fetch_origin_roots.borrow().as_slice(),
        [retrying.clone(), retrying.clone(), retrying.clone()]
    );
    assert_eq!(services.push_tracked_roots.borrow().as_slice(), [retrying]);
    assert_eq!(result.stdout, "Synced:\n  ~/projects/retrying\n");
}

#[test]
fn sync_all_configured_push_access_pushes_before_fetch() {
    // Verifies: Configured push access skips the live permission probe and avoids fetch-first transport.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[[repo.rules]]
repo = "example-owner/*"

[repo.rules.sync]
push_access = true
"#,
    );
    let trusted = workspace.create_jj_workspace("projects/trusted");
    TestWorkspace::write_git_config_at(
        &trusted,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/trusted.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        origin_push_access_roots: Some(BTreeSet::new()),
        clean_status_repos: vec!["trusted".to_owned()],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync", "--all"], &environment, &services)
        .expect("global sync succeeds through configured push access");

    assert!(services.fetch_origin_roots.borrow().is_empty());
    assert_eq!(services.push_tracked_roots.borrow().as_slice(), [trusted]);
    assert_eq!(result.stdout, "Synced:\n  ~/projects/trusted\n");
}

#[test]
fn sync_all_configured_push_access_fetches_once_after_push_rejection() {
    // Verifies: Push-first sync recovers stale remote refs with one fetch/rebase pass, not fetch retries.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[[repo.rules]]
repo = "example-owner/*"

[repo.rules.sync]
push_access = true
"#,
    );
    let trusted = workspace.create_jj_workspace("projects/trusted");
    TestWorkspace::write_git_config_at(
        &trusted,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/trusted.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        origin_push_access_roots: Some(BTreeSet::new()),
        clean_status_repos: vec!["trusted".to_owned()],
        push_rejections_before_success: std::cell::Cell::new(1),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync", "--all"], &environment, &services)
        .expect("global sync recovers rejected push");

    assert_eq!(
        services.fetch_origin_roots.borrow().as_slice(),
        std::slice::from_ref(&trusted)
    );
    assert_eq!(
        services.push_tracked_roots.borrow().as_slice(),
        [trusted.clone(), trusted]
    );
    assert_eq!(result.stdout, "Synced:\n  ~/projects/trusted\n");
}

#[test]
fn sync_all_filter_matches_owner_repo_suffix_in_provider_path() {
    // Verifies: `sync --all` filters match inside provider/owner/repo identities, not just compact keys.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[layout]
default_root = "~/projects"

[layout.default]
path = "{owner}/{repo}"
"#,
    );
    let selected = workspace.create_jj_workspace("projects/example-owner/foo");
    let skipped = workspace.create_jj_workspace("projects/other-owner/bar");
    for (root, owner, name) in [
        (&selected, "example-owner", "foo"),
        (&skipped, "other-owner", "bar"),
    ] {
        TestWorkspace::write_git_config_at(
            root,
            &format!(
                r#"
[remote "origin"]
    url = https://github.com/{owner}/{name}.git
"#,
            ),
        );
    }
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        origin_push_access_roots: Some(BTreeSet::from([selected.clone(), skipped])),
        clean_status_repos: vec!["foo".to_owned(), "bar".to_owned()],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "sync", "--all", "example-owner/*"],
        &environment,
        &services,
    )
    .expect("filtered global sync succeeds");

    assert_eq!(
        services.fetch_origin_roots.borrow().as_slice(),
        std::slice::from_ref(&selected)
    );
    assert_eq!(services.push_tracked_roots.borrow().as_slice(), [selected]);
    assert_eq!(result.stdout, "Synced:\n  ~/projects/example-owner/foo\n");
}

#[test]
fn sync_all_reports_local_work_when_tracked_push_has_nothing_to_sync() {
    // Verifies: Global sync does not call a repo up to date when unpushed jj work remains.
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
    let local_work = workspace.create_jj_workspace("projects/local-work");
    TestWorkspace::write_git_config_at(
        &local_work,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/local-work.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        origin_push_access_roots: Some(BTreeSet::from([local_work.clone()])),
        up_to_date_sync_roots: BTreeSet::from([local_work.clone()]),
        status: StatusReport {
            remotes: vec![domain::RemoteStatusReport {
                name: "origin".to_owned(),
                url: "https://github.com/example-owner/local-work.git".to_owned(),
                github_url: "https://github.com/example-owner/local-work".to_owned(),
                branch: "main".to_owned(),
                local_trunk_sha: "1111222233334444".to_owned(),
                local_trunk_short_sha: "11112222".to_owned(),
                local_ahead_by: 1,
                comparison: StatusComparison {
                    state: StatusState::UpToDate,
                    github_ahead_by: 0,
                    github_behind_by: 0,
                    counts_exact: true,
                },
            }],
            fork: None,
        },
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            trunk: None,
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

    let result = run_with_args_and_services(["jx", "sync", "--all"], &environment, &services)
        .expect("global sync succeeds");

    assert_eq!(
        services.fetch_origin_roots.borrow().as_slice(),
        std::slice::from_ref(&local_work)
    );
    assert_eq!(
        services.push_tracked_roots.borrow().as_slice(),
        [local_work]
    );
    assert_eq!(
        result.stdout,
        "Skipped: local work\n  ~/projects/local-work  working copy has 1 local change\n"
    );
}

#[test]
fn sync_all_fetches_pull_needed_repo_when_only_empty_working_copy_is_local() {
    // Verifies: Global sync may pull when jj has only its empty working-copy child locally.
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
    let pull_only = workspace.create_jj_workspace("projects/pull-only");
    TestWorkspace::write_git_config_at(
        &pull_only,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/pull-only.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        global_fetch_ready_roots: Some(BTreeSet::from([pull_only.clone()])),
        origin_push_access_roots: Some(BTreeSet::from([pull_only.clone()])),
        up_to_date_sync_roots: BTreeSet::from([pull_only.clone()]),
        fetch: FetchOutcome {
            branch: "main".to_owned(),
            trunk: None,
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

    let result = run_with_args_and_services(["jx", "sync", "--all"], &environment, &services)
        .expect("global sync succeeds");

    assert_eq!(
        services.fetch_origin_roots.borrow().as_slice(),
        std::slice::from_ref(&pull_only)
    );
    assert_eq!(services.push_tracked_roots.borrow().as_slice(), [pull_only]);
    assert_eq!(result.stdout, "Synced:\n  ~/projects/pull-only\n");
}

#[test]
fn sync_fetches_then_pushes_repository_tracked_state_with_commit_lists() {
    // Verifies: Bare sync uses repository policy, rendering rebases before pushed heads/deletions.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert_eq!(services.advance_trunk_calls.get(), 0);
    assert_eq!(
        services.push_tracked_roots.borrow().as_slice(),
        [workspace.path()]
    );
    assert!(services.push_syncable_revision_requests.borrow().is_empty());
    assert_eq!(
        result.stdout,
        format!(
            "Synced: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n\nRebased on origin/main:\n  default@  changeaa  example change\n  default@  changebb  follow-up change\n\nPushed commits:\n  Commit    PR       Title\n  changecc  {}  example change default@\n\nDeleted bookmarks:\n  {}: changedd obsolete example change\n",
            blank_sync_pull_request_cell(),
            example_bookmark_link("example-user/old")
        )
    );
}

#[test]
fn sync_protects_test_green_review_pending_root_and_syncs_only_changed_protected_bookmarks() {
    // Verifies: PR-aware sync avoids churn once policy-normalized tests pass, even while review is pending.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo.sync]
rebase_strategy = "stack_green_pull_requests"
rebase_needed_labels = ["rebase-needed"]

[repo.stack_status]
review_gate_checks = ["^review gate$"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let root_commit = "1111222233334444555566667777888899990000";
    let child_commit = "2222333344445555666677778888999900001111";
    let mut root_status = sync_pull_request_status(10, "topic/root", root_commit);
    root_status.check_status = PullRequestCheckStatus::Failing;
    root_status.review_status = PullRequestReviewStatus::ReviewRequired;
    root_status.checks = vec![
        PullRequestCheck {
            name: "ci/build".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
        PullRequestCheck {
            name: "review gate".to_owned(),
            status: PullRequestCheckStatus::Failing,
        },
    ];
    root_status.labels = vec![PullRequestLabel {
        name: "merge-queue".to_owned(),
        color: "0e8a16".to_owned(),
    }];
    let services = FakeServices {
        fetch: FetchOutcome {
            rebased_commits: Vec::new(),
            ..FakeServices::default().fetch
        },
        local_stack_branches: std::cell::RefCell::new(vec![vec![
            LocalStackBranch {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                title: "Root".to_owned(),
                commit_id: root_commit.to_owned(),
            },
            LocalStackBranch {
                branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                parent_branch: Some("topic/root".to_owned()),
                title: "Child".to_owned(),
                commit_id: child_commit.to_owned(),
            },
        ]]),
        authored_open_pull_requests_by_head: BTreeMap::from([(
            "topic/root".to_owned(),
            PullRequestRecord {
                number: 10,
                title: "Root".to_owned(),
                body: None,
                head_branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                draft: false,
                merged: false,
                reviewers: ReviewerSelection::default(),
            },
        )]),
        pull_request_statuses: BTreeMap::from([(10, root_status)]),
        tracked_push: TrackedPushOutcome {
            pushed_refs: 1,
            bookmarks: vec![
                PushedBookmarkSummary {
                    branch: "topic/root".to_owned(),
                    old_short_commit_id: Some("11112222".to_owned()),
                    new_short_commit_id: Some("11112222".to_owned()),
                    old_short_change_id: Some("rootold".to_owned()),
                    new_short_change_id: Some("rootold".to_owned()),
                    old_description: Some("Root".to_owned()),
                    new_description: Some("Root".to_owned()),
                    pull_request_description: Some("Root".to_owned()),
                    pull_request_base: Some("main".to_owned()),
                    new_workspace_visibility: current_workspace_visibility(),
                },
                PushedBookmarkSummary {
                    branch: "topic/child".to_owned(),
                    old_short_commit_id: Some("22223333".to_owned()),
                    new_short_commit_id: Some("33334444".to_owned()),
                    old_short_change_id: Some("childld".to_owned()),
                    new_short_change_id: Some("childnw".to_owned()),
                    old_description: Some("Old child".to_owned()),
                    new_description: Some("Child".to_owned()),
                    pull_request_description: Some("Child".to_owned()),
                    pull_request_base: Some("topic/root".to_owned()),
                    new_workspace_visibility: current_workspace_visibility(),
                },
            ],
            pushed_commits: vec![PushedCommitSummary {
                short_commit_id: "33334444".to_owned(),
                description: "Child".to_owned(),
            }],
        },
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert_eq!(
        services.fetch_options.borrow().as_slice(),
        &[FetchOptions {
            protected_rebase_roots: vec!["topic/root".to_owned()],
        }]
    );
    let pushes = services.sync_pull_request_pushes.borrow();
    assert_eq!(pushes.len(), 1);
    assert_eq!(
        pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.branch.as_str())
            .collect::<Vec<_>>(),
        ["topic/child"]
    );
}

#[test]
fn sync_stack_uses_green_root_rebase_protection() {
    // Verifies: sync -s uses the same protected-root fetch and metadata-sync rules as repo sync.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo.sync]
rebase_strategy = "stack_green_pull_requests"
rebase_needed_labels = ["rebase-needed"]
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
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
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let root_commit = "1111222233334444555566667777888899990000";
    let child_commit = "2222333344445555666677778888999900001111";
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        fetch: FetchOutcome {
            rebased_commits: Vec::new(),
            ..FakeServices::default().fetch
        },
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        pull_request_bookmarks: vec!["topic/root".to_owned(), "topic/child".to_owned()],
        local_stack_branches: std::cell::RefCell::new(vec![vec![
            LocalStackBranch {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                title: "Root".to_owned(),
                commit_id: root_commit.to_owned(),
            },
            LocalStackBranch {
                branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                parent_branch: Some("topic/root".to_owned()),
                title: "Child".to_owned(),
                commit_id: child_commit.to_owned(),
            },
        ]]),
        authored_open_pull_requests_by_head: BTreeMap::from([(
            "topic/root".to_owned(),
            PullRequestRecord {
                number: 10,
                title: "Root".to_owned(),
                body: None,
                head_branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                draft: false,
                merged: false,
                reviewers: ReviewerSelection::default(),
            },
        )]),
        pull_request_statuses: BTreeMap::from([(
            10,
            sync_pull_request_status(10, "topic/root", root_commit),
        )]),
        tracked_push: TrackedPushOutcome {
            pushed_refs: 1,
            bookmarks: vec![
                PushedBookmarkSummary {
                    branch: "topic/root".to_owned(),
                    old_short_commit_id: Some("11112222".to_owned()),
                    new_short_commit_id: Some("11112222".to_owned()),
                    old_short_change_id: Some("rootold".to_owned()),
                    new_short_change_id: Some("rootold".to_owned()),
                    old_description: Some("Root".to_owned()),
                    new_description: Some("Root".to_owned()),
                    pull_request_description: Some("Root".to_owned()),
                    pull_request_base: Some("main".to_owned()),
                    new_workspace_visibility: current_workspace_visibility(),
                },
                PushedBookmarkSummary {
                    branch: "topic/child".to_owned(),
                    old_short_commit_id: Some("22223333".to_owned()),
                    new_short_commit_id: Some("33334444".to_owned()),
                    old_short_change_id: Some("childld".to_owned()),
                    new_short_change_id: Some("childnw".to_owned()),
                    old_description: Some("Old child".to_owned()),
                    new_description: Some("Child".to_owned()),
                    pull_request_description: Some("Child".to_owned()),
                    pull_request_base: Some("topic/root".to_owned()),
                    new_workspace_visibility: current_workspace_visibility(),
                },
            ],
            pushed_commits: vec![PushedCommitSummary {
                short_commit_id: "33334444".to_owned(),
                description: "Child".to_owned(),
            }],
        },
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "sync", "-s"], &environment, &services)
        .expect("stack sync succeeds");

    assert_eq!(
        services.fetch_options.borrow().as_slice(),
        &[FetchOptions {
            protected_rebase_roots: vec!["topic/root".to_owned()],
        }]
    );
    let pushes = services.sync_pull_request_pushes.borrow();
    assert_eq!(pushes.len(), 1);
    assert_eq!(
        pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.branch.as_str())
            .collect::<Vec<_>>(),
        ["topic/child"]
    );
}

#[test]
fn sync_renders_updated_trunk_state() {
    // Verifies: Single-repo sync reports the local trunk target without a GitHub lookup.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let committed_at = chrono::Utc::now() - chrono::Duration::minutes(90);
    let services = FakeServices {
        fetch: FetchOutcome {
            trunk: Some(crate::jj::TrunkStateSummary {
                branch: "main".to_owned(),
                short_change_id: "trunkchg".to_owned(),
                short_commit_id: "abc12345".to_owned(),
                committed_at_unix_ms: committed_at.timestamp_millis(),
                description: "Update example trunk".to_owned(),
            }),
            rebased_commits: Vec::new(),
            ..FakeServices::default().fetch
        },
        tracked_push: TrackedPushOutcome {
            pushed_refs: 0,
            bookmarks: Vec::new(),
            pushed_commits: Vec::new(),
        },
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert_eq!(
        result.stdout,
        "Synced: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\nTrunk:  trunkchg  1 hour ago  Update example trunk\n"
    );
}

#[test]
fn sync_stack_pushes_current_pull_request_stack() {
    // Verifies: -s syncs every local bookmark in the tracked PR stack, not every repository bookmark.
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
            work_item_handler_runs: Vec::new(),
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
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
                StackMetadataNode {
                    branch: "topic/draft".to_owned(),
                    base_branch: "topic/child".to_owned(),
                    parent_branch: Some("topic/child".to_owned()),
                    pull_request: Some(12),
                    parent_pull_request: Some(11),
                    title: "Draft".to_owned(),
                    url: None,
                    draft: true,
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        pull_request_bookmarks: vec![
            "topic/root".to_owned(),
            "topic/child".to_owned(),
            "topic/draft".to_owned(),
            "other/topic".to_owned(),
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync", "-s"], &environment, &services)
        .expect("stack sync succeeds");

    assert_eq!(
        services.push_syncable_revision_requests.borrow().as_slice(),
        [
            Some("topic/root".to_owned()),
            Some("topic/child".to_owned()),
            Some("topic/draft".to_owned())
        ]
    );
    assert!(result.stdout.starts_with("Synced: origin/main ("));
}

#[test]
fn sync_stack_refreshes_stored_metadata_by_pull_request_number() {
    // Verifies: sync -s refreshes stack context metadata for durable PR nodes before syncing descriptions.
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
            work_item_handler_runs: Vec::new(),
            nodes: vec![
                StackMetadataNode {
                    branch: "topic/root".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Stale root".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
            ],
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
        reviewers: ReviewerSelection::default(),
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        pull_request_bookmarks: vec!["topic/child".to_owned()],
        pull_requests_by_number: BTreeMap::from([(10, root)]),
        tracked_push: TrackedPushOutcome {
            pushed_refs: 1,
            bookmarks: vec![PushedBookmarkSummary {
                branch: "topic/child".to_owned(),
                old_short_commit_id: Some("11112222".to_owned()),
                new_short_commit_id: Some("33334444".to_owned()),
                old_short_change_id: Some("changeoo".to_owned()),
                new_short_change_id: Some("changech".to_owned()),
                old_description: Some("old child".to_owned()),
                new_description: Some("Child".to_owned()),
                pull_request_description: Some("Child".to_owned()),
                pull_request_base: Some("topic/root".to_owned()),
                new_workspace_visibility: current_workspace_visibility(),
            }],
            pushed_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync", "-s"], &environment, &services)
        .expect("stack sync succeeds");

    assert!(result.stdout.starts_with("Synced: origin/main ("));
    assert_eq!(
        services.pull_request_number_calls.borrow().as_slice(),
        [10, 11]
    );
    let synced_metadata = services.sync_pull_request_metadata.borrow();
    assert_eq!(synced_metadata.len(), 1);
    assert_eq!(synced_metadata[0].nodes[0].branch, "topic/root");
    assert_eq!(synced_metadata[0].nodes[0].title, "Merged root");
    assert!(synced_metadata[0].nodes[0].merged);
}

#[test]
fn sync_stack_skips_completed_tree_without_pruning_cached_progress() {
    // Verifies: sync -s ignores completed stack trees while retaining recent merged context for status.
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
            work_item_handler_runs: Vec::new(),
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
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: None,
                    draft: false,
                    merged: true,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        pull_request_bookmarks: vec!["topic/root".to_owned(), "topic/child".to_owned()],
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "sync", "-s"], &environment, &services)
        .expect_err("completed stack is not syncable");

    assert!(matches!(
        error,
        CommandError::Workflow(error) if matches!(*error, WorkflowError::MissingPullRequest)
    ));
    assert!(services.push_syncable_revision_requests.borrow().is_empty());
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes.len(), 2);
    assert!(metadata.nodes.iter().all(|node| node.merged));
    assert!(workspace.path().join(".jx/stack.toml").exists());
}

#[test]
fn sync_preserves_completed_stack_tree_after_refresh() {
    // Verifies: sync refreshes completed PR stack trees without deleting cached progress context.
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
            work_item_handler_runs: Vec::new(),
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
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: None,
                    draft: false,
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        tracked_push: TrackedPushOutcome {
            pushed_refs: 0,
            bookmarks: Vec::new(),
            pushed_commits: Vec::new(),
        },
        pull_requests_by_number: BTreeMap::from([
            (
                10,
                PullRequestRecord {
                    number: 10,
                    title: "Root".to_owned(),
                    body: None,
                    head_branch: "deleted/root".to_owned(),
                    base_branch: "main".to_owned(),
                    html_url: Some(
                        "https://github.com/example-owner/example-repo/pull/10".to_owned(),
                    ),
                    draft: false,
                    merged: true,
                    reviewers: ReviewerSelection::default(),
                },
            ),
            (
                11,
                PullRequestRecord {
                    number: 11,
                    title: "Child".to_owned(),
                    body: None,
                    head_branch: "deleted/child".to_owned(),
                    base_branch: "deleted/root".to_owned(),
                    html_url: Some(
                        "https://github.com/example-owner/example-repo/pull/11".to_owned(),
                    ),
                    draft: false,
                    merged: true,
                    reviewers: ReviewerSelection::default(),
                },
            ),
        ]),
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.starts_with("Synced: origin/main ("));
    assert_eq!(
        services.pull_request_number_calls.borrow().as_slice(),
        [10, 11]
    );
    assert!(services.sync_pull_request_metadata.borrow().is_empty());
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(
        metadata
            .nodes
            .iter()
            .map(|node| (node.branch.as_str(), node.merged))
            .collect::<Vec<_>>(),
        vec![("topic/root", true), ("topic/child", true)]
    );
    assert!(workspace.path().join(".jx/stack.toml").exists());
}

#[test]
fn sync_accepts_revision_argument() {
    // Verifies: A positional argument selects one jj target instead of changing repository scope.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "sync", "example-user/current"],
        &environment,
        &services,
    )
    .expect("specific sync succeeds");

    assert_eq!(
        services.push_syncable_revision_requests.borrow().as_slice(),
        [Some("example-user/current".to_owned())]
    );
    assert!(result.stdout.starts_with("Synced: origin/main ("));
}

#[test]
fn sync_repo_shorthand_creates_missing_origin_repository_from_layout() {
    // Verifies: -r forces repository sync while preserving missing-origin bootstrap behavior.
    let workspace = TestWorkspace::new_under("work/example-repo");
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let target = InitialPublishTarget {
        commit_id: "a1b2c3d4e5f6".to_owned(),
        short_commit_id: "a1b2c3d4".to_owned(),
        description: "example change".to_owned(),
    };
    let services = FakeServices {
        initial_publish_target: target.clone(),
        expected_bootstrap: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            target,
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync", "-r"], &environment, &services)
        .expect("sync bootstrap succeeds");

    assert_eq!(services.create_repository_calls.get(), 1);
    assert_eq!(
            result.stdout,
            format!(
                "Created private {} repo\nPushed a1b2c3d4 to {}\nWorking copy now at bf4799d5 (empty)\n",
                osc8_link(
                    "https://github.com/example-owner/example-repo",
                    "git@github.com:example-owner/example-repo.git"
                ),
                osc8_link("https://github.com/example-owner/example-repo/tree/main", "main")
            )
        );
}

#[test]
fn sync_offers_layout_repository_initialization_before_bootstrap() {
    // Verifies: Missing-workspace sync initializes an inferred layout repo before bootstrap.
    let workspace = TestWorkspace::new_uninitialized_under("work/example-repo");
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    workspace.write_file("README.md", "hello\n");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let target = InitialPublishTarget {
        commit_id: "a1b2c3d4e5f6".to_owned(),
        short_commit_id: "a1b2c3d4".to_owned(),
        description: "example change".to_owned(),
    };
    let services = FakeServices {
        expected_init_repository: Some(workspace.path()),
        initial_publish_target: target.clone(),
        expected_bootstrap: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            target,
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect("sync bootstrap succeeds");

    assert_eq!(services.init_repository_calls.get(), 1);
    assert_eq!(services.create_repository_calls.get(), 1);
    assert_eq!(
        result.stdout,
        format!(
            "Created private {} repo\nPushed a1b2c3d4 to {}\nWorking copy now at bf4799d5 (empty)\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            ),
            osc8_link("https://github.com/example-owner/example-repo/tree/main", "main")
        )
    );
}

#[test]
fn sync_can_cancel_layout_repository_initialization() {
    // Verifies: Declining local initialization stops before jj, GitHub, or push mutation.
    let workspace = TestWorkspace::new_uninitialized_under("work/example-repo");
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [(
            "HOME".to_owned(),
            workspace.home.to_string_lossy().into_owned(),
        )],
    );
    let services = FakeServices {
        expected_init_repository: Some(workspace.path()),
        ..FakeServices::default()
    };
    let confirmer = FixedRepositoryInitializationConfirmer { confirmed: false };

    let result = run_with_args_and_repository_initialization_confirmer(
        ["jx", "sync"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("sync cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
    assert_eq!(services.init_repository_calls.get(), 0);
    assert_eq!(services.create_repository_calls.get(), 0);
    assert!(services.push_tracked_roots.borrow().is_empty());
}

#[test]
fn sync_refuses_uninitialized_directory_outside_configured_layout() {
    // Verifies: Sync only initializes directories whose GitHub identity is layout-derived.
    let workspace = TestWorkspace::new_uninitialized_under("misc/example-repo");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx", "sync", "--repo"], &environment, &services)
        .expect_err("off-layout directory is not initialized");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::LayoutPathNotMatched { .. })
    ));
    assert_eq!(services.init_repository_calls.get(), 0);
    assert_eq!(services.create_repository_calls.get(), 0);
}

#[test]
fn sync_prepares_undescribed_initial_commit_before_bootstrap() {
    // Verifies: Missing-origin sync describes a fresh initial commit before pushing main.
    let workspace = TestWorkspace::new_under("work/example-repo");
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let target = InitialPublishTarget {
        commit_id: "a1b2c3d4e5f6".to_owned(),
        short_commit_id: "a1b2c3d4".to_owned(),
        description: String::new(),
    };
    let prepared = InitialPublishTarget {
        commit_id: "111122223333".to_owned(),
        short_commit_id: "11112222".to_owned(),
        description: "initial commit".to_owned(),
    };
    let services = FakeServices {
        initial_publish_target: target,
        prepared_initial_publish_target: Some(prepared.clone()),
        expected_bootstrap: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            prepared.clone(),
        )),
        bootstrap_push: BootstrapPushOutcome {
            branch: "main".to_owned(),
            short_commit_id: prepared.short_commit_id.clone(),
            description: prepared.description.clone(),
            working_copy_short_commit_id: Some("bf4799d5".to_owned()),
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync", "--repo"], &environment, &services)
        .expect("sync bootstrap succeeds");

    assert_eq!(services.prepare_initial_publish_calls.get(), 1);
    assert_eq!(services.create_repository_calls.get(), 1);
    assert_eq!(
        result.stdout,
        format!(
            "Created private {} repo\nPushed 11112222 to {}\nWorking copy now at bf4799d5 (empty)\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            ),
            osc8_link("https://github.com/example-owner/example-repo/tree/main", "main")
        )
    );
}

#[test]
fn sync_can_cancel_missing_origin_repository_creation() {
    // Verifies: Declining repository creation stops before GitHub or jj mutation.
    let workspace = TestWorkspace::new_under("work/example-repo");
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let services = FakeServices::default();
    let confirmer = FixedRepositoryCreationConfirmer { confirmed: false };

    let result = run_with_args_and_repository_creation_confirmer(
        ["jx", "sync", "--repo"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("sync cancellation succeeds");

    assert_eq!(services.prepare_initial_publish_calls.get(), 0);
    assert_eq!(services.create_repository_calls.get(), 0);
    assert_eq!(result.stdout, "cancelled\n");
}

#[test]
fn sync_refuses_missing_origin_outside_configured_layout() {
    // Verifies: Repository bootstrap only runs when layout can infer the GitHub identity.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx", "sync", "--repo"], &environment, &services)
        .expect_err("off-layout repo cannot be bootstrapped");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::LayoutPathNotMatched { .. })
    ));
    assert_eq!(services.create_repository_calls.get(), 0);
}

#[test]
fn sync_advances_trunk_when_repo_policy_enables_it() {
    // Verifies: Bare sync runs repository trunk-advance preparation for matching repo policy.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
advance_trunk = true
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let committed_at = chrono::Utc::now() - chrono::Duration::hours(3);
    let services = FakeServices {
        fetch: FetchOutcome {
            rebased_commits: Vec::new(),
            ..FakeServices::default().fetch
        },
        advance_trunk: AdvanceTrunkOutcome {
            trunk: Some(crate::jj::TrunkStateSummary {
                branch: "main".to_owned(),
                short_change_id: "newtrunk".to_owned(),
                short_commit_id: "def67890".to_owned(),
                committed_at_unix_ms: committed_at.timestamp_millis(),
                description: "Publish example trunk".to_owned(),
            }),
            ..FakeServices::default().advance_trunk
        },
        tracked_push: TrackedPushOutcome {
            pushed_refs: 0,
            bookmarks: Vec::new(),
            pushed_commits: Vec::new(),
        },
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert_eq!(services.advance_trunk_calls.get(), 1);
    assert_eq!(
        result.stdout,
        "Synced: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\nTrunk:  newtrunk  3 hours ago  Publish example trunk\n"
    );
}

#[test]
fn sync_links_pull_requests_in_pushed_commit_table() {
    // Verifies: Sync shows pushed PRs inline while keeping deleted PR annotations secondary.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        sync_pull_requests: vec![
            PullRequestRecord {
                number: 1234,
                title: "current pull request".to_owned(),
                body: None,
                head_branch: "example-user/current".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some(
                    "https://github.com/example-owner/example-repo/pull/1234".to_owned(),
                ),
                draft: false,
                merged: false,
                reviewers: ReviewerSelection::default(),
            },
            PullRequestRecord {
                number: 1200,
                title: "old pull request".to_owned(),
                body: None,
                head_branch: "example-user/old".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some(
                    "https://github.com/example-owner/example-repo/pull/1200".to_owned(),
                ),
                draft: false,
                merged: false,
                reviewers: ReviewerSelection::default(),
            },
        ],
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.contains(&format!(
        "Pushed commits:\n  Commit    PR       Title\n  changecc  {}  current pull request default@\n",
        sync_pull_request_cell(1234)
    )));
    assert!(result.stdout.contains(&format!(
        "Deleted bookmarks:\n  {}: changedd obsolete example change\n  ↳ PR {}\n",
        example_bookmark_link("example-user/old"),
        example_pull_request_link(1200)
    )));
}

#[test]
fn sync_aligns_blank_pull_request_cells_and_deleted_bookmarks() {
    // Verifies: Sync preserves PR column space for pushed commits without known PRs.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut services = FakeServices::default();
    services.tracked_push.bookmarks = vec![
        PushedBookmarkSummary {
            branch: "b".to_owned(),
            old_short_commit_id: Some("11112222".to_owned()),
            new_short_commit_id: Some("22223333".to_owned()),
            old_short_change_id: Some("changebb".to_owned()),
            new_short_change_id: Some("changesh".to_owned()),
            old_description: Some("previous short branch".to_owned()),
            new_description: Some("short branch".to_owned()),
            pull_request_description: Some("short branch".to_owned()),
            pull_request_base: Some("main".to_owned()),
            new_workspace_visibility: current_workspace_visibility(),
        },
        PushedBookmarkSummary {
            branch: "long".to_owned(),
            old_short_commit_id: Some("22223333".to_owned()),
            new_short_commit_id: Some("33334444".to_owned()),
            old_short_change_id: Some("changesh".to_owned()),
            new_short_change_id: Some("changelg".to_owned()),
            old_description: Some("previous long branch".to_owned()),
            new_description: Some("long branch".to_owned()),
            pull_request_description: Some("long branch".to_owned()),
            pull_request_base: Some("main".to_owned()),
            new_workspace_visibility: current_workspace_visibility(),
        },
        PushedBookmarkSummary {
            branch: "old".to_owned(),
            old_short_commit_id: Some("44445555".to_owned()),
            new_short_commit_id: None,
            old_short_change_id: Some("changeod".to_owned()),
            new_short_change_id: None,
            old_description: Some("old branch".to_owned()),
            new_description: None,
            pull_request_description: None,
            pull_request_base: None,
            new_workspace_visibility: WorkspaceVisibility::default(),
        },
    ];

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.contains(&format!(
        "Pushed commits:\n  Commit    PR       Title\n  changesh  {}  short branch default@\n  changelg  {}  long branch default@\n",
        blank_sync_pull_request_cell(),
        blank_sync_pull_request_cell()
    )));
    assert!(result.stdout.contains(&format!(
        "Deleted bookmarks:\n  {}: changeod old branch\n",
        example_bookmark_link("old")
    )));
}

#[test]
fn sync_expands_and_aligns_workspace_rows() {
    // Verifies: Workspace labels define scan order while unowned rows keep commit columns aligned.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut services = FakeServices::default();
    services.fetch.rebased_commits = vec![
        RebasedCommitSummary {
            short_change_id: "changemt".to_owned(),
            old_short_commit_id: "aaaamult".to_owned(),
            new_short_commit_id: "bbbbmult".to_owned(),
            description: "multi workspace".to_owned(),
            has_conflict: false,
            is_empty: false,
            workspace_visibility: visible_in(&["default", "review"], true),
        },
        RebasedCommitSummary {
            short_change_id: "changeot".to_owned(),
            old_short_commit_id: "aaaaothr".to_owned(),
            new_short_commit_id: "bbbbothr".to_owned(),
            description: "other workspace".to_owned(),
            has_conflict: false,
            is_empty: false,
            workspace_visibility: visible_in(&["review"], false),
        },
        RebasedCommitSummary {
            short_change_id: "changecu".to_owned(),
            old_short_commit_id: "aaaacurr".to_owned(),
            new_short_commit_id: "bbbbcurr".to_owned(),
            description: "current workspace".to_owned(),
            has_conflict: false,
            is_empty: false,
            workspace_visibility: current_workspace_visibility(),
        },
        RebasedCommitSummary {
            short_change_id: "changenw".to_owned(),
            old_short_commit_id: "aaaanone".to_owned(),
            new_short_commit_id: "bbbbnone".to_owned(),
            description: "no workspace".to_owned(),
            has_conflict: false,
            is_empty: false,
            workspace_visibility: WorkspaceVisibility::default(),
        },
    ];
    services.tracked_push.bookmarks = Vec::new();

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.contains(
            "Rebased on origin/main:\n  default@  changemt  multi workspace\n  default@  changecu  current workspace\n  review@   changemt  multi workspace\n  review@   changeot  other workspace\n            changenw  no workspace\n"
        ));
}

#[test]
fn sync_omits_deleted_bookmark_section_when_none_were_deleted() {
    // Verifies: Sync only shows deleted bookmark details when tracked deletions were pushed.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut tracked_push = FakeServices::default().tracked_push;
    tracked_push
        .bookmarks
        .retain(|bookmark| bookmark.new_short_commit_id.is_some());
    let services = FakeServices {
        tracked_push,
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(result.stdout.contains(&format!(
        "Pushed commits:\n  Commit    PR       Title\n  changecc  {}  example change default@\n",
        blank_sync_pull_request_cell()
    )));
    assert!(!result.stdout.contains("Deleted bookmarks:"));
}

#[test]
fn sync_omits_empty_rebase_and_push_sections_when_only_deletions_changed() {
    // Verifies: Sync only renders sections whose underlying operation changed visible state.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut tracked_push = FakeServices::default().tracked_push;
    tracked_push
        .bookmarks
        .retain(|bookmark| bookmark.new_short_commit_id.is_none());
    let services = FakeServices {
        fetch: FetchOutcome {
            rebased_commits: Vec::new(),
            ..FakeServices::default().fetch
        },
        tracked_push,
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert!(!result.stdout.contains("Rebased on origin/main:"));
    assert!(!result.stdout.contains("Pushed commits:"));
    assert!(result.stdout.contains(&format!(
        "Deleted bookmarks:\n  {}:",
        example_bookmark_link("example-user/old")
    )));
}

#[test]
fn sync_renders_only_summary_when_nothing_changed() {
    // Verifies: A no-op sync remains glanceable by omitting empty detail sections.
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
            rebased_commits: Vec::new(),
            ..FakeServices::default().fetch
        },
        tracked_push: TrackedPushOutcome {
            bookmarks: Vec::new(),
            pushed_commits: Vec::new(),
            pushed_refs: 0,
        },
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "sync"], &environment, &services).expect("sync succeeds");

    assert_eq!(
            result.stdout,
            "Synced: origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\)\n"
        );
}

#[test]
fn sync_pushes_clean_bookmarks_and_reports_conflicted_skips() {
    // Verifies: Sync keeps pushing safe bookmark updates and reports conflicted ones with a failing exit code.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut fetch = FakeServices::default().fetch;
    fetch.rebased_commits[0].has_conflict = true;
    let services = FakeServices {
        fetch,
        sync_conflicted_bookmarks: vec![crate::jj::SkippedPushBookmarkSummary {
            branch: "example-user/conflicted".to_owned(),
            conflicted_commits: vec![crate::jj::ConflictedCommitSummary {
                short_commit_id: "ccccdddd".to_owned(),
                description: "example change".to_owned(),
                workspace_visibility: current_workspace_visibility(),
            }],
        }],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "sync"], &environment, &services)
        .expect("conflicted sync returns normal output");

    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.contains(&format!(
        "Pushed commits:\n  Commit    PR       Title\n  changecc  {}  example change default@\n",
        blank_sync_pull_request_cell()
    )));
    assert!(result.stdout.contains(&format!(
        "Skipped bookmarks with conflicts:\n  {}  ccccdddd  example change (conflicted)\n",
        example_bookmark_link("example-user/conflicted")
    )));
}
