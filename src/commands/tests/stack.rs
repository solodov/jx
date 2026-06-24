use super::*;

#[test]
fn stack_help_describes_cached_display_and_live_refresh() {
    // Verifies: Stack help distinguishes local cached display from GitHub-backed refresh.
    let help = help_output(["jx", "stack", "--help"]);

    assert!(help.contains(
        "Show, move, publish, status-check, or refresh repo-local pull request stack state"
    ));
    assert!(help.contains(".jx/stack.toml"));
    assert!(help.contains("without contacting GitHub"));
    assert!(help.contains("create or update pull requests"));
    assert!(help.contains("--onto"));
    assert!(help.contains("--trunk"));
    assert!(help.contains("--no-sync"));
    assert!(help.contains("show"));
    assert!(help.contains("refresh"));
    assert!(help.contains("status"));
    assert!(help.contains("plan"));
    assert!(help.contains("publish"));
    assert!(!help.contains("track"));
    assert!(!help.contains("reset"));
}

#[test]
fn stack_complete_reviewers_lists_configured_repo_reviewers() {
    // Verifies: Reviewer completion is repo-scoped and filtered while publish still accepts arbitrary reviewers.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[repo]
reviewers = ["base-reviewer"]

[[repo.rules]]
repo = "example-owner/*"
reviewers = ["area-reviewer"]

[[repo.rules]]
repo = "example-owner/example-repo"
reviewers = ["repo-reviewer", "ExampleOrg/platform"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "stack", "complete-reviewers", "--prefix", "repo-"],
        &environment,
        &services,
    )
    .expect("reviewer completion succeeds");

    assert_eq!(result.stdout, "repo-reviewer\n");
}

#[test]
fn stack_status_interactive_flags_parse_without_loading_dashboard() {
    // Verifies: stack status dashboard flags are represented in the typed request.
    let matches = cli()
        .try_get_matches_from([
            "jx",
            "stack",
            "status",
            "-a",
            "-i",
            "--refresh-seconds",
            "20",
            "api-*",
        ])
        .expect("stack status dashboard args parse");
    let request = CommandRequest::from_matches(&matches).expect("request builds");

    let CommandRequest::Stack(StackRequest::Status(request)) = request else {
        panic!("expected stack status request");
    };
    assert!(request.all);
    assert_eq!(request.repo_filters, vec!["api-*".to_owned()]);
    assert_eq!(request.parallelism, 8);
    assert_eq!(request.format, StackStatusFormat::Human);
    assert!(request.interactive);
    assert_eq!(request.refresh_seconds, 20);
}

#[test]
fn stack_status_interactive_rejects_json_format() {
    // Verifies: live terminal dashboards only support human output.
    let matches = cli()
        .try_get_matches_from(["jx", "stack", "status", "-i", "--format", "json"])
        .expect("CLI shape parses before request validation");
    let result = CommandRequest::from_matches(&matches);

    assert!(matches!(
        result.as_ref().map_err(clap::Error::kind),
        Err(clap::error::ErrorKind::ArgumentConflict)
    ));
}

#[test]
fn stack_status_renders_check_and_review_summary() {
    // Verifies: stack status fetches batched GitHub health for locally tracked PR numbers.
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
                stack_status_node(101, "topic/root", "main", "Root change", false),
                stack_status_node(102, "topic/child", "topic/root", "Child change", true),
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/root".to_owned(), "topic/child".to_owned()],
        pull_request_statuses: BTreeMap::from([
            (101, {
                let mut status = stack_status_record(
                    101,
                    "Root change",
                    "topic/root",
                    "main",
                    PullRequestCheckStatus::Passing,
                    PullRequestReviewStatus::Approved,
                    ReviewerSelection::default(),
                );
                status.labels = vec![
                    PullRequestLabel {
                        name: "bug".to_owned(),
                        color: "d73a4a".to_owned(),
                    },
                    PullRequestLabel {
                        name: "help wanted".to_owned(),
                        color: "008672".to_owned(),
                    },
                    PullRequestLabel {
                        name: "area: backend".to_owned(),
                        color: "0e8a16".to_owned(),
                    },
                ];
                status.approved_reviewers = vec!["reviewer-approved".to_owned()];
                status.review_activity = vec![PullRequestReviewActivity {
                    reviewer: "reviewer-approved".to_owned(),
                    reviewed_at: "2099-01-01T00:00:00Z".to_owned(),
                }];
                status
            }),
            (102, {
                let mut status = stack_status_record(
                    102,
                    "Child change",
                    "topic/child",
                    "topic/root",
                    PullRequestCheckStatus::Pending,
                    PullRequestReviewStatus::ReviewRequested,
                    ReviewerSelection::new(["reviewer-one"], ["platform"]),
                );
                status.draft = true;
                status.created_at = Some("2099-01-01T00:00:00Z".to_owned());
                status.suggested_reviewers = vec!["suggested-reviewer".to_owned()];
                status.labels = vec![PullRequestLabel {
                    name: "ui".to_owned(),
                    color: "fbca04".to_owned(),
                }];
                status
            }),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![101, 102]]
    );
    assert!(result.stdout.contains(&format!(
        "{}  ",
        osc8_link(
            "https://github.com/example-owner/example-repo",
            "example-owner/example-repo"
        )
    )));
    assert!(result.stdout.contains("(origin/main behind)"));
    assert!(result.stdout.contains("PR       Chk  Rev  Lag   Title"));
    assert!(result.stdout.contains(&format!(
        "{}  ✓    ✓    <1h   ◯ Root change [bug] [help wanted] [area:backend] reviewer-approved",
        stack_status_pull_request_cell(101)
    )));
    assert!(result.stdout.contains(&format!(
        "{}  ◷    ◷    <1h   └ ◌ Child change [ui] reviewer-one, team/platform, suggested-reviewer",
        stack_status_pull_request_cell(102)
    )));
    assert!(result
        .stdout
        .contains("Legend:\n  Title: ● merged, ◯ ready/closed, ◌ draft"));
    assert!(result.stdout.contains("labels/reviewers follow"));
    assert!(result
        .stdout
        .contains("Chk: ✓ passing, ✗ failing, ◷ pending"));
    assert!(result
        .stdout
        .contains("Rev: ✓ approved, ! changes requested, ◷ waiting"));
}

