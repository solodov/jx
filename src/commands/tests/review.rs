use super::*;

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
    assert_eq!(request.repo_filters, vec!["api-*".to_owned()]);
    assert!(request.interactive);
    assert_eq!(request.refresh_seconds, 15);
    assert_eq!(request.format, ReviewFormat::Human);
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
                review_status_record(44, "Tighten parser behavior", "outside-author", true),
            ),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(result
        .stdout
        .contains("Review requests for example-reviewer: 2 pull requests across 2 repositories"));
    assert!(result.stdout.contains("api-alpha"));
    assert!(result.stdout.contains("  PR       Chk  Req  Lag   Title"));
    assert!(result
        .stdout
        .contains("✓    ◷    —     ◯ Update alpha endpoint"));
    assert!(!result.stdout.contains("@example-author"));
    assert!(!result.stdout.contains("peer-reviewer"));
    assert!(result.stdout.contains("commenting-reviewer"));
    assert!(result.stdout.contains("approving-reviewer"));
    assert!(result.stdout.contains("addressed-reviewer"));
    assert!(result.stdout.contains("outside-owner/tooling-lib"));
    assert!(result.stdout.contains("Tighten parser behavior"));
    assert!(result
        .stdout
        .contains("Legend:\n  Title: ◯ ready, ◌ draft; labels/reviewer activity follow title"));
    assert!(result
        .stdout
        .contains("Req: ◷ requested, ! commented, ✓ approved"));
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

    assert!(result.stdout.contains("◷    <1h   ◯ Needs first review"));
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
        .contains("\x1b[1m\x1b[31m◷\x1b[0m    \x1b[1m\x1b[31m5h  \x1b[0m  ◯ Old review"));
    assert!(result
        .stdout
        .contains("\x1b[36m◷\x1b[0m    \x1b[2m1h  \x1b[0m  ◯ Fresh review"));
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
    assert!(result.stdout.contains("1 pull request across 1 repository"));
}

#[test]
fn review_keeps_already_approved_pull_requests_visible() {
    // Verifies: PRs discovered from prior reviewer activity render as approved instead of disappearing.
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

    assert!(result.stdout.contains("✓    ✓    <1h   ◯ Already approved"));
}

#[test]
fn review_renders_reviewer_display_names() {
    // Verifies: review inbox keeps login-based state but renders cached public names for humans.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(12, "Update alpha endpoint", "example-author", false);
    status.commented_reviewers = vec!["commenting-reviewer".to_owned()];
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

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("review inbox renders");

    assert!(result
        .stdout
        .contains("Review requests for Example Reviewer"));
    assert!(result.stdout.contains("Commenting Reviewer"));
    assert!(!result.stdout.contains("commenting-reviewer"));
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
fn review_ellipsizes_long_titles_before_labels_and_reviewer_activity() {
    // Verifies: review rows keep labels/reviewer activity visible after title shortening.
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = review_status_record(
        12,
        "Implement a very long synthetic review title that demonstrates the compact subject width convention",
        "example-author",
        false,
    );
    status.commented_reviewers = vec!["commenting-reviewer".to_owned()];
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
    assert!(result.stdout.contains("[backend] commenting-reviewer"));
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

    assert!(result
        .stdout
        .contains("Review requests for example-reviewer"));
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
review_gate_checks = ["approval gate"]

[[repo.rules.stack_status.ignored_checks]]
name = "^ci/noisy-advisory$"

[[repo.rules.stack_status.ignored_labels]]
name = "generated-noise"

[[repo.rules.stack_status.ignored_reviewers]]
name = "ignored-bot"
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
    assert!(result.stdout.contains("—    ◷    —     ◯ Needs approval"));
    assert!(!result.stdout.contains("generated-noise"));
    assert!(!result.stdout.contains("ignored-bot"));
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
            "https://github.com/example-owner/example-repo/pull/{number}"
        )),
        created_at: None,
        head_branch: format!("topic/review-{number}"),
        base_branch: "main".to_owned(),
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
        review_status: PullRequestReviewStatus::ReviewRequested,
        requested_reviewers: ReviewerSelection::new(
            ["example-reviewer", "peer-reviewer"],
            Vec::<String>::new(),
        ),
        suggested_reviewers: Vec::new(),
        approved_reviewers: Vec::new(),
        commented_reviewers: Vec::new(),
        addressed_reviewers: Vec::new(),
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
