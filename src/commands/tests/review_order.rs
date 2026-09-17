use super::*;

#[test]
fn review_orders_exact_lag_within_unchanged_repository_groups() {
    let workspace = review_workspace();
    let beta = workspace.create_jj_workspace("projects/api-beta");
    TestWorkspace::write_git_config_at(
        &beta,
        "[remote \"origin\"]\n    url = https://github.com/example-owner/api-beta.git\n",
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let since = (chrono::Utc::now() - chrono::Duration::hours(6) - chrono::Duration::minutes(10))
        .timestamp();
    let mut fallback_created = review_with_lag(15, None);
    fallback_created.status.created_at = Some(
        chrono::DateTime::from_timestamp(since, 500_000_000)
            .expect("valid timestamp")
            .to_rfc3339(),
    );
    let mut fallback_requested = review_with_lag(20, None);
    fallback_requested.status.created_at = Some(lag_date(since - 86_400));
    fallback_requested.status.timeline_events = vec![review_requested_at(since + 1)];
    let mut history_overrides_creation = review_with_lag(80, Some(since + 2));
    history_overrides_creation.status.created_at = Some(lag_date(since - 172_800));
    let pull_requests = [
        review_with_lag(12, Some(since)),
        fallback_created,
        fallback_requested,
        history_overrides_creation,
        review_with_lag(30, Some(since + 3)),
        review_with_lag(40, Some(since + 3)),
        review_with_lag(90, None),
        review_with_lag(99, Some(i64::MAX)),
        review_with_lag(7, Some(since - 86_400)),
        review_with_lag(8, Some(since - 172_800)),
        review_with_lag(9, Some(since - 864_000)),
        review_with_lag(10, Some(since - 1_728_000)),
    ];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: pull_requests
            .iter()
            .rev()
            .map(|pull_request| {
                let number = pull_request.status.number;
                let (owner, repository) = match number {
                    7 | 8 => ("example-owner", "api-beta"),
                    9 => ("aaa-owner", "external"),
                    10 => ("zzz-owner", "external"),
                    _ => ("example-owner", "api-alpha"),
                };
                review_request(owner, repository, number)
            })
            .collect(),
        pull_requests_with_history: pull_requests
            .into_iter()
            .map(|pull_request| (pull_request.status.number, pull_request))
            .collect(),
        ..FakeServices::default()
    };
    let expected = vec![12, 15, 20, 80, 40, 30, 99, 90, 8, 7, 9, 10];

    let result = run_with_args_and_services(["jx", "review"], &environment, &services)
        .expect("human review renders");
    let rows = result
        .stdout
        .lines()
        .filter_map(|line| {
            let number = line
                .split_whitespace()
                .next()?
                .strip_prefix('#')?
                .parse::<u64>()
                .ok()?;
            Some((number, line))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rows.iter().map(|(number, _)| *number).collect::<Vec<_>>(),
        expected
    );
    for (number, row) in &rows[..8] {
        if [90, 99].contains(number) {
            assert!(row.contains("—     ◯"), "{row}");
        } else {
            assert!(row.contains("7h"), "{row}");
        }
    }

    let result = run_with_args_and_services(
        ["jx", "review", "--format", "json"],
        &environment,
        &services,
    )
    .expect("JSON review renders");
    let output: serde_json::Value = serde_json::from_str(&result.stdout).expect("valid JSON");
    let rows = output["pullRequests"].as_array().expect("review rows");
    assert_eq!(review_numbers(&output), expected);
    assert_eq!(rows[0]["repository"], "example-owner/api-alpha");
    assert_eq!(rows[8]["repository"], "example-owner/api-beta");
    assert_eq!(rows[10]["repository"], "aaa-owner/external");
    assert_eq!(rows[11]["repository"], "zzz-owner/external");
}

#[test]
fn cached_review_uses_the_same_lag_order_without_fetching() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repository = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "api-alpha".to_owned(),
    };
    // Keep request events later than the store-generated first_seen timestamp without sleeping.
    let since = chrono::Utc::now().timestamp() + 86_400;
    let statuses = [(12, since), (30, since + 60), (40, since + 60)].map(|(number, timestamp)| {
        let mut status = review_with_lag(number, None).status;
        status.timeline_events = vec![review_requested_at(timestamp)];
        status
    });
    let store = PullRequestStore::open(&environment).expect("store opens");
    store
        .record_review_inbox_snapshot(&PullRequestReviewRequests {
            viewer: crate::github::AuthenticatedUser {
                login: "example-reviewer".to_owned(),
            },
            requests: statuses
                .iter()
                .map(|status| review_request("example-owner", "api-alpha", status.number))
                .collect(),
        })
        .expect("inbox records");
    store
        .record_pull_request_snapshots(&repository, &statuses)
        .expect("snapshots record");
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "review", "--cached", "--format", "json"],
        &environment,
        &services,
    )
    .expect("cached review renders");
    let output = serde_json::from_str(&result.stdout).expect("valid JSON");
    assert_eq!(review_numbers(&output), vec![12, 40, 30]);
    assert!(services.pull_request_status_calls.borrow().is_empty());
}

#[test]
fn review_help_describes_lag_order_within_repository_groups() {
    let help = cli()
        .try_get_matches_from(["jx", "review", "--help"])
        .expect_err("help prints")
        .to_string();
    let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(help.contains("Repository groups keep their configured-first alphabetical order"));
    assert!(help.contains("PRs with the longest review lag appear first"));
    assert!(help.contains("unknown lag comes last"));
}

fn review_with_lag(number: u64, since: Option<i64>) -> PullRequestWithHistory {
    PullRequestWithHistory {
        status: review_status_record(number, &format!("Review {number}"), "author", false),
        history: since
            .map(|timestamp| PullRequestHistoryRecord {
                kind: "reviewer_requested".to_owned(),
                changed_at_unix: timestamp,
                old_json: None,
                new_json: Some(serde_json::json!({ "login": "example-reviewer" })),
                details_json: serde_json::json!({}),
            })
            .into_iter()
            .collect(),
        actions: Vec::new(),
    }
}

fn review_requested_at(timestamp: i64) -> PullRequestTimelineEvent {
    PullRequestTimelineEvent {
        kind: PullRequestTimelineEventKind::ReviewRequested,
        created_at: lag_date(timestamp),
        reviewer: Some("example-reviewer".to_owned()),
    }
}

fn lag_date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .expect("valid timestamp")
        .to_rfc3339()
}

fn review_numbers(output: &serde_json::Value) -> Vec<u64> {
    output["pullRequests"]
        .as_array()
        .expect("review rows")
        .iter()
        .map(|row| row["number"].as_u64().expect("PR number"))
        .collect()
}
