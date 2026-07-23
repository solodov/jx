use super::*;
use crate::github::{PullRequestMergeStatus, PullRequestReviewerMention};

#[test]
fn review_interactive_flags_parse_without_loading_dashboard() {
    // Verifies: review dashboard flags are represented in the typed request.
    let matches = cli()
        .try_get_matches_from(["jx", "review", "-i", "--refresh-seconds", "15", "api-*"])
        .expect("review dashboard args parse");
    let request = CommandRequest::from_matches(&matches).expect("request builds");

    let CommandRequest::Review(request) = request else {
        panic!("expected review request");
    };
    assert_eq!(request.action, ReviewAction::Show);
    assert_eq!(request.repo_filters, vec!["api-*".to_owned()]);
    assert!(request.interactive);
    assert_eq!(request.refresh_seconds, 15);
    assert_eq!(request.format, ReviewFormat::Human);
}

#[test]
fn review_dismiss_flag_parses_target_selector() {
    // Verifies: review dismissal is a typed subcommand rather than a repository filter.
    let matches = cli()
        .try_get_matches_from(["jx", "review", "dismiss", "example-owner/api-alpha#12"])
        .expect("review dismiss args parse");
    let request = CommandRequest::from_matches(&matches).expect("request builds");

    let CommandRequest::Review(request) = request else {
        panic!("expected review request");
    };
    assert_eq!(
        request.action,
        ReviewAction::Dismiss {
            selector: "example-owner/api-alpha#12".to_owned(),
        }
    );
}

#[test]
fn review_dismissed_history_and_undismiss_parse_as_typed_actions() {
    // Verifies: local dismissal management stays separate from review repository filters.
    let matches = cli()
        .try_get_matches_from(["jx", "review", "dismissed"])
        .expect("review dismissed args parse");
    let request = CommandRequest::from_matches(&matches).expect("request builds");
    let CommandRequest::Review(request) = request else {
        panic!("expected review request");
    };
    assert_eq!(request.action, ReviewAction::Dismissed);

    let matches = cli()
        .try_get_matches_from(["jx", "review", "history", "api-alpha#12"])
        .expect("review history args parse");
    let request = CommandRequest::from_matches(&matches).expect("request builds");
    let CommandRequest::Review(request) = request else {
        panic!("expected review request");
    };
    assert_eq!(
        request.action,
        ReviewAction::History {
            selector: "api-alpha#12".to_owned(),
        }
    );

    let matches = cli()
        .try_get_matches_from(["jx", "review", "undismiss", "api-alpha#12"])
        .expect("review undismiss args parse");
    let request = CommandRequest::from_matches(&matches).expect("request builds");
    let CommandRequest::Review(request) = request else {
        panic!("expected review request");
    };
    assert_eq!(
        request.action,
        ReviewAction::Undismiss {
            selector: "api-alpha#12".to_owned(),
        }
    );
}

#[test]
fn review_interactive_rejects_json_format() {
    // Verifies: machine-readable provider output stays non-interactive for external wrappers.
    let matches = cli()
        .try_get_matches_from(["jx", "review", "-i", "--format", "json"])
        .expect("CLI shape parses before request validation");
    let result = CommandRequest::from_matches(&matches);

    assert!(result.is_err());
}

#[test]
fn review_render_uses_viewer_review_state_symbols() {
    // Verifies: review rows summarize the viewer's own wait, comment, change-request, and approval states.
    let mut waiting = review_status_record(12, "Waiting on me", "example-author", false);
    waiting.requested_reviewers =
        ReviewerSelection::new(["example-reviewer"], Vec::<String>::new());
    let mut commented = review_status_record(13, "I left comments", "example-author", false);
    commented.commented_reviewers = vec!["example-reviewer".to_owned()];
    let mut changes_requested =
        review_status_record(14, "I requested changes", "example-author", false);
    changes_requested.changes_requested_reviewers = vec!["example-reviewer".to_owned()];
    let mut approved = review_status_record(15, "I approved", "example-author", false);
    approved.approved_reviewers = vec!["example-reviewer".to_owned()];
    let mut approved_with_comments =
        review_status_record(16, "I approved with comments", "example-author", false);
    approved_with_comments.approved_reviewers = vec!["example-reviewer".to_owned()];
    approved_with_comments.commented_reviewers = vec!["example-reviewer".to_owned()];
    let mut stale_approval =
        review_status_record(17, "My approval was dismissed", "example-author", false);
    stale_approval.dismissed_reviewers = vec!["example-reviewer".to_owned()];
    let view = ReviewRequestsView {
        viewer: "example-reviewer".to_owned(),
        repositories: vec![ReviewRequestRepositoryView {
            repository: GitHubRepository {
                owner: "example-owner".to_owned(),
                name: "api-alpha".to_owned(),
            },
            layout_key: None,
            root: None,
            display_root: None,
            rows: vec![
                ReviewRequestRowView {
                    status: waiting,
                    state: crate::domain::ReviewRequestState::New,
                    viewer_signal: ReviewRequestViewerSignal::None,
                    lag_since_unix: None,
                    dismissal: None,
                },
                ReviewRequestRowView {
                    status: commented,
                    state: crate::domain::ReviewRequestState::Commented,
                    viewer_signal: ReviewRequestViewerSignal::None,
                    lag_since_unix: None,
                    dismissal: None,
                },
                ReviewRequestRowView {
                    status: changes_requested,
                    state: crate::domain::ReviewRequestState::ChangesRequested,
                    viewer_signal: ReviewRequestViewerSignal::None,
                    lag_since_unix: None,
                    dismissal: None,
                },
                ReviewRequestRowView {
                    status: approved,
                    state: crate::domain::ReviewRequestState::Approved,
                    viewer_signal: ReviewRequestViewerSignal::None,
                    lag_since_unix: None,
                    dismissal: None,
                },
                ReviewRequestRowView {
                    status: approved_with_comments,
                    state: crate::domain::ReviewRequestState::Approved,
                    viewer_signal: ReviewRequestViewerSignal::None,
                    lag_since_unix: None,
                    dismissal: None,
                },
                ReviewRequestRowView {
                    status: stale_approval,
                    state: crate::domain::ReviewRequestState::New,
                    viewer_signal: ReviewRequestViewerSignal::DismissedApproval,
                    lag_since_unix: None,
                    dismissal: None,
                },
            ],
            external: false,
            review_wait_threshold_seconds: None,
        }],
    };

    let output = render_review_requests(&view, true, None, &BTreeMap::new());

    assert!(output.contains("\x1b[36m?\x1b[0m    —     ◯ Waiting on me"));
    assert!(output.contains("\x1b[38;2;194;95;0m!\x1b[0m    —     ◯ I left comments"));
    assert!(output.contains("\x1b[1m\x1b[31m!\x1b[0m    —     ◯ I requested changes"));
    assert!(output.contains("\x1b[32m✓\x1b[0m    —     ◯ I approved"));
    assert!(output.contains("\x1b[38;2;194;95;0m✓\x1b[0m    —     ◯ I approved with comments"));
    assert!(output.contains("\x1b[38;2;194;95;0m✓\x1b[0m    —     ◯ My approval was dismissed"));
    assert!(output.contains("\x1b[1mexample-author\x1b[0m"));
    assert!(!output.contains("Legend:"));
}