#[test]
fn stack_status_styles_review_wait_threshold() {
    // Verifies: configured review SLA styling highlights stale review waits without distracting fresh, draft, or merged rows.
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
[repo.stack_status]
review_wait_threshold = "4h"
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![
                stack_status_node(201, "topic/old", "main", "Old waiting", false),
                stack_status_node(202, "topic/fresh", "main", "Fresh waiting", false),
                stack_status_node(203, "topic/draft", "main", "Draft waiting", true),
                stack_status_node(204, "topic/merged", "main", "Merged change", false),
            ],
        },
    )
    .expect("stack metadata writes");
    let timestamp = |hours: i64| {
        (chrono::Utc::now() - chrono::Duration::hours(hours))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    let mut old = stack_status_record(
        201,
        "Old waiting",
        "topic/old",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::ReviewRequested,
        ReviewerSelection::new(["reviewer-one"], Vec::<String>::new()),
    );
    old.created_at = Some(timestamp(5));
    let mut fresh = stack_status_record(
        202,
        "Fresh waiting",
        "topic/fresh",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::ReviewRequested,
        ReviewerSelection::new(["reviewer-one"], Vec::<String>::new()),
    );
    fresh.created_at = Some(timestamp(5));
    fresh.timeline_events = vec![PullRequestTimelineEvent {
        kind: PullRequestTimelineEventKind::ReadyForReview,
        created_at: timestamp(1),
        reviewer: None,
    }];
    let mut draft = stack_status_record(
        203,
        "Draft waiting",
        "topic/draft",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::ReviewRequested,
        ReviewerSelection::new(["reviewer-one"], Vec::<String>::new()),
    );
    draft.created_at = Some(timestamp(10));
    draft.timeline_events = vec![PullRequestTimelineEvent {
        kind: PullRequestTimelineEventKind::ConvertToDraft,
        created_at: timestamp(6),
        reviewer: None,
    }];
    draft.draft = true;
    let mut merged = stack_status_record(
        204,
        "Merged change",
        "topic/merged",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    merged.created_at = Some(timestamp(7));
    merged.merged = true;
    merged.closed = true;
    let services = FakeServices {
        pull_request_bookmarks: vec![
            "topic/old".to_owned(),
            "topic/fresh".to_owned(),
            "topic/draft".to_owned(),
            "topic/merged".to_owned(),
        ],
        pull_request_statuses: BTreeMap::from([
            (201, old),
            (202, fresh),
            (203, draft),
            (204, merged),
        ]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let prompts = test_prompt_handlers();

    let result = run_with_args_and_progress(
        ["jx", "stack", "status"],
        &environment,
        &services,
        &NoProgress,
        prompts,
        OutputMode {
            color: true,
            terminal_width: None,
        },
    )
    .expect("colored stack status succeeds");

    assert!(result
        .stdout
        .contains("\x1b[1m\x1b[31m◷\x1b[0m    \x1b[1m\x1b[31m5h  \x1b[0m  ◯ Old waiting"));
    assert!(result
        .stdout
        .contains("\x1b[36m◷\x1b[0m    \x1b[2m1h  \x1b[0m  ◯ Fresh waiting"));
    assert!(result
        .stdout
        .contains("◷    \x1b[2m6h  \x1b[0m\x1b[2m\x1b[38;2;190;184;176m  ◌ Draft waiting"));
    assert!(result
        .stdout
        .contains("\x1b[32m✓\x1b[0m    \x1b[32m7h  \x1b[0m  \x1b[32m● Merged change\x1b[0m"));
}

#[test]
fn stack_status_applies_configured_work_item_fix_handler_on_merge_transition() {
    // Verifies: configured work-item side effects run only when stack status observes a fixing PR merge.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let marker = workspace.home.join("resolved-work-item.txt");
    workspace.write_home_file(
        ".config/jx/config.toml",
        &format!(
            r#"
[[repo.rules]]
repo = "example-owner/*"

[repo.rules.work_items]
apply_on_stack_status = true

[[repo.rules.work_item_handlers]]
id = "resolve-work"
on = "work_item.fixed"
command = ["sh", "-c", "printf '%s\\n%s\\n' \"$1\" \"$(pwd)\" > \"$2\"", "_", "{{work_id}}", "{}"]
"#,
            marker.display()
        ),
    );
    let mut node = stack_status_node(101, "topic/root", "main", "Root change", false);
    node.work_ids = vec!["ABC-123".to_owned()];
    node.fixes_work_ids = vec!["ABC-123".to_owned()];
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![node],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = stack_status_record(
        101,
        "Root change",
        "topic/root",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    status.merged = true;
    status.closed = true;
    status.merged_at = Some("2026-06-09T12:00:00Z".to_owned());
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/root".to_owned()],
        pull_request_statuses: BTreeMap::from([(101, status)]),
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    let marker_contents = std::fs::read_to_string(marker).expect("handler marker is written");
    let marker_lines = marker_contents.lines().collect::<Vec<_>>();
    assert_eq!(marker_lines[0], "ABC-123");
    assert_eq!(
        std::fs::canonicalize(marker_lines[1]).expect("handler cwd exists"),
        std::fs::canonicalize(workspace.path()).expect("workspace path exists")
    );
    let log = std::fs::read_to_string(
        workspace
            .home
            .join(".local/state/jx/jx-work-item-handlers.log"),
    )
    .expect("handler log is written");
    let events = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("handler log json"))
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["status"], "start");
    assert_eq!(events[1]["status"], "success");
    assert_eq!(events[0]["handler"], "resolve-work");
    assert_eq!(events[0]["workId"], "ABC-123");
    assert_eq!(events[0]["cwd"], workspace.path().display().to_string());
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(
        metadata.work_item_handler_runs,
        vec![StackMetadataWorkItemHandlerRun {
            handler: "resolve-work".to_owned(),
            work_id: "ABC-123".to_owned(),
            pull_request: 101,
        }]
    );
    assert!(
        !std::fs::read_to_string(workspace.path().join(".jx/.gitignore"))
            .expect("metadata gitignore is written")
            .contains("/work-item-handlers.log")
    );
}

#[test]
fn stack_status_reconciles_missing_work_item_fix_handler_ledger() {
    // Verifies: merged fixing PRs still apply configured side effects when stack metadata has no success ledger entry.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let marker = workspace.home.join("resolved-work-items.txt");
    workspace.write_home_file(
        ".config/jx/config.toml",
        &format!(
            r#"
[[repo.rules]]
repo = "example-owner/*"

[repo.rules.work_items]
apply_on_stack_status = true

[[repo.rules.work_item_handlers]]
id = "resolve-work"
on = "work_item.fixed"
command = ["sh", "-c", "printf '%s\\n' \"$1\" >> \"$2\"", "_", "{{work_id}}", "{}"]
"#,
            marker.display()
        ),
    );
    let mut node = stack_status_node(102, "topic/root", "main", "Root change", false);
    node.merged = true;
    node.work_ids = vec!["ABC-124".to_owned()];
    node.fixes_work_ids = vec!["ABC-124".to_owned()];
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![node],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = stack_status_record(
        102,
        "Root change",
        "topic/root",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    status.merged = true;
    status.closed = true;
    status.merged_at = Some("2026-06-09T12:00:00Z".to_owned());
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/root".to_owned()],
        pull_request_statuses: BTreeMap::from([(102, status)]),
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("first stack status succeeds");
    run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("second stack status succeeds");

    assert_eq!(
        std::fs::read_to_string(marker).expect("handler marker is written"),
        "ABC-124\n"
    );
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(
        metadata.work_item_handler_runs,
        vec![StackMetadataWorkItemHandlerRun {
            handler: "resolve-work".to_owned(),
            work_id: "ABC-124".to_owned(),
            pull_request: 102,
        }]
    );
}

#[test]
fn stack_status_renders_reviewer_display_names() {
    // Verifies: stack status keeps login-based facts but renders cached public names for humans.
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
            nodes: vec![stack_status_node(
                119,
                "topic/display-name",
                "main",
                "Display names",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/display-name".to_owned()],
        pull_request_statuses: BTreeMap::from([(
            119,
            stack_status_record(
                119,
                "Display names",
                "topic/display-name",
                "main",
                PullRequestCheckStatus::Passing,
                PullRequestReviewStatus::ReviewRequested,
                ReviewerSelection::new(["human-reviewer"], Vec::<String>::new()),
            ),
        )]),
        github_user_display_names: BTreeMap::from([(
            "human-reviewer".to_owned(),
            "Human Reviewer".to_owned(),
        )]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result.stdout.contains("Human Reviewer"));
    assert!(!result.stdout.contains("human-reviewer"));
}

#[test]
fn stack_status_rewrites_titles_before_rendering() {
    // Verifies: repo policy can normalize PR title prefixes before stack status display.
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
[[repo.rules]]
repo = "example-owner/*"

[[repo.rules.stack_status.title_rewrites]]
pattern = "^\\[([A-Z]+-[0-9]+)\\] (.+)$"
replace = "$1: $2"
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![stack_status_node(
                121,
                "topic/title-rewrite",
                "main",
                "Cached title",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/title-rewrite".to_owned()],
        pull_request_statuses: BTreeMap::from([(
            121,
            stack_status_record(
                121,
                "[TASK-123] Wire up synthetic endpoint",
                "topic/title-rewrite",
                "main",
                PullRequestCheckStatus::Passing,
                PullRequestReviewStatus::Approved,
                ReviewerSelection::default(),
            ),
        )]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result
        .stdout
        .contains("TASK-123: Wire up synthetic endpoint"));
    assert!(!result.stdout.contains("[TASK-123]"));
}

#[test]
fn stack_status_ellipsizes_long_titles_before_labels_and_reviewers() {
    // Verifies: PR titles follow the conventional short subject width while preserving row signals.
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
            nodes: vec![stack_status_node(
                120,
                "topic/title-width",
                "main",
                "Cached title",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let mut status = stack_status_record(
        120,
        "Implement a very long synthetic stack title that demonstrates the compact subject width convention",
        "topic/title-width",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::ReviewRequested,
        ReviewerSelection::new(["human-reviewer"], Vec::<String>::new()),
    );
    status.labels = vec![PullRequestLabel {
        name: "backend".to_owned(),
        color: "5319e7".to_owned(),
    }];
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/title-width".to_owned()],
        pull_request_statuses: BTreeMap::from([(120, status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result
        .stdout
        .contains("Implement a very long synthetic stack title that demonstrates the compa…"));
    assert!(result.stdout.contains("[backend] human-reviewer"));
    assert!(!result.stdout.contains("compact subject width convention"));
}

#[test]
fn stack_status_ellipsizes_rows_to_terminal_width() {
    // Verifies: long stack-status rows stay within the detected terminal width.
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
            nodes: vec![stack_status_node(
                119,
                "topic/long-row",
                "main",
                "Implement a very long synthetic stack feature title that must be truncated",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/long-row".to_owned()],
        pull_request_statuses: BTreeMap::from([(
            119,
            stack_status_record(
                119,
                "Implement a very long synthetic feature title that must be truncated",
                "topic/long-row",
                "main",
                PullRequestCheckStatus::Passing,
                PullRequestReviewStatus::Approved,
                ReviewerSelection::default(),
            ),
        )]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_progress(
        ["jx", "stack", "status"],
        &environment,
        &services,
        &NoProgress,
        test_prompt_handlers(),
        OutputMode::plain_with_width(64),
    )
    .expect("stack status succeeds");

    let row = result
        .stdout
        .lines()
        .find(|line| line.contains("#119"))
        .expect("stack row renders");
    assert!(row.contains('…'), "row: {row:?}\n{}", result.stdout);
    assert!(rendered_visible_width(row) <= 64, "row: {row:?}");
}

#[test]
fn stack_status_moves_configured_review_gate_failures_to_review_column() {
    // Verifies: repo policy can classify approval-gate checks separately from test health.
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
[[repo.rules]]
repo = "example-owner/example-repo"

[repo.rules.stack_status]
review_gate_checks = ["approval gate"]

[[repo.rules.stack_status.ignored_checks]]
name = "^ci/noisy-advisory$"

[[repo.rules.stack_status.ignored_labels]]
name = "generated-noise"

[[repo.rules.stack_status.ignored_reviewers]]
name = "ignored-bot"
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![stack_status_node(
                120,
                "topic/review-gate",
                "main",
                "Review gate change",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let mut status = stack_status_record(
        120,
        "Review gate change",
        "topic/review-gate",
        "main",
        PullRequestCheckStatus::Failing,
        PullRequestReviewStatus::NotReviewed,
        ReviewerSelection::new(["human-reviewer", "ignored-bot"], Vec::<String>::new()),
    );
    status.labels = vec![
        PullRequestLabel {
            name: "useful-label".to_owned(),
            color: "0e8a16".to_owned(),
        },
        PullRequestLabel {
            name: "generated-noise".to_owned(),
            color: "5319e7".to_owned(),
        },
    ];
    status.commented_reviewers = vec!["ignored-bot".to_owned()];
    status.checks = vec![
        // GitHub rollups can include stale duplicate contexts; the latest matching name wins.
        PullRequestCheck {
            name: "approval gate".to_owned(),
            status: PullRequestCheckStatus::Failing,
        },
        PullRequestCheck {
            name: "ci/build".to_owned(),
            status: PullRequestCheckStatus::Failing,
        },
        PullRequestCheck {
            name: "ci/noisy-advisory".to_owned(),
            status: PullRequestCheckStatus::Failing,
        },
        PullRequestCheck {
            name: "ci/build".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
    ];
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/review-gate".to_owned()],
        pull_request_statuses: BTreeMap::from([(120, status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result.stdout.contains(&format!(
        "{}  ✓    ◷    —     ◯ Review gate change [useful-label] human-reviewer",
        stack_status_pull_request_cell(120)
    )));
    assert!(!result.stdout.contains("generated-noise"));
    assert!(!result.stdout.contains("ignored-bot"));
}

#[test]
fn stack_status_counts_passing_review_gate_checks_as_approved() {
    // Verifies: repo-defined gate checks can make the review column green without encoding repo-specific names in code.
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
[[repo.rules]]
repo = "example-owner/example-repo"

[repo.rules.stack_status]
review_gate_checks = ["approval gate", "committer gate"]
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![stack_status_node(
                121,
                "topic/review-gate-approved",
                "main",
                "Review gate approved",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let mut status = stack_status_record(
        121,
        "Review gate approved",
        "topic/review-gate-approved",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::NotReviewed,
        ReviewerSelection::default(),
    );
    status.checks = vec![
        PullRequestCheck {
            name: "approval gate".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
        PullRequestCheck {
            name: "committer gate".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
        PullRequestCheck {
            name: "ci/build".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
    ];
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/review-gate-approved".to_owned()],
        pull_request_statuses: BTreeMap::from([(121, status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result.stdout.contains(&format!(
        "{}  ✓    ✓    —     ◯ Review gate approved",
        stack_status_pull_request_cell(121)
    )));
}

#[test]
fn stack_status_preserves_github_review_required_with_passing_review_gate_checks() {
    // Verifies: protected human approval is not hidden by passing
    // repo-specific gates.
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
[[repo.rules]]
repo = "example-owner/example-repo"

[repo.rules.stack_status]
review_gate_checks = ["approval gate", "committer gate"]
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![stack_status_node(
                122,
                "topic/review-required",
                "main",
                "Review required",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let mut status = stack_status_record(
        122,
        "Review required",
        "topic/review-required",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::ReviewRequired,
        ReviewerSelection::new(["human-reviewer"], Vec::<String>::new()),
    );
    status.checks = vec![
        PullRequestCheck {
            name: "approval gate".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
        PullRequestCheck {
            name: "committer gate".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
        PullRequestCheck {
            name: "ci/build".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
    ];
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/review-required".to_owned()],
        pull_request_statuses: BTreeMap::from([(122, status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result.stdout.contains(&format!(
        "{}  ✓    ?    —     ◯ Review required human-reviewer",
        stack_status_pull_request_cell(122)
    )));
}

#[test]
fn stack_status_uses_latest_contexts_when_rollup_has_stale_failure() {
    // Verifies: stale duplicate GitHub contexts do not keep Chk failing after a newer success.
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
            nodes: vec![stack_status_node(
                121,
                "topic/stale-rollup",
                "main",
                "Stale rollup change",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let mut status = stack_status_record(
        121,
        "Stale rollup change",
        "topic/stale-rollup",
        "main",
        PullRequestCheckStatus::Failing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    status.checks = vec![
        PullRequestCheck {
            name: "Trunk Runner".to_owned(),
            status: PullRequestCheckStatus::Failing,
        },
        PullRequestCheck {
            name: "Trunk Runner".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
        PullRequestCheck {
            name: "ci/build".to_owned(),
            status: PullRequestCheckStatus::Pending,
        },
    ];
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/stale-rollup".to_owned()],
        pull_request_statuses: BTreeMap::from([(121, status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result.stdout.contains(&format!(
        "{}  ◷    ✓    —     ◯ Stale rollup change",
        stack_status_pull_request_cell(121)
    )));
}

#[test]
fn stack_status_resolves_branch_only_stack_nodes_before_fetching_status() {
    // Verifies: status repairs stack entries that know only the local branch, not the PR number.
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
            nodes: vec![StackMetadataNode {
                branch: "topic/branch-only-status".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: None,
                parent_pull_request: None,
                title: "Example branch-only status".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            }],
        },
    )
    .expect("stack metadata writes");
    let mut status = stack_status_record(
        451,
        "Example branch-only status",
        "topic/branch-only-status",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::ReviewRequested,
        ReviewerSelection::new(["example-reviewer"], std::iter::empty::<&str>()),
    );
    status.draft = true;
    let services = FakeServices {
        pull_requests_by_head: BTreeMap::from([(
            "topic/branch-only-status".to_owned(),
            pull_request_choice_record(
                451,
                "Example branch-only status",
                "topic/branch-only-status",
                "main",
                true,
            ),
        )]),
        pull_request_statuses: BTreeMap::from([(451, status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert_eq!(
        services.pull_request_head_calls.borrow().as_slice(),
        ["topic/branch-only-status"]
    );
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![451]]
    );
    assert!(result.stdout.contains(&format!(
        "{}  ✓    ◷    —     ◌ Example branch-only status example-reviewer",
        stack_status_pull_request_cell(451)
    )));
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes[0].pull_request, Some(451));
}

#[test]
fn stack_status_resolves_merged_branch_only_stack_nodes() {
    // Verifies: merged PRs can reattach their PR number even when only the local branch was cached.
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
            nodes: vec![StackMetadataNode {
                branch: "topic/merged-branch-only".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: None,
                parent_pull_request: None,
                title: "Merged branch-only status".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            }],
        },
    )
    .expect("stack metadata writes");
    let mut pull_request = pull_request_choice_record(
        452,
        "Merged branch-only status",
        "topic/merged-branch-only",
        "main",
        false,
    );
    pull_request.merged = true;
    let mut status = stack_status_record(
        452,
        "Merged branch-only status",
        "topic/merged-branch-only",
        "main",
        PullRequestCheckStatus::Failing,
        PullRequestReviewStatus::ChangesRequested,
        ReviewerSelection::default(),
    );
    status.merged = true;
    status.closed = true;
    status.merged_at = Some(chrono::Utc::now().to_rfc3339());
    let services = FakeServices {
        pull_requests_by_head: BTreeMap::from([(
            "topic/merged-branch-only".to_owned(),
            pull_request,
        )]),
        pull_request_statuses: BTreeMap::from([(452, status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(services.open_pull_request_head_calls.borrow().is_empty());
    assert_eq!(
        services.pull_request_head_calls.borrow().as_slice(),
        ["topic/merged-branch-only"]
    );
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![452]]
    );
    assert!(result.stdout.contains(&format!(
        "{}  ✓    ✓    —     ● Merged branch-only status",
        stack_status_pull_request_cell(452)
    )));
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes[0].pull_request, Some(452));
    assert!(metadata.nodes[0].merged);
}

#[test]
fn stack_status_colorizes_labels_with_github_backgrounds() {
    // Verifies: colored stack status renders labels as GitHub-colored chips.
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
[repo.stack_status]
ignored_labels_when_merged = ["auto-merge", "run-ci"]
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![
                stack_status_node(110, "topic/labeled", "main", "Labeled change", false),
                stack_status_node(111, "topic/draft-label", "main", "Draft label", true),
                stack_status_node(112, "topic/merged", "main", "Merged change", false),
            ],
        },
    )
    .expect("stack metadata writes");
    let mut ready_status = stack_status_record(
        110,
        "Labeled change",
        "topic/labeled",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    ready_status.labels = vec![
        PullRequestLabel {
            name: "bug".to_owned(),
            color: "000000".to_owned(),
        },
        PullRequestLabel {
            name: "docs".to_owned(),
            color: "fbca04".to_owned(),
        },
        PullRequestLabel {
            name: "run-ci".to_owned(),
            color: "0e8a16".to_owned(),
        },
        PullRequestLabel {
            name: "pink".to_owned(),
            color: "d73a4a".to_owned(),
        },
        PullRequestLabel {
            name: "area: backend".to_owned(),
            color: "5319e7".to_owned(),
        },
    ];
    ready_status.requested_reviewers = ReviewerSelection::new(
        [
            "reviewer-pending",
            "reviewer-commented-requested",
            "reviewer-addressed",
        ],
        std::iter::empty::<&str>(),
    );
    ready_status.approved_reviewers = vec![
        "reviewer-approved".to_owned(),
        "reviewer-commented-approved".to_owned(),
    ];
    ready_status.commented_reviewers = vec![
        "reviewer-commented-requested".to_owned(),
        "reviewer-commented".to_owned(),
        "reviewer-commented-approved".to_owned(),
    ];
    ready_status.addressed_reviewers = vec!["reviewer-addressed".to_owned()];
    let mut draft_status = stack_status_record(
        111,
        "Draft label",
        "topic/draft-label",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    draft_status.draft = true;
    draft_status.labels = vec![PullRequestLabel {
        name: "ui".to_owned(),
        color: "d73a4a".to_owned(),
    }];
    draft_status.requested_reviewers =
        ReviewerSelection::new(["draft-pending"], std::iter::empty::<&str>());
    draft_status.approved_reviewers = vec!["draft-approved".to_owned()];
    let mut merged_status = stack_status_record(
        112,
        "Merged change",
        "topic/merged",
        "main",
        PullRequestCheckStatus::Failing,
        PullRequestReviewStatus::ChangesRequested,
        ReviewerSelection::new(["obsolete-reviewer"], std::iter::empty::<&str>()),
    );
    merged_status.merged = true;
    merged_status.closed = true;
    merged_status.merged_at = Some(chrono::Utc::now().to_rfc3339());
    merged_status.labels = vec![
        PullRequestLabel {
            name: "done".to_owned(),
            color: "0e8a16".to_owned(),
        },
        PullRequestLabel {
            name: "run-ci".to_owned(),
            color: "d73a4a".to_owned(),
        },
        PullRequestLabel {
            name: "auto-merge".to_owned(),
            color: "fbca04".to_owned(),
        },
    ];
    merged_status.approved_reviewers = vec![
        "merged-approved".to_owned(),
        "merged-commented-approved".to_owned(),
    ];
    merged_status.commented_reviewers = vec![
        "merged-commented".to_owned(),
        "merged-commented-approved".to_owned(),
    ];
    merged_status.addressed_reviewers = vec!["merged-addressed".to_owned()];
    let services = FakeServices {
        pull_request_bookmarks: vec![
            "topic/labeled".to_owned(),
            "topic/draft-label".to_owned(),
            "topic/merged".to_owned(),
        ],
        pull_request_statuses: BTreeMap::from([
            (110, ready_status),
            (111, draft_status),
            (112, merged_status),
        ]),
        ..FakeServices::default()
    };
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

    let result = run_with_args_and_progress(
        ["jx", "stack", "status"],
        &environment,
        &services,
        &NoProgress,
        prompts,
        OutputMode {
            color: true,
            terminal_width: None,
        },
    )
    .expect("colored stack status succeeds");

    assert!(result.stdout.contains(
        "\x1b[48;2;0;0;0m\x1b[38;2;255;255;255m bug \x1b[0m\x1b[48;2;251;202;4m\x1b[38;2;0;0;0m docs \x1b[0m\x1b[48;2;14;138;22m\x1b[38;2;255;255;255m run-ci \x1b[0m\x1b[48;2;215;58;74m\x1b[38;2;0;0;0m pink \x1b[0m\x1b[48;2;83;25;231m\x1b[38;2;255;255;255m area:backend \x1b[0m"
    ));
    assert!(result
        .stdout
        .contains("\x1b[1m\x1b[30mreviewer-pending\x1b[0m"));
    assert!(result
        .stdout
        .contains("\x1b[38;2;194;95;0mreviewer-commented-requested\x1b[0m"));
    assert!(result
        .stdout
        .contains("\x1b[38;2;194;95;0mreviewer-commented\x1b[0m"));
    assert!(result
        .stdout
        .contains("\x1b[3m\x1b[30mreviewer-addressed\x1b[0m"));
    assert!(result.stdout.contains("\x1b[32mreviewer-approved\x1b[0m"));
    assert!(result
        .stdout
        .contains("\x1b[3m\x1b[32mreviewer-commented-approved\x1b[0m"));
    assert!(result.stdout.contains(
        "\x1b[48;2;246;237;234m\x1b[38;2;190;184;176m ui \x1b[0m\x1b[2m\x1b[38;2;190;184;176m draft-pending, draft-approved"
    ));
    assert!(result.stdout.contains("\x1b[32m#112\x1b[0m"));
    assert!(result.stdout.contains("\x1b[32m● Merged change\x1b[0m"));
    assert!(result.stdout.contains(
        "\x1b[32m● Merged change\x1b[0m \x1b[48;2;236;241;231m\x1b[38;2;190;184;176m done \x1b[0m"
    ));
    let merged_line = result
        .stdout
        .lines()
        .find(|line| line.contains("Merged change"))
        .expect("merged row renders");
    assert!(!merged_line.contains("run-ci"));
    assert!(!merged_line.contains("auto-merge"));
    assert!(result
        .stdout
        .contains("\x1b[38;2;194;95;0mmerged-commented\x1b[0m"));
    assert!(result
        .stdout
        .contains("\x1b[38;2;118;108;96mmerged-approved\x1b[0m"));
    assert!(result
        .stdout
        .contains("\x1b[38;2;118;108;96mmerged-commented-approved\x1b[0m"));
    assert!(!result.stdout.contains("obsolete-reviewer"));
    assert!(!result.stdout.contains("merged-addressed"));
    assert!(result
        .stdout
        .contains("\x1b[2m\x1b[38;2;190;184;176mLegend:"));
    assert!(!result.stdout.contains("[ui]"));
    assert!(!result.stdout.contains("\x1b[1m\x1b[30mdraft-pending"));
    assert!(!result.stdout.contains("\x1b[32mdraft-approved"));
}

#[test]
fn stack_status_shows_recently_merged_rows_as_progress_markers() {
    // Verifies: status output keeps recently merged PRs visible as non-actionable progress context.
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
                stack_status_node(201, "merged/done", "main", "Fully merged change", false),
                stack_status_node(202, "mixed/root", "main", "Merged ancestor", false),
                stack_status_node(203, "mixed/child", "mixed/root", "Open child", false),
            ],
        },
    )
    .expect("stack metadata writes");
    let merged_at = chrono::Utc::now().to_rfc3339();
    let mut fully_merged = stack_status_record(
        201,
        "Fully merged change",
        "merged/done",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    fully_merged.merged = true;
    fully_merged.closed = true;
    fully_merged.merged_at = Some(merged_at.clone());
    let mut merged_ancestor = stack_status_record(
        202,
        "Merged ancestor",
        "mixed/root",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    merged_ancestor.merged = true;
    merged_ancestor.closed = true;
    merged_ancestor.merged_at = Some(merged_at);
    let services = FakeServices {
        pull_request_statuses: BTreeMap::from([
            (201, fully_merged),
            (202, merged_ancestor),
            (
                203,
                stack_status_record(
                    203,
                    "Open child",
                    "mixed/child",
                    "mixed/root",
                    PullRequestCheckStatus::Passing,
                    PullRequestReviewStatus::Approved,
                    ReviewerSelection::default(),
                ),
            ),
        ]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result.stdout.contains("● Fully merged change"));
    assert!(result.stdout.contains("● Merged ancestor"));
    assert!(result.stdout.contains("◯ Open child"));
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(
        metadata
            .nodes
            .iter()
            .map(|node| (node.branch.as_str(), node.merged))
            .collect::<Vec<_>>(),
        vec![
            ("merged/done", true),
            ("mixed/root", true),
            ("mixed/child", false),
        ]
    );
}

#[test]
fn stack_status_keeps_recently_closed_rows_as_reminders() {
    // Verifies: recently closed PRs remain visible as non-actionable reminder rows.
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
                stack_status_node(301, "closed/root", "main", "Closed root", false),
                stack_status_node(302, "open/child", "closed/root", "Open child", false),
            ],
        },
    )
    .expect("stack metadata writes");
    let mut closed = stack_status_record(
        301,
        "Closed root",
        "closed/root",
        "main",
        PullRequestCheckStatus::Failing,
        PullRequestReviewStatus::ReviewRequired,
        ReviewerSelection::default(),
    );
    closed.closed = true;
    closed.closed_at = Some((chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339());
    let services = FakeServices {
        pull_request_statuses: BTreeMap::from([
            (301, closed),
            (
                302,
                stack_status_record(
                    302,
                    "Open child",
                    "open/child",
                    "closed/root",
                    PullRequestCheckStatus::Passing,
                    PullRequestReviewStatus::Approved,
                    ReviewerSelection::default(),
                ),
            ),
        ]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result.stdout.contains("◯ Closed root"));
    assert!(result.stdout.contains("Open child"));
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes.len(), 2);
    assert_eq!(metadata.nodes[0].branch, "closed/root");
    assert_eq!(metadata.nodes[1].branch, "open/child");
    assert_eq!(
        metadata.nodes[1].parent_branch.as_deref(),
        Some("closed/root")
    );
    assert_eq!(metadata.nodes[1].parent_pull_request, None);
}

#[test]
fn stack_status_json_renders_machine_readable_pull_request_health() {
    // Verifies: JSON output exposes stable labels for scripting without terminal formatting.
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
            nodes: vec![stack_status_node(
                103,
                "topic/json",
                "main",
                "JSON change",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/json".to_owned()],
        pull_request_statuses: BTreeMap::from([(103, {
            let mut status = stack_status_record(
                103,
                "JSON change",
                "topic/json",
                "main",
                PullRequestCheckStatus::Failing,
                PullRequestReviewStatus::ChangesRequested,
                ReviewerSelection::default(),
            );
            status.labels = vec![PullRequestLabel {
                name: "bug".to_owned(),
                color: "d73a4a".to_owned(),
            }];
            status.commented_reviewers = vec!["reviewer-commented".to_owned()];
            status.addressed_reviewers = vec!["reviewer-addressed".to_owned()];
            status
        })]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "status", "--format", "json"],
        &environment,
        &services,
    )
    .expect("stack status json succeeds");
    let value: serde_json::Value = serde_json::from_str(&result.stdout).expect("valid json");

    assert_eq!(value["command"], "stack-status");
    assert_eq!(
        value["repositories"][0]["repository"],
        "example-owner/example-repo"
    );
    assert_eq!(value["repositories"][0]["trunk"]["remote"], "origin");
    assert_eq!(value["repositories"][0]["trunk"]["branch"], "main");
    assert_eq!(value["repositories"][0]["trunk"]["state"], "github-ahead");
    assert_eq!(value["repositories"][0]["trunk"]["githubAheadBy"], 0);
    assert_eq!(value["repositories"][0]["trunk"]["countsExact"], false);
    assert_eq!(value["repositories"][0]["pullRequests"][0]["number"], 103);
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["checkStatus"],
        "failing"
    );
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["reviewStatus"],
        "changes_requested"
    );
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["labels"][0]["name"],
        "bug"
    );
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["labels"][0]["color"],
        "d73a4a"
    );
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["commentedUsers"][0],
        "reviewer-commented"
    );
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["addressedUsers"][0],
        "reviewer-addressed"
    );
}

#[test]
fn stack_status_all_filters_configured_repositories() {
    // Verifies: -a positional patterns match configured repository identities like jx sync -a.
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
    let alpha = workspace.create_jj_workspace("projects/api-alpha");
    let beta = workspace.create_jj_workspace("projects/web-beta");
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api-alpha.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &beta,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/web-beta.git
"#,
    );
    write_stack_metadata(
        &alpha,
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![stack_status_node(
                201,
                "topic/alpha",
                "main",
                "Alpha change",
                false,
            )],
        },
    )
    .expect("alpha stack metadata writes");
    write_stack_metadata(
        &beta,
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![stack_status_node(
                202,
                "topic/beta",
                "main",
                "Beta change",
                false,
            )],
        },
    )
    .expect("beta stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        pull_request_statuses: BTreeMap::from([(
            201,
            stack_status_record(
                201,
                "Alpha change",
                "topic/alpha",
                "main",
                PullRequestCheckStatus::Passing,
                PullRequestReviewStatus::Approved,
                ReviewerSelection::default(),
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "status", "-a", "api-*"],
        &environment,
        &services,
    )
    .expect("filtered global stack status succeeds");

    assert!(result.stdout.contains("Stack status: 1 repository checked"));
    assert!(result.stdout.contains("api-alpha"));
    assert!(result.stdout.contains("Alpha change"));
    assert!(!result.stdout.contains("web-beta"));
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![201]]
    );
}

#[test]
fn stack_status_all_reconciles_work_item_handler_ledger() {
    // Verifies: global stack status uses the same metadata maintenance path as current-repo status.
    let workspace = TestWorkspace::new();
    let marker = workspace.home.join("resolved-global-work-item.txt");
    workspace.write_home_file(
        ".config/jx/config.toml",
        &format!(
            r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{{repo}}"

[[repo.rules]]
repo = "example-owner/*"

[repo.rules.work_items]
apply_on_stack_status = true

[[repo.rules.work_item_handlers]]
id = "resolve-work"
on = "work_item.fixed"
command = ["sh", "-c", "printf '%s\\n%s\\n' \"$1\" \"$(pwd)\" > \"$2\"", "_", "{{work_id}}", "{}"]
"#,
            marker.display()
        ),
    );
    let repo = workspace.create_jj_workspace("projects/api-alpha");
    TestWorkspace::write_git_config_at(
        &repo,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api-alpha.git
"#,
    );
    let mut node = stack_status_node(203, "topic/alpha", "main", "Alpha change", false);
    node.work_ids = vec!["ABC-203".to_owned()];
    node.fixes_work_ids = vec!["ABC-203".to_owned()];
    write_stack_metadata(
        &repo,
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![node],
        },
    )
    .expect("stack metadata writes");
    let mut status = stack_status_record(
        203,
        "Alpha change",
        "topic/alpha",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    status.merged = true;
    status.closed = true;
    status.merged_at = Some("2026-06-09T12:00:00Z".to_owned());
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        pull_request_statuses: BTreeMap::from([(203, status)]),
        ..FakeServices::default()
    };

    run_with_args_and_services(
        ["jx", "stack", "status", "-a", "api-*"],
        &environment,
        &services,
    )
    .expect("global stack status succeeds");

    let marker_contents = std::fs::read_to_string(marker).expect("handler marker is written");
    let marker_lines = marker_contents.lines().collect::<Vec<_>>();
    assert_eq!(marker_lines[0], "ABC-203");
    assert_eq!(
        std::fs::canonicalize(marker_lines[1]).expect("handler cwd exists"),
        std::fs::canonicalize(&repo).expect("repo path exists")
    );
    let metadata = read_stack_metadata(&repo).expect("stack metadata reads");
    assert_eq!(
        metadata.work_item_handler_runs,
        vec![StackMetadataWorkItemHandlerRun {
            handler: "resolve-work".to_owned(),
            work_id: "ABC-203".to_owned(),
            pull_request: 203,
        }]
    );
}

#[test]
fn stack_status_all_skips_repositories_pruned_to_empty_stack_state() {
    // Verifies: -a status cleans fully merged cached stacks and omits them from global output.
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
    let merged_repo = workspace.create_jj_workspace("projects/api-merged");
    let open_repo = workspace.create_jj_workspace("projects/api-open");
    TestWorkspace::write_git_config_at(
        &merged_repo,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api-merged.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &open_repo,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api-open.git
"#,
    );
    write_stack_metadata(
        &merged_repo,
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![stack_status_node(
                401,
                "topic/merged",
                "main",
                "Merged change",
                false,
            )],
        },
    )
    .expect("merged stack metadata writes");
    write_stack_metadata(
        &open_repo,
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![stack_status_node(
                402,
                "topic/open",
                "main",
                "Open change",
                false,
            )],
        },
    )
    .expect("open stack metadata writes");
    let mut merged = stack_status_record(
        401,
        "Merged change",
        "topic/merged",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    merged.merged = true;
    merged.closed = true;
    merged.merged_at = Some(chrono::Utc::now().to_rfc3339());
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        pull_request_statuses: BTreeMap::from([
            (401, merged),
            (
                402,
                stack_status_record(
                    402,
                    "Open change",
                    "topic/open",
                    "main",
                    PullRequestCheckStatus::Passing,
                    PullRequestReviewStatus::Approved,
                    ReviewerSelection::default(),
                ),
            ),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "status", "-a", "api-*"],
        &environment,
        &services,
    )
    .expect("global stack status succeeds");

    assert!(result.stdout.contains(
        "Stack status: 2 repositories checked, 2 repositories with stacks, 2 pull requests"
    ));
    assert!(result.stdout.contains("api-merged"));
    assert!(result.stdout.contains("● Merged change"));
    assert!(result.stdout.contains("api-open"));
    assert_eq!(
        read_stack_metadata(&merged_repo)
            .expect("merged metadata reads")
            .nodes
            .iter()
            .map(|node| (node.branch.as_str(), node.merged))
            .collect::<Vec<_>>(),
        vec![("topic/merged", true)]
    );
}

#[test]
fn stack_status_records_perf_steps() {
    // Verifies: stack status emits timing spans from its first implementation.
    let workspace = TestWorkspace::new();
    let perf_log = workspace.path().join("perf/stack-status.jsonl");
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
            nodes: vec![stack_status_node(
                301,
                "topic/perf",
                "main",
                "Perf change",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("JX_PERF_LOG".to_owned(), perf_log.display().to_string())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/perf".to_owned()],
        pull_request_statuses: BTreeMap::from([(
            301,
            stack_status_record(
                301,
                "Perf change",
                "topic/perf",
                "main",
                PullRequestCheckStatus::Passing,
                PullRequestReviewStatus::Approved,
                ReviewerSelection::default(),
            ),
        )]),
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");
    let log = fs::read_to_string(perf_log).expect("perf log writes");
    let event = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("perf event json"))
        .find(|event| event["op"] == "stack.status")
        .expect("stack status span is logged");

    assert_eq!(event["op"], "stack.status");
    assert_eq!(event["pr_count"], 1);
    let steps = event["steps"].as_array().expect("steps");
    assert!(steps
        .iter()
        .any(|step| step["name"] == "load_stack_snapshot"));
    assert!(steps
        .iter()
        .any(|step| step["name"] == "fetch_github_status"));
    assert!(steps
        .iter()
        .any(|step| step["name"] == "maintain_stack_metadata"));
    assert!(steps
        .iter()
        .any(|step| step["name"] == "fetch_trunk_status"));
    assert!(steps.iter().any(|step| step["name"] == "render"));
}

fn stack_status_pull_request_cell(number: u64) -> String {
    let target = format!("#{number}");
    format!(
        "{}{}",
        example_pull_request_link(number),
        " ".repeat(7_usize.saturating_sub(target.chars().count()))
    )
}

fn stack_status_node(
    number: u64,
    branch: &str,
    base_branch: &str,
    title: &str,
    draft: bool,
) -> StackMetadataNode {
    StackMetadataNode {
        branch: branch.to_owned(),
        base_branch: base_branch.to_owned(),
        parent_branch: (base_branch != "main").then(|| base_branch.to_owned()),
        pull_request: Some(number),
        parent_pull_request: None,
        title: title.to_owned(),
        url: Some(format!(
            "https://github.com/example-owner/example-repo/pull/{number}"
        )),
        draft,
        merged: false,
        work_ids: Vec::new(),
        fixes_work_ids: Vec::new(),
    }
}

fn stack_status_record(
    number: u64,
    title: &str,
    branch: &str,
    base_branch: &str,
    check_status: PullRequestCheckStatus,
    review_status: PullRequestReviewStatus,
    requested_reviewers: ReviewerSelection,
) -> PullRequestStatusRecord {
    PullRequestStatusRecord {
        number,
        title: title.to_owned(),
        url: Some(format!(
            "https://github.com/example-owner/example-repo/pull/{number}"
        )),
        created_at: None,
        head_branch: branch.to_owned(),
        base_branch: base_branch.to_owned(),
        author: None,
        draft: false,
        merged: false,
        closed: false,
        merged_at: None,
        closed_at: None,
        check_status,
        checks: Vec::new(),
        review_status,
        requested_reviewers,
        suggested_reviewers: Vec::new(),
        approved_reviewers: Vec::new(),
        commented_reviewers: Vec::new(),
        addressed_reviewers: Vec::new(),
        review_activity: Vec::new(),
        timeline_events: Vec::new(),
        labels: Vec::new(),
        latest_commit_oid: Some(format!("commit-{number}")),
    }
}

#[test]
fn stack_publish_existing_plans_include_projected_stack_context() {
    // Verifies: existing stacked PR updates can write the final stack-context body during publish.
    let mut root = preview_plan();
    root.title = "Root change".to_owned();
    root.body = "Root body".to_owned();
    root.bookmark.branch = "topic/root".to_owned();
    root.head = PullRequestHead::same_repository("example-owner", "topic/root");
    root.existing_pull_request = Some(pull_request_choice_record(
        1,
        "Root change",
        "topic/root",
        "main",
        false,
    ));

    let mut child = preview_plan();
    child.title = "Child change".to_owned();
    child.body = "Child body".to_owned();
    child.bookmark.branch = "topic/child".to_owned();
    child.head = PullRequestHead::same_repository("example-owner", "topic/child");
    child.base = "topic/root".to_owned();
    child.existing_pull_request = Some(pull_request_choice_record(
        2,
        "Child change",
        "topic/child",
        "topic/root",
        false,
    ));

    let mut plans = vec![root, child];
    add_projected_stack_context_to_existing_plans(&mut plans);

    assert!(plans[0].body.contains("<!-- jx-stack:start -->"));
    assert!(plans[0].body.contains("#1 Root change"));
    assert!(plans[0].body.contains("#2 Child change"));
    assert!(plans[1].body.contains("<!-- jx-stack:start -->"));
    assert!(plans[1].body.contains("#1 Root change"));
    assert!(plans[1].body.contains("#2 Child change"));
}

#[test]
fn stack_plan_renders_neighbourhood_tree_without_github_mutations() {
    // Verifies: stack plan shows selected and context commits in tree order without publishing.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut root = workspace_facts();
    root.target_change.short_commit_id = "aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    let mut left = workspace_facts();
    left.target_change.short_commit_id = "bbbbbbbb".to_owned();
    left.target_change.description = "Left change".to_owned();
    let mut right = workspace_facts();
    right.target_change.short_commit_id = "cccccccc".to_owned();
    right.target_change.description = "Right change".to_owned();
    let services = FakeServices {
        stack_plan_facts: Some(StackPlanFacts {
            trunk: root.trunk.clone(),
            nodes: vec![
                crate::jj::StackPlanNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPlanNodeFacts {
                    workspace: left,
                    parent_index: Some(0),
                },
                crate::jj::StackPlanNodeFacts {
                    workspace: right,
                    parent_index: Some(0),
                },
            ],
            selected_indexes: vec![1, 2],
            anchor_index: None,
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "plan", "-r", "left | right"],
        &environment,
        &services,
    )
    .expect("stack plan succeeds");

    assert_eq!(
        services.stack_plan_selections.borrow().as_slice(),
        [StackPlanSelection::ExplicitRevisions {
            revisions: vec!["left | right".to_owned()]
        }]
    );
    assert_eq!(
        result.stdout,
        "Stack plan: 3 commits, 2 selected\nBase: main @ 11112222\nRoot: aaaaaaaa Root change\n\n◯ aaaaaaaa Root change  context\n├ ◉ bbbbbbbb Left change  selected\n└ ◉ cccccccc Right change  selected\n\nSelected revisions share one stack root. Publish would create/update PRs for selected rows.\n"
    );
    assert!(services.sync_pull_request_pushes.borrow().is_empty());
}

#[test]
fn stack_publish_without_revision_publishes_inferred_stack() {
    // Verifies: stack publish expands the working-copy stack when no explicit revset is supplied.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let root_pr =
        pull_request_choice_record(42, "Root change", "example-user/00-aaaaaaaa", "main", false);
    let child_pr = pull_request_choice_record(
        43,
        "Child change",
        "example-user/01-bbbbbbbb",
        "example-user/00-aaaaaaaa",
        false,
    );
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        sync_pull_requests: vec![root_pr, child_pr],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "--apply-to-stack"],
        &environment,
        &services,
    )
    .expect("stack publishes");

    assert_eq!(
        services.stack_publish_selections.borrow().first(),
        Some(&StackPublishSelection::InferredStack { anchor: None })
    );
    assert_eq!(
        services.sync_pull_request_pushes.borrow()[0]
            .bookmarks
            .iter()
            .map(|bookmark| (
                bookmark.branch.as_str(),
                bookmark.pull_request_base.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("example-user/00-aaaaaaaa", Some("main")),
            ("example-user/01-bbbbbbbb", Some("example-user/00-aaaaaaaa")),
        ]
    );
    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nCreated {}\nStack: refreshed stack context on {}, {}\n",
            example_pull_request_link(42),
            example_pull_request_link(43),
            example_pull_request_link(42),
            example_pull_request_link(43),
        )
    );
}

#[test]
fn stack_publish_intent_flags_apply_to_current_commit_only() {
    // Verifies: publish intent such as task, labels, reviewers, draft, and fixes stays on the current stack commit.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "TASK-123: Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let root_pr =
        pull_request_choice_record(42, "Root change", "example-user/00-aaaaaaaa", "main", false);
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        reviewer_candidates: vec![ReviewerCandidate::new(
            ReviewerTarget::user("path-reviewer"),
            vec!["matched 1 file".to_owned()],
        )],
        pull_requests_by_head: BTreeMap::from([(
            "example-user/00-aaaaaaaa".to_owned(),
            root_pr.clone(),
        )]),
        sync_pull_requests: vec![
            root_pr,
            pull_request_choice_record(
                43,
                "TASK-123: Child change",
                "example-user/01-bbbbbbbb",
                "example-user/00-aaaaaaaa",
                true,
            ),
        ],
        expected_labels_by_publish: Some(vec![Vec::new(), vec!["needs-review".to_owned()]]),
        ..FakeServices::default()
    };

    run_with_args_and_services(
        [
            "jx",
            "stack",
            "publish",
            "-t",
            "TASK-123",
            "-F",
            "TASK-123",
            "--label",
            "needs-review",
            "--draft",
            "-R",
            "manual-reviewer",
        ],
        &environment,
        &services,
    )
    .expect("stack publishes");

    assert_eq!(
        services.pull_request_plan_task_ids.borrow().as_slice(),
        &[None, Some("TASK-123".to_owned())]
    );
    let published_plans = services.published_plans.borrow();
    assert_eq!(published_plans[0].labels, Vec::<String>::new());
    assert_eq!(published_plans[0].reviewers, ReviewerSelection::default());
    assert!(!published_plans[0].draft);
    assert_eq!(published_plans[1].labels, ["needs-review".to_owned()]);
    assert_eq!(
        published_plans[1].reviewers,
        ReviewerSelection::new(
            ["manual-reviewer", "path-reviewer"],
            std::iter::empty::<&str>()
        )
    );
    assert!(published_plans[1].draft);

    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    let root = metadata
        .nodes
        .iter()
        .find(|node| node.pull_request == Some(42))
        .expect("root node exists");
    let child = metadata
        .nodes
        .iter()
        .find(|node| node.pull_request == Some(43))
        .expect("child node exists");
    assert!(root.work_ids.is_empty());
    assert!(root.fixes_work_ids.is_empty());
    assert_eq!(child.work_ids, ["TASK-123".to_owned()]);
    assert_eq!(child.fixes_work_ids, ["TASK-123".to_owned()]);
}

#[test]
fn stack_publish_apply_to_stack_applies_intent_to_every_published_pr() {
    // Verifies: explicit stack-wide intent restores uniform labels, reviewers, readiness, and task context.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "TASK-123: Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "TASK-123: Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        reviewer_candidates: vec![ReviewerCandidate::new(
            ReviewerTarget::user("path-reviewer"),
            vec!["matched 1 file".to_owned()],
        )],
        expected_task_id: Some(Some("TASK-123".to_owned())),
        expected_labels: vec!["needs-review".to_owned()],
        expected_draft: Some(true),
        expected_reviewers: Some(ReviewerSelection::new(
            ["manual-reviewer", "path-reviewer"],
            std::iter::empty::<&str>(),
        )),
        sync_pull_requests: vec![
            pull_request_choice_record(
                42,
                "TASK-123: Root change",
                "example-user/00-aaaaaaaa",
                "main",
                true,
            ),
            pull_request_choice_record(
                43,
                "TASK-123: Child change",
                "example-user/01-bbbbbbbb",
                "example-user/00-aaaaaaaa",
                true,
            ),
        ],
        ..FakeServices::default()
    };

    run_with_args_and_services(
        [
            "jx",
            "stack",
            "publish",
            "-A",
            "-t",
            "TASK-123",
            "--label",
            "needs-review",
            "--draft",
            "-R",
            "manual-reviewer",
        ],
        &environment,
        &services,
    )
    .expect("stack publishes");

    assert_eq!(services.published_pull_request_count.get(), 2);
}

#[test]
fn stack_publish_apply_to_stack_interleaves_previews_and_confirmations() {
    // Verifies: stack-wide confirmation shows each PR preview immediately before its prompt.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        ..FakeServices::default()
    };
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let previewer = TitleRecordingPullRequestPreviewer {
        events: events.clone(),
    };
    let confirmer = TitleRecordingPullRequestConfirmer {
        events: events.clone(),
    };
    let prompts = PromptHandlers {
        pull_request_previewer: &previewer,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &confirmer,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };

    run_with_args_and_progress(
        ["jx", "stack", "publish", "-A"],
        &environment,
        &services,
        &NoProgress,
        prompts,
        OutputMode::plain(),
    )
    .expect("stack publishes");

    assert_eq!(
        events.borrow().as_slice(),
        &[
            "preview Root change".to_owned(),
            "confirm Root change".to_owned(),
            "preview Child change".to_owned(),
            "confirm Child change".to_owned(),
        ]
    );
}

#[test]
fn stack_publish_apply_to_stack_records_fix_intent_on_stack_tip() {
    // Verifies: stack-wide publish intent treats the final stack PR as the ticket-fixing PR.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "TASK-123: Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "TASK-123: Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        ..FakeServices::default()
    };

    run_with_args_and_services(
        [
            "jx",
            "stack",
            "publish",
            "--apply-to-stack",
            "-F",
            "TASK-123",
        ],
        &environment,
        &services,
    )
    .expect("stack publishes");

    assert_eq!(services.published_pull_request_count.get(), 2);
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    let root = metadata
        .nodes
        .iter()
        .find(|node| node.pull_request == Some(42))
        .expect("root node exists");
    let child = metadata
        .nodes
        .iter()
        .find(|node| node.pull_request == Some(43))
        .expect("child node exists");
    assert_eq!(root.work_ids, ["TASK-123".to_owned()]);
    assert!(root.fixes_work_ids.is_empty());
    assert_eq!(child.work_ids, ["TASK-123".to_owned()]);
    assert_eq!(child.fixes_work_ids, ["TASK-123".to_owned()]);
}

#[test]
fn stack_publish_bare_fixes_requires_attached_work_id() {
    // Verifies: -F without a value fails before publishing when no work ID is attached.
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
    let services = FakeServices::default();

    let error = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@", "-F"],
        &environment,
        &services,
    )
    .expect_err("bare fixes without work ID is rejected");

    assert!(matches!(error, CommandError::Usage(_)));
    assert_eq!(services.published_pull_request_count.get(), 0);
}

#[test]
fn stack_publish_fixes_requires_single_intent_target() {
    // Verifies: fix intent needs one current or explicitly selected target, not an ambiguous explicit set.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: None,
            metrics: StackPublishMetrics::default(),
        }),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(
        [
            "jx", "stack", "publish", "-r", "root", "-r", "child", "-F", "TASK-123",
        ],
        &environment,
        &services,
    )
    .expect_err("multi-pr fixes are rejected");

    assert!(matches!(error, CommandError::Usage(_)));
    assert_eq!(services.published_pull_request_count.get(), 0);
}

#[test]
fn stack_publish_ignores_empty_commits_in_selected_stack() {
    // Verifies: empty commits do not block publishing non-empty stack descendants.
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
    let mut empty_root = workspace_facts();
    empty_root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    empty_root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    empty_root.target_change.description = "Empty root".to_owned();
    empty_root.target_change.is_empty = true;
    empty_root.changed_files = Vec::new();
    empty_root.local_bookmarks_at_target = vec!["example-user/empty-root".to_owned()];
    empty_root.nearest_ancestor_bookmark = None;
    empty_root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = Some("example-user/empty-root".to_owned());
    child.stack_index = 1;
    let child_pr = pull_request_choice_record(
        42,
        "Child change",
        "example-user/01-bbbbbbbb",
        "main",
        false,
    );
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: empty_root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        sync_pull_requests: vec![child_pr],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "publish"], &environment, &services)
        .expect("stack publishes non-empty child");

    assert_eq!(services.published_pull_request_count.get(), 1);
    assert_eq!(
        services.sync_pull_request_pushes.borrow()[0]
            .bookmarks
            .iter()
            .map(|bookmark| (
                bookmark.branch.as_str(),
                bookmark.pull_request_base.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![("example-user/01-bbbbbbbb", Some("main"))]
    );
    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nStack: refreshed stack context on {}\n",
            example_pull_request_link(42),
            example_pull_request_link(42),
        )
    );
}

#[test]
fn stack_publish_updates_only_metadata_when_existing_branch_is_up_to_date() {
    // Verifies: unchanged existing PRs can update reviewers, labels, and ready state without PR code updates.
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
    let existing = PullRequestRecord {
        number: 77,
        title: "Existing title".to_owned(),
        body: Some("Existing body".to_owned()),
        head_branch: "example-user/02-zzzzzzzz".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/77".to_owned()),
        draft: true,
        merged: false,
        reviewers: ReviewerSelection::new(["existing-reviewer"], std::iter::empty::<&str>()),
    };
    let services = FakeServices {
        existing_pull_request: Some(existing),
        push: PushOutcome {
            branch: String::new(),
            pushed_refs: 0,
            pushed_commits: Vec::new(),
        },
        bookmark_update: BookmarkUpdate {
            branch: String::new(),
            created: false,
        },
        reviewer_candidates: vec![ReviewerCandidate::new(
            ReviewerTarget::team("example-org/platform", "platform"),
            vec!["matched 1 file".to_owned()],
        )],
        expected_draft: Some(false),
        expected_labels: vec!["needs-review".to_owned()],
        expected_reviewers: Some(ReviewerSelection::new(["existing-reviewer"], ["platform"])),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        [
            "jx",
            "stack",
            "publish",
            "--ready",
            "--label",
            "needs-review",
        ],
        &environment,
        &services,
    )
    .expect("metadata-only stack publish succeeds");

    assert_eq!(services.published_pull_request_count.get(), 0);
    assert_eq!(services.metadata_only_pull_request_count.get(), 1);
    assert_eq!(
        services.push_bookmark_calls.borrow().as_slice(),
        &["example-user/02-zzzzzzzz".to_owned()]
    );
    assert_eq!(
        services.sync_pull_request_pushes.borrow()[0]
            .bookmarks
            .iter()
            .map(|bookmark| (
                bookmark.branch.as_str(),
                bookmark.pull_request_base.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![("example-user/02-zzzzzzzz", Some("main"))]
    );
    assert_eq!(
        result.stdout,
        format!("Updated {}\n", example_pull_request_link(77))
    );
}

#[test]
fn stack_publish_declined_existing_pr_refreshes_context_only() {
    // Verifies: declining a PR update still refreshes its generated stack tree without retargeting it.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let root_branch = "example-user/00-aaaaaaaa";
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let child_branch = "example-user/01-bbbbbbbb";
    let root_pr = pull_request_choice_record(50, "Root change", root_branch, "main", false);
    let child_pr = pull_request_choice_record(51, "Child change", child_branch, root_branch, false);
    let confirmer = SequencePullRequestConfirmer::new([false, true]);
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        pull_requests_by_head: BTreeMap::from([
            (root_branch.to_owned(), root_pr.clone()),
            (child_branch.to_owned(), child_pr.clone()),
        ]),
        sync_pull_requests: vec![child_pr, root_pr],
        pull_request_action: PullRequestAction::Updated,
        ..FakeServices::default()
    };

    let result = run_with_args_and_prompts(
        ["jx", "stack", "publish"],
        &environment,
        &services,
        &SelectAllReviewers,
        &confirmer,
    )
    .expect("stack publish proceeds past declined existing PR");

    assert_eq!(confirmer.titles.borrow().as_slice(), ["Child change"]);
    assert_eq!(services.published_pull_request_count.get(), 1);
    assert_eq!(
        services.push_bookmark_calls.borrow().as_slice(),
        &[root_branch.to_owned()]
    );
    let sync_pushes = services.sync_pull_request_pushes.borrow();
    assert_eq!(sync_pushes.len(), 2);
    assert_eq!(
        sync_pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| (
                bookmark.branch.as_str(),
                bookmark.pull_request_base.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![(root_branch, Some("main"))]
    );
    assert_eq!(
        sync_pushes[1]
            .bookmarks
            .iter()
            .map(|bookmark| (
                bookmark.branch.as_str(),
                bookmark.pull_request_base.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![(child_branch, None)]
    );
    assert!(result
        .stdout
        .contains(&format!("Updated {}", example_pull_request_link(42))));
}

#[test]
fn stack_publish_declined_unpublished_parent_skips_descendants() {
    // Verifies: descendants are not published when their unpublished stack base was declined.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let confirmer = SequencePullRequestConfirmer::new([false, true]);
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_prompts(
        ["jx", "stack", "publish"],
        &environment,
        &services,
        &SelectAllReviewers,
        &confirmer,
    )
    .expect("stack publish cancellation succeeds");

    assert!(confirmer.titles.borrow().is_empty());
    assert_eq!(services.published_pull_request_count.get(), 0);
    assert!(services.push_bookmark_calls.borrow().is_empty());
    assert!(services.sync_pull_request_pushes.borrow().is_empty());
    assert_eq!(result.stdout, "cancelled\n");
}

#[test]
fn stack_publish_readiness_selectors_can_create_mixed_ready_draft_stack() {
    // Verifies: readiness revsets assign final ready/draft state within the published stack.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let facts = StackPublishFacts {
        nodes: vec![
            crate::jj::StackPublishNodeFacts {
                workspace: root,
                parent_index: None,
            },
            crate::jj::StackPublishNodeFacts {
                workspace: child,
                parent_index: Some(0),
            },
        ],
        publish_indexes: vec![0, 1],
        anchor_index: Some(1),
        metrics: StackPublishMetrics::default(),
    };
    let mut ready_root_facts = facts.clone();
    ready_root_facts.publish_indexes = vec![0];
    ready_root_facts.anchor_index = None;
    let services = FakeServices {
        stack_publish_facts: Some(facts),
        stack_publish_facts_by_revision: BTreeMap::from([("root".to_owned(), ready_root_facts)]),
        expected_drafts: Some(vec![false, true]),
        ..FakeServices::default()
    };

    run_with_args_and_services(
        ["jx", "stack", "publish", "--draft", "--ready=root"],
        &environment,
        &services,
    )
    .expect("stack publishes with mixed readiness");

    assert_eq!(services.published_pull_request_count.get(), 2);
    assert_eq!(
        services.stack_publish_selections.borrow().as_slice(),
        &[
            StackPublishSelection::InferredStack { anchor: None },
            StackPublishSelection::ExplicitRevisions {
                revisions: vec!["root".to_owned()],
            },
        ]
    );
}

#[test]
fn stack_publish_rejects_overlapping_readiness_selectors() {
    // Verifies: contradictory ready/draft revsets fail before any publishing mutation.
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
    let mut selected = workspace_facts();
    selected.target_change.change_id = "aaaaaaaa11111111".to_owned();
    let facts = StackPublishFacts {
        nodes: vec![crate::jj::StackPublishNodeFacts {
            workspace: selected,
            parent_index: None,
        }],
        publish_indexes: vec![0],
        anchor_index: Some(0),
        metrics: StackPublishMetrics::default(),
    };
    let services = FakeServices {
        stack_publish_facts: Some(facts.clone()),
        stack_publish_facts_by_revision: BTreeMap::from([("selected".to_owned(), facts)]),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(
        [
            "jx",
            "stack",
            "publish",
            "--ready=selected",
            "--draft=selected",
        ],
        &environment,
        &services,
    )
    .expect_err("overlapping readiness is rejected");

    assert!(error
        .to_string()
        .contains("--ready and --draft selectors overlap"));
    assert_eq!(services.published_pull_request_count.get(), 0);
}

#[test]
fn stack_publish_writes_perf_trace_for_publish_and_stack_update() {
    // Verifies: stack publish emits structured timings for the command and stack update phases.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let log_path = workspace.path().join("jx-perf.log");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
            ("JX_PERF_LOG".to_owned(), log_path.display().to_string()),
        ],
    );
    let services = FakeServices::default();

    run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("stack publishes");

    let events = fs::read_to_string(&log_path).expect("perf log writes");
    let events = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("perf line is json"))
        .collect::<Vec<_>>();
    let publish = events
        .iter()
        .find(|event| event["op"] == "stack.publish")
        .expect("stack publish span is logged");
    assert_eq!(publish["repo"], "example-owner/example-repo");
    assert_eq!(publish["explicit_revisions"], true);
    assert!(publish["duration_us"].as_u64().is_some());
    let step_names = publish["steps"]
        .as_array()
        .expect("publish steps are logged")
        .iter()
        .filter_map(|step| step["name"].as_str())
        .collect::<Vec<_>>();
    assert!(step_names.contains(&"load_publish_stack"));
    assert!(step_names.contains(&"stack_publish_facts.resolve_trunk"));
    assert!(step_names.contains(&"stack_publish_facts.workspace_facts"));
    assert!(step_names.contains(&"plan_pull_requests"));
    assert!(step_names.contains(&"pull_request_plan"));
    assert!(step_names.contains(&"publish_pull_request"));
    assert!(step_names.contains(&"update_stack"));
    let update = events
        .iter()
        .find(|event| event["op"] == "stack.update_after_publish")
        .expect("stack update span is logged");
    let update_step_names = update["steps"]
        .as_array()
        .expect("update steps are logged")
        .iter()
        .filter_map(|step| step["name"].as_str())
        .collect::<Vec<_>>();
    assert!(update_step_names.contains(&"load_local_stack_branches"));
    assert!(update_step_names.contains(&"local_stack_branches.resolve_trunk"));
    assert!(update_step_names.contains(&"local_stack_branches.linear_stack_path"));
    assert!(update_step_names
        .contains(&"stack_metadata.apply_existing_local.apply_local_stack_branches"));
    assert!(
        update_step_names.contains(&"stack_metadata.apply_seeded_local.apply_local_stack_branches")
    );
    assert!(update_step_names
        .contains(&"component_metadata.apply_local_stack_metadata.apply_local_stack_branches"));
    assert!(!update_step_names
        .iter()
        .any(|name| name.ends_with(".local_stack_branches")));
    assert!(events
        .iter()
        .any(|event| event["op"] == "stack.sync_pull_requests"));
}

#[test]
fn stack_publish_revision_publishes_explicit_subset_only() {
    // Verifies: -r selects the publish set instead of expanding to the whole stack.
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
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = Some("topic/root".to_owned());
    child.stack_index = 1;
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![1],
            anchor_index: None,
            metrics: StackPublishMetrics::default(),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("selected change publishes");

    assert_eq!(
        services.stack_publish_selections.borrow().first(),
        Some(&StackPublishSelection::ExplicitRevisions {
            revisions: vec!["@".to_owned()]
        })
    );
    assert_eq!(
        services.sync_pull_request_pushes.borrow()[0].bookmarks[0]
            .pull_request_base
            .as_deref(),
        Some("topic/root")
    );
    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
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
    assert!(refresh_help.contains("searches open GitHub PRs authored by the authenticated login"));
    assert!(refresh_help.contains("writes"));
    assert!(refresh_help.contains(".jx/stack.toml"));
    assert!(refresh_help.contains("syncs affected PR bases/descriptions"));
    assert!(refresh_help.contains("push branches"));
    assert!(refresh_help.contains("create, close, or delete pull requests"));
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
            work_item_handler_runs: Vec::new(),
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
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
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
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
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
    let selector = RecordingPullRequestSelector::new(0);

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
            "\x1b[2m\x1b[38;2;190;184;176m◌ #11     Chosen change\x1b[0m".to_owned(),
            "◯ #10     Older change".to_owned(),
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
            work_item_handler_runs: Vec::new(),
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
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
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
            work_item_handler_runs: Vec::new(),
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
                    url: Some("https://github.com/example-owner/example-repo/pull/11".to_owned()),
                    draft: false,
                    merged: false,
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
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
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
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
    let selector = RecordingPullRequestSelector::new(2);

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
            "\x1b[2m\x1b[38;2;190;184;176m◌ #12     Draft\x1b[0m".to_owned(),
            "✓ #10     Root".to_owned(),
            "└ ◉ #11     Child".to_owned(),
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
            work_item_handler_runs: Vec::new(),
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
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
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
                    work_ids: Vec::new(),
                    fixes_work_ids: Vec::new(),
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
        OutputMode {
            color: true,
            terminal_width: None,
        },
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
            work_item_handler_runs: Vec::new(),
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
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
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
            work_item_handler_runs: Vec::new(),
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
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
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
        CommandError::Workflow(error)
            if matches!(*error, WorkflowError::MissingLocalBookmarkPullRequests { ref repository } if repository == "example-owner/example-repo")
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
        "◯ #10     Root\n├ ◯ #11     Child\n└ ◌ #12     Draft\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join(".jx/.gitignore")).expect("read gitignore"),
        "/.gitignore\n/workspace.toml\n/stack.toml\n"
    );
    let stack_file =
        fs::read_to_string(workspace.path().join(".jx/stack.toml")).expect("read stack state");
    assert!(stack_file.contains("pull_request = 10"));
    assert!(stack_file.contains("parent_branch = \"topic/root\""));
    let sync_pushes = services.sync_pull_request_pushes.borrow();
    assert_eq!(sync_pushes.len(), 1);
    assert_eq!(
        sync_pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["topic/root", "topic/child", "topic/draft"]
    );
    assert_eq!(
        sync_pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.pull_request_base.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("main"), Some("topic/root"), Some("topic/root")]
    );
}

#[test]
fn stack_refresh_includes_authored_agent_pr_without_local_bookmark() {
    // Verifies: repo-authored search seeds stack metadata for same-repo agent branches not tracked locally.
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
        github_login: "example-user".to_owned(),
        authored_open_pull_requests: vec![pull_request_choice_record(
            246,
            "Agent-authored plan",
            "agent/generated-plan",
            "main",
            true,
        )],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "refresh"], &environment, &services)
        .expect("stack refresh succeeds");

    assert_eq!(result.stdout, "◌ #246    Agent-authored plan\n");
    let stack_file =
        fs::read_to_string(workspace.path().join(".jx/stack.toml")).expect("read stack state");
    assert!(stack_file.contains("branch = \"agent/generated-plan\""));
    assert!(stack_file.contains("pull_request = 246"));
    assert_eq!(
        services
            .authored_open_pull_request_calls
            .borrow()
            .as_slice(),
        ["example-user"]
    );
    assert!(services
        .authored_open_pull_request_head_calls
        .borrow()
        .is_empty());
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
            work_item_handler_runs: Vec::new(),
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
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
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
fn sk_alias_shows_stack_state() {
    // Verifies: the short `sk` alias keeps the read-only stack view easy to invoke.
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
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "sk"], &environment, &FakeServices::default())
        .expect("stack alias succeeds");

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
            work_item_handler_runs: Vec::new(),
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
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
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
                title: "Old child".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
        ],
    };
    let child = pull_request_choice_record(11, "Child", "topic/child", "main", false);

    let metadata = stack_metadata_from_pull_requests(&[child], &existing);

    assert_eq!(
        stack_metadata_rows(&metadata.nodes),
        vec!["✓ #10     Root", "└ ◯ #11     Child"]
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
            work_item_handler_runs: Vec::new(),
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
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
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
        reviewers: ReviewerSelection::default(),
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

    assert_eq!(result.stdout, "✓ #10     Merged root\n└ ◯ #11     Child\n");
    assert_eq!(
        services.pull_request_number_calls.borrow().as_slice(),
        [10, 10]
    );
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes[0].branch, "topic/root");
    assert_eq!(metadata.nodes[0].title, "Merged root");
    assert!(metadata.nodes[0].merged);
}

#[test]
fn stack_onto_moves_current_stack_and_syncs_old_and_new_components() {
    // Verifies: Stack moves sync every branch affected by the old and new stack graph.
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
                stack_node("topic/root", "main", None, 10, "Root"),
                stack_node("topic/child", "topic/root", Some("topic/root"), 11, "Child"),
                stack_node("topic/new-root", "main", None, 12, "New root"),
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec![
            "topic/root".to_owned(),
            "topic/child".to_owned(),
            "topic/new-root".to_owned(),
        ],
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        local_stack_branches: std::cell::RefCell::new(vec![
            vec![
                local_stack_branch("topic/root", "main", None),
                local_stack_branch("topic/child", "topic/root", Some("topic/root")),
                local_stack_branch("topic/new-root", "main", None),
            ],
            vec![
                local_stack_branch("topic/root", "main", None),
                local_stack_branch("topic/child", "topic/new-root", Some("topic/new-root")),
                local_stack_branch("topic/new-root", "main", None),
            ],
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "--onto", "new-root"],
        &environment,
        &services,
    )
    .expect("stack move succeeds");

    assert_eq!(
        services.stack_move_targets.borrow().as_slice(),
        &[StackMoveTarget::Onto("new-root".to_owned())]
    );
    assert_eq!(
        services.push_syncable_revision_requests.borrow().as_slice(),
        &[
            Some("topic/root".to_owned()),
            Some("topic/child".to_owned()),
            Some("topic/new-root".to_owned()),
        ]
    );
    let synced_metadata = services.sync_pull_request_metadata.borrow();
    let child = synced_metadata
        .last()
        .expect("sync receives metadata")
        .nodes
        .iter()
        .find(|node| node.branch == "topic/child")
        .expect("child metadata exists");
    assert_eq!(child.base_branch, "topic/new-root");
    assert_eq!(child.parent_branch.as_deref(), Some("topic/new-root"));
    assert!(result
        .stdout
        .starts_with("Moved current stack from a1b2c3d4 onto new-root\nSynced:"));
}

#[test]
fn stack_move_no_sync_updates_local_metadata_without_github_mutations() {
    // Verifies: --no-sync keeps the stack move local while still repairing cached stack state.
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
                stack_node("topic/root", "main", None, 10, "Root"),
                stack_node("topic/child", "topic/root", Some("topic/root"), 11, "Child"),
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        local_stack_branches: std::cell::RefCell::new(vec![vec![
            local_stack_branch("topic/root", "main", None),
            local_stack_branch("topic/child", "main", None),
        ]]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "--trunk", "--no-sync"],
        &environment,
        &services,
    )
    .expect("local stack move succeeds");

    assert_eq!(
        services.stack_move_targets.borrow().as_slice(),
        &[StackMoveTarget::Trunk]
    );
    assert!(services.fetch_origin_roots.borrow().is_empty());
    assert!(services.push_syncable_revision_requests.borrow().is_empty());
    assert!(services.sync_pull_request_metadata.borrow().is_empty());
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    let child = metadata
        .nodes
        .iter()
        .find(|node| node.branch == "topic/child")
        .expect("child metadata exists");
    assert_eq!(child.base_branch, "main");
    assert_eq!(child.parent_branch, None);
    assert_eq!(
        result.stdout,
        "Moved current stack from a1b2c3d4 onto trunk\nSync skipped (--no-sync)\n"
    );
}

struct TitleRecordingPullRequestPreviewer {
    events: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl PullRequestPreviewer for TitleRecordingPullRequestPreviewer {
    fn show_preview(
        &self,
        plan: &PullRequestPlan,
        _status: &WorkspaceStatus,
        _prepare_effects: &[PullRequestEventEffect],
    ) {
        self.events
            .borrow_mut()
            .push(format!("preview {}", plan.title));
    }
}

struct TitleRecordingPullRequestConfirmer {
    events: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl PullRequestConfirmer for TitleRecordingPullRequestConfirmer {
    fn confirm_pull_request(
        &self,
        plan: &PullRequestPlan,
    ) -> Result<bool, PullRequestConfirmationError> {
        self.events
            .borrow_mut()
            .push(format!("confirm {}", plan.title));
        Ok(true)
    }
}

fn stack_node(
    branch: &str,
    base_branch: &str,
    parent_branch: Option<&str>,
    pull_request: u64,
    title: &str,
) -> StackMetadataNode {
    StackMetadataNode {
        branch: branch.to_owned(),
        base_branch: base_branch.to_owned(),
        parent_branch: parent_branch.map(str::to_owned),
        pull_request: Some(pull_request),
        parent_pull_request: None,
        title: title.to_owned(),
        url: None,
        draft: false,
        merged: false,
        work_ids: Vec::new(),
        fixes_work_ids: Vec::new(),
    }
}

fn local_stack_branch(
    branch: &str,
    base_branch: &str,
    parent_branch: Option<&str>,
) -> LocalStackBranch {
    LocalStackBranch {
        branch: branch.to_owned(),
        base_branch: base_branch.to_owned(),
        parent_branch: parent_branch.map(str::to_owned),
        title: branch.to_owned(),
    }
}

fn help_output<const N: usize>(args: [&str; N]) -> String {
    let error = cli()
        .try_get_matches_from(args)
        .expect_err("help exits before command execution");
    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
    error.to_string()
}
