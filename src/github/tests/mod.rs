use super::*;

#[test]
fn same_repository_head_keeps_user_scoped_branch_namespace() {
    // Verifies: Same-repository PR heads preserve user-scoped branch namespaces.
    let head = PullRequestHead::same_repository("example-owner", "example-user/abc-123-00-a1b2c3d");

    assert_eq!(
        head.label(),
        "example-owner:example-user/abc-123-00-a1b2c3d"
    );
}

#[test]
fn label_application_is_normalized() {
    // Verifies: Label requests trim names, drop empties, and deduplicate entries.
    let labels = LabelApplyResult {
        labels: normalize_names([" bug ", "", "bug", "help wanted"]),
    };

    assert_eq!(
        labels.labels,
        vec!["bug".to_owned(), "help wanted".to_owned()]
    );
}

#[test]
fn reviewer_selection_is_normalized() {
    // Verifies: Reviewer selection trims names, drops empties, and deduplicates entries.
    let selection = ReviewerSelection::new(
        [
            "example-reviewer",
            "",
            "example-reviewer",
            "second-reviewer",
        ],
        ["example-team", " example-team "],
    );

    assert_eq!(
        selection.users,
        vec!["example-reviewer".to_owned(), "second-reviewer".to_owned()]
    );
    assert_eq!(selection.teams, vec!["example-team".to_owned()]);
}

#[test]
fn reviewer_difference_preserves_sorted_left_order() {
    // Verifies: Reviewer set differences preserve the normalized left-hand order.
    let missing = difference(
        &["example-a".to_owned(), "example-b".to_owned()],
        &["example-b".to_owned()],
    );

    assert_eq!(missing, vec!["example-a".to_owned()]);
}

#[test]
fn token_source_build_rejects_missing_token() {
    // Verifies: GitHub client construction rejects a missing token source.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let error = match OctocrabGitHubClient::from_token_source(&TokenSource::Missing, &environment) {
        Ok(_) => panic!("token is required"),
        Err(error) => error,
    };

    assert!(matches!(error, GitHubError::MissingToken));
}

#[test]
fn api_response_error_maps_empty_auth_body_without_octocrab_backtrace() {
    // Verifies: Empty GitHub auth failures stay actionable instead of exposing octocrab internals.
    let error = api_response_error("load authenticated user", 401, "");

    assert!(matches!(
        error,
        GitHubError::AuthenticationFailed {
            operation: "load authenticated user",
            ref message,
        } if message == "HTTP 401: empty response body"
    ));
    assert_eq!(
        error.to_string(),
        "GitHub authentication failed while trying to load authenticated user: HTTP 401: empty response body"
    );
    assert!(!error.to_string().contains("Found at"));
}

#[test]
fn api_response_error_preserves_github_auth_message() {
    // Verifies: GitHub JSON error bodies keep their concise server-provided message.
    let error = api_response_error(
        "load authenticated user",
        403,
        r#"{"message":"Resource not accessible by integration"}"#,
    );

    assert!(matches!(
        error,
        GitHubError::AuthenticationFailed {
            operation: "load authenticated user",
            ref message,
        } if message == "HTTP 403: Resource not accessible by integration"
    ));
}

#[test]
fn token_source_reads_discovered_environment_value() {
    // Verifies: Token lookup reads the discovered environment value without storing secrets.
    let environment = RuntimeEnvironment::new(
        "/workspace",
        [("JX_GITHUB_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let token_source = TokenSource::Environment("JX_GITHUB_TOKEN");

    assert_eq!(
        token_source.token(&environment).expect("token reads"),
        Some("placeholder-token".to_owned())
    );
}

#[test]
fn deserializes_minimal_compare_response() {
    // Verifies: Compare deserialization ignores nested fields GitHub may omit.
    let comparison: CompareCommitsResponse = serde_json::from_str(
        r#"
{
  "status": "behind",
  "ahead_by": 0,
  "behind_by": 3,
  "base_commit": { "sha": "1111222233334444" }
}
"#,
    )
    .expect("compare response deserializes");

    assert_eq!(
        map_comparison_status(comparison.status),
        ComparisonStatus::Behind
    );
    assert_eq!(comparison.ahead_by, 0);
    assert_eq!(comparison.behind_by, 3);
}

#[test]
fn maps_pull_request_status_rollup_and_review_decision() {
    // Verifies: GraphQL PR status facts collapse to stable jx check/review labels.
    let status = map_graphql_pull_request_status(GraphQlPullRequestStatus {
        number: 42,
        title: "Example change".to_owned(),
        url: "https://github.com/example-owner/example-repo/pull/42".to_owned(),
        head_ref_name: "topic/example".to_owned(),
        base_ref_name: "main".to_owned(),
        is_draft: false,
        merged: false,
        closed: false,
        review_decision: Some("CHANGES_REQUESTED".to_owned()),
        review_requests: GraphQlReviewRequests {
            total_count: 1,
            nodes: vec![GraphQlReviewRequestNode {
                requested_reviewer: Some(GraphQlRequestedReviewer {
                    type_name: "User".to_owned(),
                    login: Some("reviewer-one".to_owned()),
                }),
            }],
        },
        commits: GraphQlPullRequestStatusCommits {
            nodes: vec![GraphQlPullRequestStatusCommitNode {
                commit: GraphQlPullRequestStatusCommit {
                    oid: "aaaabbbbccccdddd".to_owned(),
                    status_check_rollup: Some(GraphQlStatusCheckRollup {
                        state: "FAILURE".to_owned(),
                    }),
                },
            }],
        },
    });

    assert_eq!(status.check_status, PullRequestCheckStatus::Failing);
    assert_eq!(
        status.review_status,
        PullRequestReviewStatus::ChangesRequested
    );
    assert_eq!(
        status.requested_reviewers.users,
        ["reviewer-one".to_owned()]
    );
    assert_eq!(
        status.latest_commit_oid.as_deref(),
        Some("aaaabbbbccccdddd")
    );
}

#[test]
fn pull_request_status_query_batches_numbers_with_aliases() {
    // Verifies: stack status can fetch several PRs in one GraphQL request.
    let query = pull_request_status_query(&[41, 42]);

    assert!(query.contains("pr0: pullRequest(number: 41)"));
    assert!(query.contains("pr1: pullRequest(number: 42)"));
    assert!(query.contains("statusCheckRollup"));
    assert!(query.contains("reviewDecision"));
    assert!(query.contains("reviewRequests"));
    assert!(query.contains("totalCount"));
    assert!(query.contains("requestedReviewer"));
    assert!(!query.contains("slug"));
}

#[test]
fn maps_compare_status_to_domain_status() {
    // Verifies: GitHub compare status strings map to stable domain states.
    assert_eq!(
        map_comparison_status(CompareCommitsStatus::Ahead),
        ComparisonStatus::Ahead
    );
    assert_eq!(
        map_comparison_status(CompareCommitsStatus::Behind),
        ComparisonStatus::Behind
    );
    assert_eq!(
        map_comparison_status(CompareCommitsStatus::Diverged),
        ComparisonStatus::Diverged
    );
    assert_eq!(
        map_comparison_status(CompareCommitsStatus::Identical),
        ComparisonStatus::Identical
    );
}