#[test]
fn review_groups_layout_and_external_repositories() {
    // Verifies: review requests are live-fetched, grouped by layout repo first, and render PR health.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![
            review_request("example-owner", "api-alpha", 12),
            review_request("outside-owner", "tooling-lib", 44),
        ],
        pull_request_statuses: BTreeMap::from([
            (12, {
                let mut status =
                    review_status_record(12, "Update alpha endpoint", "example-author", false);
                status.commented_reviewers = vec!["commenting-reviewer".to_owned()];
                status.approved_reviewers = vec!["approving-reviewer".to_owned()];
                status.addressed_reviewers = vec!["addressed-reviewer".to_owned()];
                status
            }),
            (
                44,
                review_status_record(44, "Tighten parser behavior", "outside-author", false),
            ),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(!result.stdout.contains("Review requests for"));
    assert!(result.stdout.contains("api-alpha"));
    assert!(result.stdout.contains("  PR       Chk  Rev  Lag   Title"));
    assert!(result
        .stdout
        .contains("✓    ?    —     ◯ Update alpha endpoint"));
    assert!(result
        .stdout
        .contains("◯ Update alpha endpoint [backend] example-author"));
    assert!(!result.stdout.contains("by example-author"));
    assert!(!result.stdout.contains("peer-reviewer"));
    assert!(!result.stdout.contains("commenting-reviewer"));
    assert!(!result.stdout.contains("approving-reviewer"));
    assert!(!result.stdout.contains("addressed-reviewer"));
    assert!(result.stdout.contains("outside-owner/tooling-lib"));
    assert!(result.stdout.contains("Tighten parser behavior"));
    assert!(!result.stdout.contains("Legend:"));
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![12], vec![44]]
    );
}

#[test]
fn review_uses_pull_request_creation_age_before_viewer_review() {
    // Verifies: brand-new PRs show how long they have been waiting for review.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Needs first review", "example-author", false);
    status.created_at = Some("2099-01-01T00:00:00Z".to_owned());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders creation age");

    assert!(result.stdout.contains("?    <1h   ◯ Needs first review"));
}

#[test]
fn review_hides_pull_requests_dismissed_by_local_action() {
    // Verifies: the PR store action stream can hide a review row without legacy state.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let status = review_status_record(12, "Action dismissed review", "example-author", false);
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            PullRequestWithHistory {
                status,
                history: Vec::new(),
                actions: vec![PullRequestActionRecord {
                    action: "dismiss".to_owned(),
                    source: "manual".to_owned(),
                    reason: Some("manual".to_owned()),
                    changed_at_unix: 1_767_273_000,
                    details_json: serde_json::json!({}),
                }],
            },
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox applies action dismissal");

    assert!(!result.stdout.contains("Action dismissed review"));
}

#[test]
fn review_dismissed_lists_pull_requests_dismissed_by_local_action() {
    // Verifies: dismissed review listing can be seeded from PR-store actions without legacy state.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repository = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "api-alpha".to_owned(),
    };
    PullRequestStore::open(&environment)
        .expect("pull-request store opens")
        .record_pull_request_action(
            &repository,
            12,
            "dismiss",
            "manual",
            Some("manual"),
            serde_json::json!({ "selector": "12" }),
        )
        .expect("dismiss action records");
    let status = review_status_record(12, "Action listed dismissal", "example-author", false);
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            PullRequestWithHistory {
                status,
                history: Vec::new(),
                actions: vec![PullRequestActionRecord {
                    action: "dismiss".to_owned(),
                    source: "manual".to_owned(),
                    reason: Some("manual".to_owned()),
                    changed_at_unix: 1_767_273_000,
                    details_json: serde_json::json!({ "selector": "12" }),
                }],
            },
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review", "dismissed"], &environment, &services)
        .expect("dismissed review output renders action-backed rows");

    assert!(result.stdout.contains("Action listed dismissal"));
    assert!(result.stdout.contains("[jx:dismissed:manual]"));
}

#[test]
fn review_lag_uses_visible_epoch_from_history() {
    // Verifies: review lag is based on the PR history event that made the row visible.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let status = review_status_record(12, "History driven lag", "example-author", false);
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            PullRequestWithHistory {
                status,
                history: vec![PullRequestHistoryRecord {
                    kind: "reviewer_requested".to_owned(),
                    changed_at_unix: chrono::Utc::now().timestamp(),
                    old_json: None,
                    new_json: Some(serde_json::json!({
                        "type": "user",
                        "login": "example-reviewer",
                    })),
                    details_json: serde_json::json!({}),
                }],
                actions: Vec::new(),
            },
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders history lag");

    assert!(result.stdout.contains("?    <1h   ◯ History driven lag"));
}

#[test]
fn review_styles_configured_review_wait_threshold() {
    // Verifies: review inbox uses repo review-wait SLA styling for waiting requests.
    let workspace = review_workspace();
    workspace.write_home_file(
        ".config/jx/10-review-threshold.toml",
        r#"
[[repo.rules]]
repo = "example-owner/api-alpha"

[repo.rules.stack_status]
review_wait_threshold = "4h"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let timestamp = |hours: i64| {
        (chrono::Utc::now() - chrono::Duration::hours(hours))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    let mut old = review_status_record(12, "Old review", "example-author", false);
    old.created_at = Some(timestamp(10));
    old.timeline_events = vec![PullRequestTimelineEvent {
        kind: PullRequestTimelineEventKind::ReviewRequested,
        created_at: timestamp(5),
        reviewer: Some("example-reviewer".to_owned()),
    }];
    let mut fresh = review_status_record(13, "Fresh review", "example-author", false);
    fresh.created_at = Some(timestamp(10));
    fresh.timeline_events = vec![PullRequestTimelineEvent {
        kind: PullRequestTimelineEventKind::ReviewRequested,
        created_at: timestamp(1),
        reviewer: Some("example-reviewer".to_owned()),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![
            review_request("example-owner", "api-alpha", 12),
            review_request("example-owner", "api-alpha", 13),
        ],
        pull_request_statuses: BTreeMap::from([(12, old), (13, fresh)]),
        ..FakeServices::default()
    };
    let prompts = test_prompt_handlers();

    let result = run_with_args_and_progress(
        ["jx", "review"],
        &environment,
        &services,
        &NoProgress,
        prompts,
        OutputMode {
            color: true,
            terminal_width: None,
        },
    )
    .expect("colored review inbox renders");

    assert!(result
        .stdout
        .contains("\x1b[1m\x1b[31m?\x1b[0m    \x1b[1m\x1b[31m5h  \x1b[0m  ◯ Old review"));
    assert!(result
        .stdout
        .contains("\x1b[36m?\x1b[0m    \x1b[2m1h  \x1b[0m  ◯ Fresh review"));
}

#[test]
fn review_filters_pull_requests_authored_by_viewer() {
    // Verifies: self-authored PRs returned by GitHub review search are not part of the review inbox.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![
            review_request("example-owner", "api-alpha", 12),
            review_request("example-owner", "api-alpha", 13),
        ],
        pull_request_statuses: BTreeMap::from([
            (
                12,
                review_status_record(12, "Someone else's PR", "example-author", false),
            ),
            (
                13,
                review_status_record(13, "My PR", "example-reviewer", false),
            ),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(result.stdout.contains("Someone else's PR"));
    assert!(!result.stdout.contains("My PR"));
    assert!(!result.stdout.contains("Review requests for"));
}

#[test]
fn review_auto_dismisses_already_approved_pull_requests_from_prior_activity() {
    // Verifies: PRs discovered from prior reviewer activity disappear once the viewer has approved.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Already approved", "example-author", false);
    status.review_status = PullRequestReviewStatus::Approved;
    status.requested_reviewers = ReviewerSelection::new(["peer-reviewer"], Vec::<String>::new());
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    status.review_activity = vec![PullRequestReviewActivity {
        reviewer: "example-reviewer".to_owned(),
        reviewed_at: "2099-01-01T00:00:00Z".to_owned(),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders approved activity");

    assert!(!result.stdout.contains("Already approved"));
    assert_no_review_dismissal_state_or_log(&workspace);
}

#[test]
fn review_renders_author_display_names() {
    // Verifies: review inbox keeps login-based state but renders cached author names for humans.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let status = review_status_record(12, "Update alpha endpoint", "example-author", false);
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        github_user_display_names: BTreeMap::from([
            ("example-reviewer".to_owned(), "Example Reviewer".to_owned()),
            ("example-author".to_owned(), "Example Author".to_owned()),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(!result.stdout.contains("Review requests for"));
    assert!(result.stdout.contains("Example Author"));
    assert!(!result.stdout.contains("by Example Author"));
    assert!(!result.stdout.contains("example-author"));
}

#[test]
fn review_renders_json_provider_output() {
    // Verifies: external wrappers can consume review inbox data without parsing human tables.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Update alpha endpoint", "example-author", false);
    status.url = None;
    status.created_at = Some("2099-01-01T00:00:00Z".to_owned());
    status.commented_reviewers = vec!["commenting-reviewer".to_owned()];
    status.review_activity = vec![PullRequestReviewActivity {
        reviewer: "example-reviewer".to_owned(),
        reviewed_at: "2099-01-02T00:00:00Z".to_owned(),
    }];
    status.timeline_events = vec![PullRequestTimelineEvent {
        kind: PullRequestTimelineEventKind::ReviewRequested,
        created_at: "2099-01-03T00:00:00Z".to_owned(),
        reviewer: Some("example-reviewer".to_owned()),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        github_user_display_names: BTreeMap::from([
            ("example-reviewer".to_owned(), "Example Reviewer".to_owned()),
            (
                "commenting-reviewer".to_owned(),
                "Commenting Reviewer".to_owned(),
            ),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "review", "--format", "json"],
        &environment,
        &services,
    )
    .expect("review json renders");
    let value: serde_json::Value = serde_json::from_str(&result.stdout).expect("valid json");
    let pull_requests = value["pullRequests"].as_array().expect("pull requests");
    let pull_request = &pull_requests[0];

    assert_eq!(value["command"], "review");
    assert_eq!(value["version"], 1);
    assert_eq!(value["viewer"]["login"], "example-reviewer");
    assert_eq!(value["viewer"]["displayName"], "Example Reviewer");
    assert_eq!(
        value["displayNames"]["commenting-reviewer"],
        "Commenting Reviewer"
    );
    assert_eq!(pull_requests.len(), 1);
    assert_eq!(pull_request["repository"], "example-owner/api-alpha");
    assert_eq!(
        pull_request["repositoryUrl"],
        "https://github.com/example-owner/api-alpha"
    );
    assert_eq!(pull_request["key"], "api-alpha");
    assert_eq!(
        pull_request["root"],
        workspace
            .path()
            .join("projects/api-alpha")
            .display()
            .to_string()
    );
    assert_eq!(pull_request["displayRoot"], "~/projects/api-alpha");
    assert_eq!(pull_request["external"], false);
    assert_eq!(pull_request["number"], 12);
    assert_eq!(
        pull_request["url"],
        "https://github.com/example-owner/api-alpha/pull/12"
    );
    assert_eq!(pull_request["title"], "Update alpha endpoint");
    assert_eq!(pull_request["branch"], "topic/review-12");
    assert_eq!(pull_request["baseBranch"], "main");
    assert_eq!(pull_request["createdAt"], "2099-01-01T00:00:00Z");
    assert_eq!(pull_request["author"], "example-author");
    assert_eq!(pull_request["checkStatus"], "passing");
    assert_eq!(pull_request["mergeStatus"], "mergeable");
    assert_eq!(pull_request["reviewStatus"], "review_requested");
    assert_eq!(pull_request["requestState"], "new");
    assert_eq!(pull_request["labels"][0]["name"], "backend");
    assert_eq!(pull_request["requestedUsers"][0], "example-reviewer");
    assert_eq!(pull_request["commentedUsers"][0], "commenting-reviewer");
    assert_eq!(
        pull_request["reviewActivity"][0]["reviewer"],
        "example-reviewer"
    );
    assert_eq!(
        pull_request["timelineEvents"][0]["kind"],
        "review_requested"
    );
    assert_eq!(pull_request["latestCommitOid"], "commit-12");
}

#[test]
fn review_rewrites_titles_before_rendering() {
    // Verifies: review inbox uses the same repo-scoped title normalization as stack status.
    let workspace = review_workspace();
    workspace.write_home_file(
        ".config/jx/10-review.toml",
        r#"
[[repo.rules]]
repo = "example-owner/*"

[[repo.rules.stack_status.title_rewrites]]
pattern = "^\\[([A-Z]+-[0-9]+)\\] (.+)$"
replace = "$1: $2"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(
            12,
            review_status_record(
                12,
                "[TASK-123] Wire up synthetic endpoint",
                "example-author",
                false,
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(result
        .stdout
        .contains("TASK-123: Wire up synthetic endpoint"));
    assert!(!result.stdout.contains("[TASK-123]"));
}

#[test]
fn review_renders_closed_rows_as_on_ice_without_labels_or_reviewers() {
    // Verifies: inactive PR reminders stay visually quiet in review output.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Closed review reminder", "example-author", false);
    status.closed = true;
    status.labels = vec![PullRequestLabel {
        name: "closed-label".to_owned(),
        color: "5319e7".to_owned(),
    }];
    status.commented_reviewers = vec!["closed-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_progress(
        ["jx", "review"],
        &environment,
        &services,
        &NoProgress,
        test_prompt_handlers(),
        OutputMode {
            color: true,
            terminal_width: None,
        },
    )
    .expect("colored review inbox renders");

    assert!(result.stdout.contains(
        "\x1b[38;2;130;165;218m  \x1b]8;;https://github.com/example-owner/api-alpha/pull/12"
    ));
    assert!(result.stdout.contains("⊖ Closed review reminder\x1b[0m"));
    assert!(!result.stdout.contains("closed-label"));
    assert!(!result.stdout.contains("closed-reviewer"));
}

#[test]
fn review_marks_merge_conflicts_with_prohibited_node_symbol() {
    // Verifies: review rows show merge conflicts compactly without adding title tokens.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Update alpha endpoint", "example-author", false);
    status.merge_status = PullRequestMergeStatus::Conflicting;
    status.labels = Vec::new();
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(result.stdout.contains("⊘ Update alpha endpoint"));
    assert!(!result.stdout.contains("[merge-conflict]"));
}

#[test]
fn review_dismiss_records_current_head_oid() {
    // Verifies: review dismissal stores the reviewed PR version locally without mutating GitHub state.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Stale approved PR", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "review", "dismiss", "12"], &environment, &services)
            .expect("review dismissal succeeds");

    assert_eq!(result.stdout, dismissed_review_output("api-alpha", 12));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
    let log = fs::read_to_string(workspace.home.join(".local/state/jx/review-dismissals.log"))
        .expect("dismissal log writes");
    assert!(log.contains("\"action\":\"dismiss\""));
    assert!(log.contains("\"reason\":\"manual\""));
    assert!(log.contains("\"source\":\"manual\""));
    assert!(log.contains("\"selector\":\"12\""));
    let store = rusqlite::Connection::open(
        workspace
            .home
            .join(".local/state/jx/pull-request-store.sqlite"),
    )
    .expect("pull-request store opens");
    let (action, details): (String, String) = store
        .query_row(
            "SELECT action, details_json FROM pull_request_actions",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("dismiss action records");
    assert_eq!(action, "dismiss");
    assert!(details.contains("\"dismissedHeadOid\":\"commit-12\""));
}

#[test]
fn review_history_renders_snapshot_history_and_visibility_actions() {
    // Verifies: local review history exposes the events that drive dismissal decisions.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repository = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "api-alpha".to_owned(),
    };
    let store = PullRequestStore::open(&environment).expect("pull-request store opens");
    store
        .record_pull_request_snapshots(
            &repository,
            &[review_status_record(
                12,
                "Debug review",
                "example-author",
                false,
            )],
        )
        .expect("snapshot records");
    store
        .record_pull_request_action(
            &repository,
            12,
            "dismiss",
            "manual",
            Some("manual"),
            serde_json::json!({ "selector": "12" }),
        )
        .expect("dismiss action records");
    store
        .record_pull_request_action(
            &repository,
            12,
            "undismiss",
            "manual",
            Some("manual"),
            serde_json::json!({ "selector": "api-alpha#12" }),
        )
        .expect("undismiss action records");

    let result = run_with_args_and_services(
        ["jx", "review", "history", "api-alpha#12"],
        &environment,
        &FakeServices::default(),
    )
    .expect("review history renders");

    assert!(result
        .stdout
        .contains("example-owner/api-alpha#12 Debug review"));
    assert!(result.stdout.contains("history  first_seen"));
    assert!(result
        .stdout
        .contains("action   dismiss source=manual reason=manual"));
    assert!(result
        .stdout
        .contains("action   undismiss source=manual reason=manual"));
}

#[test]
fn review_history_json_renders_actions_for_wrappers() {
    // Verifies: debug tooling can consume local visibility actions without parsing the table.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repository = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "api-alpha".to_owned(),
    };
    PullRequestStore::open(&environment)
        .expect("pull-request store opens")
        .record_pull_request_action(
            &repository,
            12,
            "dismiss",
            "manual",
            Some("manual"),
            serde_json::json!({ "selector": "api-alpha#12" }),
        )
        .expect("dismiss action records");

    let result = run_with_args_and_services(
        [
            "jx",
            "review",
            "--format",
            "json",
            "history",
            "api-alpha#12",
        ],
        &environment,
        &FakeServices::default(),
    )
    .expect("review history JSON renders");
    let value: serde_json::Value = serde_json::from_str(&result.stdout).expect("JSON parses");

    assert_eq!(value["repository"], "example-owner/api-alpha");
    assert_eq!(value["number"], 12);
    assert_eq!(value["entries"][0]["type"], "action");
    assert_eq!(value["entries"][0]["action"], "dismiss");
}

#[test]
fn review_manual_undismiss_overrides_computed_auto_hide() {
    // Verifies: explicit local undismiss keeps a handled PR visible without GitHub mutation.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Approved review", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            PullRequestWithHistory {
                status,
                history: Vec::new(),
                actions: vec![PullRequestActionRecord {
                    action: "undismiss".to_owned(),
                    source: "manual".to_owned(),
                    reason: Some("manual".to_owned()),
                    changed_at_unix: 1_767_273_000,
                    details_json: serde_json::json!({}),
                }],
            },
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders manual undismiss override");

    assert!(result.stdout.contains("Approved review"));
}

#[test]
fn review_automatic_undismiss_does_not_override_computed_auto_hide() {
    // Verifies: automatic resurface cleanup does not act like an explicit manual undismiss.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status =
        review_status_record(12, "Approved after head change", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    let pull_request = PullRequestWithHistory {
        status,
        history: Vec::new(),
        actions: vec![review_action(
            "undismiss",
            "automatic",
            Some("head_changed"),
            serde_json::json!({
                "dismissedHeadOid": "old-commit",
                "currentHeadOid": "commit-12",
            }),
        )],
    };
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(12, pull_request)]),
        ..FakeServices::default()
    };

    let inbox = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox hides handled PR");
    let dismissed =
        run_with_args_and_services(["jx", "review", "dismissed"], &environment, &services)
            .expect("dismissed review inbox renders handled PR");

    assert!(!inbox.stdout.contains("Approved after head change"));
    assert!(dismissed.stdout.contains("Approved after head change"));
    assert!(dismissed.stdout.contains("[jx:dismissed:approved]"));
}

#[test]
fn review_auto_dismisses_approved_pull_requests() {
    // Verifies: viewer-approved PRs disappear from the review inbox without manual cleanup.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Approved review", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(!result.stdout.contains("Approved review"));
    assert_no_review_dismissal_state_or_log(&workspace);
}

#[test]
fn review_auto_dismisses_commented_pull_requests() {
    // Verifies: PRs the viewer already commented on disappear until there is a new signal.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Commented review", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.commented_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(!result.stdout.contains("Commented review"));
    assert_no_review_dismissal_state_or_log(&workspace);
}

#[test]
fn review_auto_dismissed_comment_resurfaces_on_author_response() {
    // Verifies: author replies make a previously commented PR visible instead of re-hiding it immediately.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Author replied", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.commented_reviewers = vec!["example-reviewer".to_owned()];
    status.review_activity = vec![PullRequestReviewActivity {
        reviewer: "example-reviewer".to_owned(),
        reviewed_at: "2026-01-01T12:00:00Z".to_owned(),
    }];
    status.reviewer_responses = vec![PullRequestReviewerResponse {
        reviewer: "example-reviewer".to_owned(),
        responded_at: "2026-01-01T12:45:00Z".to_owned(),
        body_text: "Fixed it".to_owned(),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(
                status,
                vec![review_action(
                    "dismiss",
                    "automatic",
                    Some("commented"),
                    serde_json::json!({
                        "dismissedHeadOid": "commit-12",
                        "dismissedViewerResponseAt": "2026-01-01T12:00:00Z",
                    }),
                )],
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders author response");

    assert!(result.stdout.contains("Author replied"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
    let log = fs::read_to_string(workspace.home.join(".local/state/jx/review-dismissals.log"))
        .expect("auto undismissal log writes");
    assert!(log.contains("\"action\":\"undismiss\""));
    assert!(log.contains("\"reason\":\"author_response\""));
    assert!(log.contains("\"source\":\"automatic\""));
    assert!(!log.contains("\"reason\":\"commented\""));
}

#[test]
fn review_does_not_auto_dismiss_after_active_author_response() {
    // Verifies: an author reply remains visible across refreshes until the viewer acts again.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Author needs follow-up", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    status.review_activity = vec![PullRequestReviewActivity {
        reviewer: "example-reviewer".to_owned(),
        reviewed_at: "2026-01-01T12:00:00Z".to_owned(),
    }];
    status.reviewer_responses = vec![PullRequestReviewerResponse {
        reviewer: "example-reviewer".to_owned(),
        responded_at: "2026-01-01T12:45:00Z".to_owned(),
        body_text: "Could you take another look?".to_owned(),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox keeps active author response visible");

    assert!(result.stdout.contains("Author needs follow-up"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
}

#[test]
fn review_auto_dismisses_draft_pull_requests_without_attention() {
    // Verifies: draft review requests are hidden until there is an explicit attention signal.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let status = review_status_record(12, "Draft work in progress", "example-author", true);
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(!result.stdout.contains("Draft work in progress"));
    assert_no_review_dismissal_state_or_log(&workspace);
}

#[test]
fn review_does_not_auto_dismiss_draft_pull_requests_with_mentions() {
    // Verifies: explicit mentions keep draft PRs visible because someone asked for attention.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Draft mention", "example-author", true);
    status.reviewer_mentions = vec![PullRequestReviewerMention {
        reviewer: "example-reviewer".to_owned(),
        mentioned_at: "2026-01-01T12:00:00Z".to_owned(),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders mentioned draft");

    assert!(result.stdout.contains("Draft mention"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
}

#[test]
fn review_draft_dismissal_ignores_head_changes_until_ready() {
    // Verifies: draft churn stays hidden, but ready-for-review resurfaces the PR.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut draft = review_status_record(12, "Draft keeps changing", "example-author", true);
    draft.latest_commit_oid = Some("commit-new".to_owned());
    let draft_action = review_action(
        "dismiss",
        "automatic",
        Some("draft"),
        serde_json::json!({ "dismissedHeadOid": "commit-old" }),
    );
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(draft.clone(), vec![draft_action.clone()]),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox keeps draft hidden");

    assert!(!result.stdout.contains("Draft keeps changing"));
    let mut ready = draft;
    ready.draft = false;
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(ready, vec![draft_action]),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox resurfaces ready PR");

    assert!(result.stdout.contains("Draft keeps changing"));
    let log = fs::read_to_string(workspace.home.join(".local/state/jx/review-dismissals.log"))
        .expect("ready-for-review undismissal log writes");
    assert!(log.contains("\"action\":\"undismiss\""));
    assert!(log.contains("\"reason\":\"ready_for_review\""));
}

#[test]
fn review_manual_dismissed_draft_uses_draft_policy() {
    // Verifies: manual draft dismissal hides draft churn with the same resurface policy.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let status = review_status_record(12, "Manual draft", "example-author", true);
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "review", "dismiss", "12"], &environment, &services)
            .expect("draft dismissal succeeds");

    assert!(result.stdout.contains("until it is ready for review"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
    let log = fs::read_to_string(workspace.home.join(".local/state/jx/review-dismissals.log"))
        .expect("manual draft dismissal log writes");
    assert!(log.contains("\"action\":\"dismiss\""));
    assert!(log.contains("\"reason\":\"draft\""));
    assert!(log.contains("\"source\":\"manual\""));
}

#[test]
fn review_does_not_auto_dismiss_commented_pull_requests_requested_again() {
    // Verifies: explicit review requests stay visible even if the viewer had already commented.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Requested again", "example-author", false);
    status.commented_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders re-requested comment");

    assert!(result.stdout.contains("?    —     ◯ Requested again"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
}

#[test]
fn review_auto_dismissal_resurfaces_when_approval_state_changes() {
    // Verifies: automatic hidden state only applies while the viewer's review checkmark is green.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Needs review again", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(
                status,
                vec![review_action(
                    "dismiss",
                    "automatic",
                    Some("approved"),
                    serde_json::json!({ "dismissedHeadOid": "commit-12" }),
                )],
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders resurfaced PR");

    assert!(result.stdout.contains("Needs review again"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
    let log = fs::read_to_string(workspace.home.join(".local/state/jx/review-dismissals.log"))
        .expect("auto undismissal log writes");
    assert!(log.contains("\"action\":\"undismiss\""));
    assert!(log.contains("\"reason\":\"viewer_review_state_changed\""));
    assert!(log.contains("\"source\":\"automatic\""));
}

#[test]
fn review_auto_dismissal_refreshes_changed_approved_heads_without_rendering() {
    // Verifies: a changed PR stays hidden when the live review signal is still approved.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Still approved", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    status.latest_commit_oid = Some("commit-new".to_owned());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(
                status,
                vec![review_action(
                    "dismiss",
                    "automatic",
                    Some("approved"),
                    serde_json::json!({ "dismissedHeadOid": "commit-old" }),
                )],
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox refreshes auto dismissal");

    assert!(!result.stdout.contains("Still approved"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
    let log = fs::read_to_string(workspace.home.join(".local/state/jx/review-dismissals.log"))
        .expect("legacy auto dismissal resurface log writes");
    assert!(log.contains("\"action\":\"undismiss\""));
    assert!(log.contains("\"reason\":\"head_changed\""));
    assert!(!log.contains("\"action\":\"dismiss\""));
}

#[test]
fn review_dismiss_log_migrates_legacy_jsonl_name() {
    // Verifies: local audit logging follows the state directory's .log naming convention without losing old entries.
    let workspace = review_workspace();
    workspace.write_home_file(
        ".local/state/jx/review-dismissals.log.jsonl",
        "{\"legacy\":true}\n",
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Stale approved PR", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "review", "dismiss", "12"], &environment, &services)
        .expect("review dismissal succeeds");

    let log = fs::read_to_string(workspace.home.join(".local/state/jx/review-dismissals.log"))
        .expect("dismissal log writes");
    assert!(log.contains("\"legacy\":true"));
    assert!(log.contains("\"action\":\"dismiss\""));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.log.jsonl")
        .exists());
}

#[test]
fn review_dismissed_renders_currently_hidden_pull_requests() {
    // Verifies: dismissed review output reuses the normal review table with computed and local reasons.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut auto_dismissed = review_status_record(12, "Stale approved PR", "example-author", false);
    auto_dismissed.requested_reviewers = ReviewerSelection::default();
    auto_dismissed.approved_reviewers = vec!["example-reviewer".to_owned()];
    let mut manual = review_status_record(13, "Manual dismissal", "example-author", false);
    manual.requested_reviewers = ReviewerSelection::default();
    let mut legacy_auto =
        review_status_record(14, "Legacy auto dismissal", "example-author", false);
    legacy_auto.requested_reviewers = ReviewerSelection::default();
    legacy_auto.approved_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![
            review_request("example-owner", "api-alpha", 12),
            review_request("example-owner", "api-alpha", 13),
            review_request("example-owner", "api-alpha", 14),
        ],
        pull_requests_with_history: BTreeMap::from([
            (
                12,
                review_pull_request_with_actions(auto_dismissed, Vec::new()),
            ),
            (
                13,
                review_pull_request_with_actions(
                    manual,
                    vec![review_dismiss_action(
                        "manual",
                        serde_json::json!({ "dismissedHeadOid": "commit-13" }),
                    )],
                ),
            ),
            (
                14,
                review_pull_request_with_actions(legacy_auto, Vec::new()),
            ),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review", "dismissed"], &environment, &services)
        .expect("dismissed review output renders");

    assert!(!result.stdout.contains("Review requests for"));
    assert!(result.stdout.contains("  PR       Chk  Rev  Lag   Title"));
    assert!(result.stdout.contains("Stale approved PR"));
    assert!(result.stdout.contains("[jx:dismissed:approved]"));
    assert!(result.stdout.contains("Manual dismissal"));
    assert!(result.stdout.contains("[jx:dismissed:manual]"));
    assert!(result.stdout.contains("Legacy auto dismissal"));
    assert!(!result.stdout.contains("[jx:dismissed:automatic]"));
}

#[test]
fn review_undismiss_removes_hidden_pull_request_from_local_state() {
    // Verifies: undismiss returns an actively hidden PR to the normal review inbox without GitHub mutation.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Stale approved PR", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "review", "undismiss", "api-alpha#12"],
        &environment,
        &services,
    )
    .expect("review undismiss succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Undismissed {}\n",
            osc8_link(
                "https://github.com/example-owner/api-alpha/pull/12",
                "example-owner/api-alpha#12",
            )
        )
    );
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
    let log = fs::read_to_string(workspace.home.join(".local/state/jx/review-dismissals.log"))
        .expect("manual undismissal log writes");
    assert!(log.contains("\"action\":\"undismiss\""));
    assert!(log.contains("\"reason\":\"manual\""));
    assert!(log.contains("\"source\":\"manual\""));
}

#[test]
fn review_dismiss_records_viewer_response_watermark() {
    // Verifies: dismissal remembers the latest author response that made the PR actionable.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Answered review comment", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.addressed_reviewers = vec!["example-reviewer".to_owned()];
    status.reviewer_responses = vec![PullRequestReviewerResponse {
        reviewer: "example-reviewer".to_owned(),
        responded_at: "2026-01-01T12:30:00Z".to_owned(),
        body_text: "Fixed it".to_owned(),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "review", "dismiss", "12"], &environment, &services)
        .expect("review dismissal succeeds");

    let store = rusqlite::Connection::open(
        workspace
            .home
            .join(".local/state/jx/pull-request-store.sqlite"),
    )
    .expect("pull-request store opens");
    let details: String = store
        .query_row("SELECT details_json FROM pull_request_actions", [], |row| {
            row.get(0)
        })
        .expect("dismiss action records");
    assert!(details.contains("\"dismissedViewerResponseAt\":\"2026-01-01T12:30:00Z\""));
}

#[test]
fn review_dismiss_matches_repository_suffix_by_full_components() {
    // Verifies: repo#PR dismissal uses full path components, not substring matching.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Stale approved PR", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![
            review_request("example-owner", "api-alpha", 12),
            review_request("example-owner", "api-alpha-tools", 12),
        ],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "review", "dismiss", "api-alpha#12"],
        &environment,
        &services,
    )
    .expect("review dismissal succeeds");

    assert_eq!(result.stdout, dismissed_review_output("api-alpha", 12));
    let store = rusqlite::Connection::open(
        workspace
            .home
            .join(".local/state/jx/pull-request-store.sqlite"),
    )
    .expect("pull-request store opens");
    let repository: String = store
        .query_row(
            "SELECT repositories.owner || '/' || repositories.name
             FROM pull_request_actions
             JOIN pull_requests ON pull_requests.id = pull_request_actions.pr_id
             JOIN repositories ON repositories.id = pull_requests.repository_id",
            [],
            |row| row.get(0),
        )
        .expect("dismiss action records repository");
    assert_eq!(repository, "example-owner/api-alpha");
}

#[test]
fn review_dismiss_accepts_full_github_url() {
    // Verifies: browser URLs normalize to the same suffix matcher as owner/repo#PR selectors.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Stale approved PR", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        [
            "jx",
            "review",
            "dismiss",
            "https://github.com/example-owner/api-alpha/pull/12",
        ],
        &environment,
        &services,
    )
    .expect("review dismissal succeeds");

    assert_eq!(result.stdout, dismissed_review_output("api-alpha", 12));
}

#[test]
fn review_dismiss_reports_ambiguous_bare_number() {
    // Verifies: bare PR numbers stay convenient only when they identify one review row.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Stale approved PR", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![
            review_request("example-owner", "api-alpha", 12),
            review_request("sample-owner", "api-beta", 12),
        ],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let error =
        run_with_args_and_services(["jx", "review", "dismiss", "12"], &environment, &services)
            .expect_err("ambiguous review dismissal is rejected");

    assert!(error
        .to_string()
        .contains("matches: example-owner/api-alpha#12, sample-owner/api-beta#12"));
    assert!(error
        .to_string()
        .contains("use a longer repo suffix such as owner/repo#number"));
}

#[test]
fn review_hides_dismissed_pr_until_head_changes() {
    // Verifies: dismissed reviewed PRs stay hidden only for the exact reviewed commit.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut dismissed = review_status_record(12, "Stale approved PR", "example-author", false);
    dismissed.requested_reviewers = ReviewerSelection::default();
    dismissed.approved_reviewers = vec!["example-reviewer".to_owned()];
    let dismiss_action = review_dismiss_action(
        "manual",
        serde_json::json!({ "dismissedHeadOid": "commit-12" }),
    );
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(dismissed.clone(), vec![dismiss_action.clone()]),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(!result.stdout.contains("Stale approved PR"));
    let mut changed = dismissed;
    changed.latest_commit_oid = Some("commit-new".to_owned());
    changed.approved_reviewers = Vec::new();
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(changed, vec![dismiss_action]),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox resurfaces changed PR");

    assert!(result.stdout.contains("Stale approved PR"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
}

#[test]
fn review_marks_dismissed_viewer_approval_from_history() {
    // Verifies: stale approvals remain visible as prior approval signal after local undismissal.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Approval needs refresh", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.dismissed_reviewers = vec!["example-reviewer".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            PullRequestWithHistory {
                status,
                history: vec![PullRequestHistoryRecord {
                    kind: "review_state_changed".to_owned(),
                    changed_at_unix: 1_767_273_000,
                    old_json: Some(
                        serde_json::json!({ "reviewer": "example-reviewer", "state": "approved" }),
                    ),
                    new_json: Some(
                        serde_json::json!({ "reviewer": "example-reviewer", "state": "dismissed" }),
                    ),
                    details_json: serde_json::json!({}),
                }],
                actions: Vec::new(),
            },
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox marks dismissed approval");

    assert!(result.stdout.contains("✓"));
    assert!(result.stdout.contains("Approval needs refresh"));
}

#[test]
fn review_dismissal_keeps_watermarked_author_response_hidden() {
    // Verifies: a response already recorded in dismissal state does not make approved PRs bounce visible.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status =
        review_status_record(12, "Already handled author reply", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    status.review_activity = vec![PullRequestReviewActivity {
        reviewer: "example-reviewer".to_owned(),
        reviewed_at: "2026-01-01T12:00:00Z".to_owned(),
    }];
    status.reviewer_responses = vec![PullRequestReviewerResponse {
        reviewer: "example-reviewer".to_owned(),
        responded_at: "2026-01-01T12:45:00Z".to_owned(),
        body_text: "Follow-up before dismissal".to_owned(),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(
                status,
                vec![review_action(
                    "dismiss",
                    "automatic",
                    Some("approved"),
                    serde_json::json!({
                        "dismissedHeadOid": "commit-12",
                        "dismissedViewerResponseAt": "2026-01-01T12:45:00Z",
                    }),
                )],
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox keeps watermarked response hidden");

    assert!(!result.stdout.contains("Already handled author reply"));
    assert_no_review_dismissal_state_or_log(&workspace);
}

#[test]
fn review_dismissal_ignores_configured_author_response_comments() {
    // Verifies: command-only author comments do not resurface a locally dismissed review.
    let workspace = review_workspace();
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo.review]
ignored_author_response_comments = ["^/automation merge\\s*$"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Command-only author reply", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.commented_reviewers = vec!["example-reviewer".to_owned()];
    status.reviewer_responses = vec![PullRequestReviewerResponse {
        reviewer: "example-reviewer".to_owned(),
        responded_at: "2026-01-01T12:45:00Z".to_owned(),
        body_text: "/automation merge".to_owned(),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(
                status,
                vec![review_dismiss_action(
                    "manual",
                    serde_json::json!({
                        "dismissedHeadOid": "commit-12",
                        "dismissedViewerResponseAt": "2026-01-01T12:30:00Z",
                    }),
                )],
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox filters command-only author response");

    assert!(!result.stdout.contains("Command-only author reply"));
    assert_no_review_dismissal_state_or_log(&workspace);
}

#[test]
fn review_dismissal_resurfaces_new_author_response() {
    // Verifies: author replies after dismissal make previously addressed review comments actionable again.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Answered review comment", "example-author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.addressed_reviewers = vec!["example-reviewer".to_owned()];
    status.review_activity = vec![PullRequestReviewActivity {
        reviewer: "example-reviewer".to_owned(),
        reviewed_at: "2026-01-01T12:00:00Z".to_owned(),
    }];
    status.reviewer_responses = vec![PullRequestReviewerResponse {
        reviewer: "example-reviewer".to_owned(),
        responded_at: "2026-01-01T12:45:00Z".to_owned(),
        body_text: "Fixed it".to_owned(),
    }];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            review_pull_request_with_actions(
                status,
                vec![review_dismiss_action(
                    "manual",
                    serde_json::json!({
                        "dismissedHeadOid": "commit-12",
                        "dismissedViewerResponseAt": "2026-01-01T12:30:00Z",
                    }),
                )],
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox resurfaces new author response");

    assert!(result.stdout.contains("Answered review comment"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
    let log = fs::read_to_string(workspace.home.join(".local/state/jx/review-dismissals.log"))
        .expect("automated undismissal log writes");
    assert!(log.contains("\"action\":\"undismiss\""));
    assert!(log.contains("\"reason\":\"author_response\""));
    assert!(log.contains("\"source\":\"automatic\""));
    assert!(log.contains("\"dismissed_viewer_response_at\":\"2026-01-01T12:30:00Z\""));
    assert!(log.contains("\"current_viewer_response_at\":\"2026-01-01T12:45:00Z\""));
}

#[test]
fn review_removes_dismissal_when_pr_disappears_from_inbox() {
    // Verifies: review loading no longer depends on legacy TOML dismissal state.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: Vec::new(),
        pull_request_statuses: BTreeMap::new(),
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox cleans stale dismissal");

    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
}

#[test]
fn review_dismissal_resurfaces_fresh_review_request() {
    // Verifies: a direct re-request beats dismissal even when the PR branch did not change.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let status = review_status_record(12, "Review requested again", "example-author", false);
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            PullRequestWithHistory {
                status,
                history: vec![PullRequestHistoryRecord {
                    kind: "reviewer_requested".to_owned(),
                    changed_at_unix: 1_767_273_100,
                    old_json: None,
                    new_json: Some(serde_json::json!({ "login": "example-reviewer" })),
                    details_json: serde_json::json!({}),
                }],
                actions: vec![review_dismiss_action(
                    "manual",
                    serde_json::json!({ "dismissedHeadOid": "commit-12" }),
                )],
            },
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox resurfaces requested PR");

    assert!(result.stdout.contains("Review requested again"));
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
}

#[test]
fn review_ellipsizes_long_titles_before_labels_and_author() {
    // Verifies: review rows keep labels and the author visible after title shortening.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let status = review_status_record(
        12,
        "Implement a very long synthetic review title that demonstrates the compact subject width convention",
        "example-author",
        false,
    );
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(result
        .stdout
        .contains("Implement a very long synthetic review title that demonstrates the comp…"));
    assert!(result.stdout.contains("[backend] example-author"));
    assert!(!result.stdout.contains("compact subject width convention"));
}

#[test]
fn review_reports_progress_while_loading_requests() {
    // Verifies: slow live review inbox loading has visible progress like stack status.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(
            12,
            review_status_record(12, "Update alpha endpoint", "example-author", false),
        )]),
        ..FakeServices::default()
    };
    let progress = RecordingProgress::default();

    let result = run_with_args_and_progress(
        ["jx", "review"],
        &environment,
        &services,
        &progress,
        test_prompt_handlers(),
        OutputMode::plain(),
    )
    .expect("review inbox renders");

    assert!(result.stdout.contains("Update alpha endpoint"));
    assert_eq!(
        progress.messages(),
        [
            "Loading review context…",
            "Loading review requests…",
            "  0% Loading pull request details…",
            "100% Loading pull request details…",
            "Loading reviewer names…",
        ]
    );
    assert!(progress.finished.get());
}

#[test]
fn review_ellipsizes_rows_to_terminal_width() {
    // Verifies: long review inbox rows stay within the detected terminal width.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(
            12,
            review_status_record(
                12,
                "Implement a very long synthetic review title that must be truncated",
                "example-author",
                false,
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_progress(
        ["jx", "review"],
        &environment,
        &services,
        &NoProgress,
        test_prompt_handlers(),
        OutputMode::plain_with_width(64),
    )
    .expect("review inbox renders");

    let row = result
        .stdout
        .lines()
        .find(|line| line.contains("#12"))
        .expect("review row renders");
    assert!(row.contains('…'));
    assert!(rendered_visible_width(row) <= 64);
}

#[test]
fn review_filters_repositories_before_fetching_details() {
    // Verifies: positional filters narrow the live review inbox before detail GraphQL requests run.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        review_requests: vec![
            review_request("example-owner", "api-alpha", 12),
            review_request("outside-owner", "tooling-lib", 44),
        ],
        pull_request_statuses: BTreeMap::from([
            (
                12,
                review_status_record(12, "Update alpha endpoint", "example-author", false),
            ),
            (
                44,
                review_status_record(44, "Tighten parser behavior", "outside-author", false),
            ),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review", "api-*"], &environment, &services)
        .expect("filtered review inbox renders");

    assert!(result.stdout.contains("api-alpha"));
    assert!(!result.stdout.contains("tooling-lib"));
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![12]]
    );
}

#[test]
fn review_applies_configured_review_gate_policy() {
    // Verifies: review inbox uses the same repo-specific check policy as stack status.
    let workspace = review_workspace();
    workspace.write_home_file(
        ".config/jx/10-review.toml",
        r#"
[[repo.rules]]
repo = "example-owner/api-alpha"

[repo.rules.stack_status]
review_gate_checks = ["^approval gate$"]

[[repo.rules.stack_status.ignored_checks]]
name = "^ci/noisy-advisory$"

[[repo.rules.stack_status.ignored_labels]]
name = "generated-noise"

[[repo.rules.stack_status.ignored_reviewers]]
name = "^ignored-.*$"

[repo.rules.review]
ignored_labels = ["review-only-noise"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Needs approval", "example-author", false);
    status.check_status = PullRequestCheckStatus::Failing;
    status.review_status = PullRequestReviewStatus::NotReviewed;
    status.labels = vec![
        PullRequestLabel {
            name: "useful-label".to_owned(),
            color: "0e8a16".to_owned(),
        },
        PullRequestLabel {
            name: "generated-noise".to_owned(),
            color: "5319e7".to_owned(),
        },
        PullRequestLabel {
            name: "review-only-noise".to_owned(),
            color: "5319e7".to_owned(),
        },
    ];
    status.commented_reviewers = vec!["ignored-bot".to_owned()];
    status.checks = vec![
        PullRequestCheck {
            name: "approval gate".to_owned(),
            status: PullRequestCheckStatus::Failing,
        },
        PullRequestCheck {
            name: "ci/noisy-advisory".to_owned(),
            status: PullRequestCheckStatus::Failing,
        },
    ];
    let services = FakeServices {
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox applies policy");

    assert!(result.stdout.contains("Needs approval [useful-label]"));
    assert!(result.stdout.contains("—    ?    —     ◯ Needs approval"));
    assert!(!result.stdout.contains("generated-noise"));
    assert!(!result.stdout.contains("review-only-noise"));
    assert!(!result.stdout.contains("ignored-bot"));
}

fn review_action(
    action: &str,
    source: &str,
    reason: Option<&str>,
    details_json: serde_json::Value,
) -> PullRequestActionRecord {
    PullRequestActionRecord {
        action: action.to_owned(),
        source: source.to_owned(),
        reason: reason.map(str::to_owned),
        changed_at_unix: 1_767_273_000,
        details_json,
    }
}

fn review_dismiss_action(reason: &str, details_json: serde_json::Value) -> PullRequestActionRecord {
    review_action("dismiss", "manual", Some(reason), details_json)
}

fn review_pull_request_with_actions(
    status: PullRequestStatusRecord,
    actions: Vec<PullRequestActionRecord>,
) -> PullRequestWithHistory {
    PullRequestWithHistory {
        status,
        history: Vec::new(),
        actions,
    }
}

fn assert_no_review_dismissal_state_or_log(workspace: &TestWorkspace) {
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.toml")
        .exists());
    assert!(!workspace
        .home
        .join(".local/state/jx/review-dismissals.log")
        .exists());
}

fn review_workspace() -> TestWorkspace {
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
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api-alpha.git
"#,
    );
    workspace
}

fn review_request(owner: &str, repo: &str, number: u64) -> PullRequestReviewRequest {
    PullRequestReviewRequest {
        repository: GitHubRepository {
            owner: owner.to_owned(),
            name: repo.to_owned(),
        },
        number,
    }
}

fn dismissed_review_output(repo: &str, number: u64) -> String {
    format!(
        "Dismissed {} until new commits, a fresh review request, or a new author response\n",
        osc8_link(
            &format!("https://github.com/example-owner/{repo}/pull/{number}"),
            &format!("example-owner/{repo}#{number}"),
        )
    )
}

fn review_status_record(
    number: u64,
    title: &str,
    author: &str,
    draft: bool,
) -> PullRequestStatusRecord {
    PullRequestStatusRecord {
        number,
        title: title.to_owned(),
        url: Some(format!(
            "https://github.com/example-owner/api-alpha/pull/{number}"
        )),
        created_at: None,
        head_branch: format!("topic/review-{number}"),
        base_branch: "main".to_owned(),
        default_branch: Some("main".to_owned()),
        author: Some(author.to_owned()),
        draft,
        merged: false,
        closed: false,
        merged_at: None,
        closed_at: None,
        check_status: PullRequestCheckStatus::Passing,
        checks: vec![PullRequestCheck {
            name: "unit tests".to_owned(),
            status: PullRequestCheckStatus::Passing,
        }],
        merge_status: PullRequestMergeStatus::Mergeable,
        review_status: PullRequestReviewStatus::ReviewRequested,
        auto_merge_status: PullRequestAutoMergeStatus::NotConfigured,
        requested_reviewers: ReviewerSelection::new(
            ["example-reviewer", "peer-reviewer"],
            Vec::<String>::new(),
        ),
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
        labels: vec![PullRequestLabel {
            name: "backend".to_owned(),
            color: "5319e7".to_owned(),
        }],
        latest_commit_oid: Some(format!("commit-{number}")),
    }
}
