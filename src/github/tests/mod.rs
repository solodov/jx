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
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        head_ref_name: "topic/example".to_owned(),
        base_ref_name: "main".to_owned(),
        author: Some(GraphQlReviewAuthor {
            login: "change-author".to_owned(),
        }),
        is_draft: false,
        merged: false,
        closed: false,
        merged_at: None,
        closed_at: None,
        review_decision: Some("CHANGES_REQUESTED".to_owned()),
        review_requests: GraphQlReviewRequests {
            total_count: 1,
            nodes: vec![GraphQlReviewRequestNode {
                requested_reviewer: Some(GraphQlRequestedReviewer {
                    type_name: "User".to_owned(),
                    login: Some("reviewer-one".to_owned()),
                    slug: None,
                }),
            }],
        },
        suggested_reviewers: vec![GraphQlSuggestedReviewer {
            reviewer: Some(GraphQlSuggestedReviewerUser {
                login: "reviewer-suggested".to_owned(),
            }),
        }],
        labels: GraphQlLabels {
            nodes: vec![GraphQlLabelNode {
                name: "bug".to_owned(),
                color: "d73a4a".to_owned(),
            }],
        },
        latest_reviews: GraphQlReviews {
            nodes: vec![
                GraphQlReviewNode {
                    state: "APPROVED".to_owned(),
                    submitted_at: Some("2026-01-02T03:04:05Z".to_owned()),
                    author: Some(GraphQlReviewAuthor {
                        login: "reviewer-approved".to_owned(),
                    }),
                },
                GraphQlReviewNode {
                    state: "COMMENTED".to_owned(),
                    submitted_at: Some("2026-01-02T03:04:05Z".to_owned()),
                    author: Some(GraphQlReviewAuthor {
                        login: "reviewer-commented-approved".to_owned(),
                    }),
                },
            ],
        },
        reviews: GraphQlReviews {
            nodes: vec![
                GraphQlReviewNode {
                    state: "COMMENTED".to_owned(),
                    submitted_at: Some("2026-01-02T03:04:05Z".to_owned()),
                    author: Some(GraphQlReviewAuthor {
                        login: "reviewer-commented".to_owned(),
                    }),
                },
                GraphQlReviewNode {
                    state: "COMMENTED".to_owned(),
                    submitted_at: Some("2026-01-02T03:04:05Z".to_owned()),
                    author: Some(GraphQlReviewAuthor {
                        login: "reviewer-approved".to_owned(),
                    }),
                },
                GraphQlReviewNode {
                    state: "COMMENTED".to_owned(),
                    submitted_at: Some("2026-01-02T03:04:05Z".to_owned()),
                    author: Some(GraphQlReviewAuthor {
                        login: "reviewer-addressed".to_owned(),
                    }),
                },
                GraphQlReviewNode {
                    state: "COMMENTED".to_owned(),
                    submitted_at: Some("2026-01-02T03:04:05Z".to_owned()),
                    author: Some(GraphQlReviewAuthor {
                        login: "reviewer-obsolete".to_owned(),
                    }),
                },
                GraphQlReviewNode {
                    state: "COMMENTED".to_owned(),
                    submitted_at: Some("2026-01-02T03:04:05Z".to_owned()),
                    author: Some(GraphQlReviewAuthor {
                        login: "change-author".to_owned(),
                    }),
                },
            ],
        },
        review_threads: GraphQlReviewThreads {
            nodes: vec![
                GraphQlReviewThreadNode {
                    is_resolved: false,
                    is_outdated: false,
                    comments: GraphQlReviewThreadComments {
                        nodes: vec![GraphQlReviewThreadCommentNode {
                            author: Some(GraphQlReviewAuthor {
                                login: "reviewer-commented".to_owned(),
                            }),
                            created_at: "2026-01-01T12:00:00Z".to_owned(),
                        }],
                    },
                },
                GraphQlReviewThreadNode {
                    is_resolved: false,
                    is_outdated: false,
                    comments: GraphQlReviewThreadComments {
                        nodes: vec![
                            GraphQlReviewThreadCommentNode {
                                author: Some(GraphQlReviewAuthor {
                                    login: "reviewer-addressed".to_owned(),
                                }),
                                created_at: "2026-01-01T12:00:00Z".to_owned(),
                            },
                            GraphQlReviewThreadCommentNode {
                                author: Some(GraphQlReviewAuthor {
                                    login: "change-author".to_owned(),
                                }),
                                created_at: "2026-01-01T12:30:00Z".to_owned(),
                            },
                        ],
                    },
                },
                GraphQlReviewThreadNode {
                    is_resolved: true,
                    is_outdated: false,
                    comments: GraphQlReviewThreadComments {
                        nodes: vec![GraphQlReviewThreadCommentNode {
                            author: Some(GraphQlReviewAuthor {
                                login: "reviewer-obsolete".to_owned(),
                            }),
                            created_at: "2026-01-01T12:00:00Z".to_owned(),
                        }],
                    },
                },
            ],
        },
        timeline_items: GraphQlTimelineItems {
            nodes: vec![
                GraphQlTimelineItemNode::ConvertToDraft {
                    created_at: "2026-01-01T01:00:00Z".to_owned(),
                },
                GraphQlTimelineItemNode::ReadyForReview {
                    created_at: "2026-01-01T02:00:00Z".to_owned(),
                },
                GraphQlTimelineItemNode::ReviewRequested {
                    created_at: "2026-01-01T02:30:00Z".to_owned(),
                    requested_reviewer: Some(GraphQlRequestedReviewer {
                        type_name: "User".to_owned(),
                        login: Some("reviewer-one".to_owned()),
                        slug: None,
                    }),
                },
            ],
        },
        commits: GraphQlPullRequestStatusCommits {
            nodes: vec![GraphQlPullRequestStatusCommitNode {
                commit: GraphQlPullRequestStatusCommit {
                    oid: "aaaabbbbccccdddd".to_owned(),
                    status_check_rollup: Some(GraphQlStatusCheckRollup {
                        state: "FAILURE".to_owned(),
                        contexts: GraphQlStatusCheckContexts {
                            nodes: vec![
                                GraphQlStatusCheckContextNode {
                                    type_name: "CheckRun".to_owned(),
                                    name: Some("unit checks".to_owned()),
                                    context: None,
                                    status: Some("COMPLETED".to_owned()),
                                    conclusion: Some("FAILURE".to_owned()),
                                    state: None,
                                },
                                GraphQlStatusCheckContextNode {
                                    type_name: "StatusContext".to_owned(),
                                    name: None,
                                    context: Some("integration checks".to_owned()),
                                    status: None,
                                    conclusion: None,
                                    state: Some("PENDING".to_owned()),
                                },
                            ],
                        },
                    }),
                },
            }],
        },
    });

    assert_eq!(status.created_at.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(status.check_status, PullRequestCheckStatus::Failing);
    assert_eq!(
        status.checks,
        [
            PullRequestCheck {
                name: "unit checks".to_owned(),
                status: PullRequestCheckStatus::Failing,
            },
            PullRequestCheck {
                name: "integration checks".to_owned(),
                status: PullRequestCheckStatus::Pending,
            },
        ]
    );
    assert_eq!(
        status.review_status,
        PullRequestReviewStatus::ChangesRequested
    );
    assert_eq!(
        status.requested_reviewers.users,
        ["reviewer-one".to_owned()]
    );
    assert_eq!(
        status.suggested_reviewers,
        ["reviewer-suggested".to_owned()]
    );
    assert_eq!(status.approved_reviewers, ["reviewer-approved".to_owned()]);
    assert_eq!(
        status.commented_reviewers,
        ["reviewer-commented".to_owned()]
    );
    assert_eq!(
        status.addressed_reviewers,
        ["reviewer-addressed".to_owned()]
    );
    assert!(status.review_activity.iter().any(|activity| {
        activity.reviewer == "reviewer-approved" && activity.reviewed_at == "2026-01-02T03:04:05Z"
    }));
    assert_eq!(status.timeline_events.len(), 3);
    assert!(status.timeline_events.iter().any(|event| {
        event.kind == PullRequestTimelineEventKind::ReadyForReview
            && event.created_at == "2026-01-01T02:00:00Z"
    }));
    assert_eq!(
        status.latest_commit_oid.as_deref(),
        Some("aaaabbbbccccdddd")
    );
    assert_eq!(
        status.labels,
        [PullRequestLabel {
            name: "bug".to_owned(),
            color: "d73a4a".to_owned(),
        }]
    );

    let review_required = map_graphql_pull_request_status(GraphQlPullRequestStatus {
        number: 43,
        title: "Waiting change".to_owned(),
        url: "https://github.com/example-owner/example-repo/pull/43".to_owned(),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        head_ref_name: "topic/waiting".to_owned(),
        base_ref_name: "main".to_owned(),
        author: None,
        is_draft: false,
        merged: false,
        closed: false,
        merged_at: None,
        closed_at: None,
        review_decision: Some("REVIEW_REQUIRED".to_owned()),
        review_requests: GraphQlReviewRequests {
            total_count: 2,
            nodes: vec![GraphQlReviewRequestNode {
                requested_reviewer: Some(GraphQlRequestedReviewer {
                    type_name: "User".to_owned(),
                    login: Some("reviewer-two".to_owned()),
                    slug: None,
                }),
            }],
        },
        suggested_reviewers: Vec::new(),
        labels: GraphQlLabels { nodes: Vec::new() },
        latest_reviews: GraphQlReviews { nodes: Vec::new() },
        reviews: GraphQlReviews { nodes: Vec::new() },
        review_threads: GraphQlReviewThreads { nodes: Vec::new() },
        timeline_items: GraphQlTimelineItems { nodes: Vec::new() },
        commits: GraphQlPullRequestStatusCommits { nodes: Vec::new() },
    });

    assert_eq!(
        review_required.review_status,
        PullRequestReviewStatus::ReviewRequired
    );
    assert_eq!(
        review_required.requested_reviewers.users,
        ["reviewer-two".to_owned()]
    );
}

#[test]
fn review_request_search_queries_include_existing_review_activity() {
    // Verifies: the review inbox keeps open PRs visible after the viewer submits a review.
    assert_eq!(
        REVIEW_REQUEST_SEARCH_QUERIES,
        &[
            "is:pr is:open review-requested:@me -author:@me",
            "is:pr is:open reviewed-by:@me -author:@me",
        ]
    );
}

#[test]
fn pull_request_status_query_batches_numbers_with_aliases() {
    // Verifies: stack status can fetch several PRs in one GraphQL request.
    let query = pull_request_status_query(&[41, 42]);

    assert!(query.contains("pr0: pullRequest(number: 41)"));
    assert!(query.contains("pr1: pullRequest(number: 42)"));
    assert!(query.contains("statusCheckRollup"));
    assert!(query.contains("contexts(first: 100)"));
    assert!(query.contains("... on CheckRun"));
    assert!(query.contains("... on StatusContext"));
    assert!(query.contains("reviewDecision"));
    assert!(query.contains("mergedAt"));
    assert!(query.contains("closedAt"));
    assert!(query.contains("createdAt"));
    assert!(query.contains("  author {"));
    assert!(query.contains("reviewRequests"));
    assert!(query.contains("totalCount"));
    assert!(query.contains("requestedReviewer"));
    assert!(query.contains("suggestedReviewers"));
    assert!(query.contains("labels(first: 100)"));
    assert!(query.contains("      name"));
    assert!(query.contains("      color"));
    assert!(query.contains("latestReviews(first: 100)"));
    assert!(query.contains("reviews(first: 100)"));
    assert!(query.contains("submittedAt"));
    assert!(query.contains("reviewThreads(first: 100)"));
    assert!(query.contains("timelineItems(last: 100"));
    assert!(query.contains("READY_FOR_REVIEW_EVENT"));
    assert!(query.contains("CONVERT_TO_DRAFT_EVENT"));
    assert!(query.contains("REVIEW_REQUESTED_EVENT"));
    assert!(query.contains("... on ReadyForReviewEvent"));
    assert!(query.contains("... on ConvertToDraftEvent"));
    assert!(query.contains("... on ReviewRequestedEvent"));
    assert!(query.contains("requestedReviewer"));
    assert!(!query.contains("slug"));
    assert!(query.contains("isResolved"));
    assert!(query.contains("isOutdated"));
    assert!(query.contains("comments(first: 100)"));
    assert!(query.contains("createdAt"));
    assert!(query.contains("      state"));
    assert!(query.contains("      author"));
    assert!(query.contains("        login"));
}

#[test]
fn user_profiles_query_batches_logins_with_aliases() {
    // Verifies: display-name enrichment can resolve several GitHub users in one GraphQL request.
    let query = user_profiles_query(&["human-reviewer".to_owned(), "peer-reviewer".to_owned()]);

    assert!(query.contains("query($login0: String!, $login1: String!)"));
    assert!(query.contains("user0: user(login: $login0)"));
    assert!(query.contains("user1: user(login: $login1)"));
    assert!(query.contains("login"));
    assert!(query.contains("name"));
}

#[test]
fn pull_request_suggested_reviewers_query_uses_direct_list_field() {
    // Verifies: GitHub suggested reviewers are loaded from the direct GraphQL list shape.
    let query = PULL_REQUEST_SUGGESTED_REVIEWERS_QUERY;

    assert!(query.contains("suggestedReviewers"));
    assert!(query.contains("reviewer"));
    assert!(query.contains("login"));
    assert!(!query.contains("suggestedReviewers(first:"));
    assert!(!query.contains("nodes"));
}

#[test]
fn suggested_reviewers_mapper_preserves_github_order() {
    // Verifies: GitHub's ranked suggestion order is kept while blank/duplicate logins are removed.
    let reviewers = suggested_reviewers_from_graphql(vec![
        GraphQlSuggestedReviewer {
            reviewer: Some(GraphQlSuggestedReviewerUser {
                login: " second-reviewer ".to_owned(),
            }),
        },
        GraphQlSuggestedReviewer {
            reviewer: Some(GraphQlSuggestedReviewerUser {
                login: "first-reviewer".to_owned(),
            }),
        },
        GraphQlSuggestedReviewer {
            reviewer: Some(GraphQlSuggestedReviewerUser {
                login: "second-reviewer".to_owned(),
            }),
        },
        GraphQlSuggestedReviewer {
            reviewer: Some(GraphQlSuggestedReviewerUser {
                login: " ".to_owned(),
            }),
        },
        GraphQlSuggestedReviewer { reviewer: None },
    ]);

    assert_eq!(reviewers, ["second-reviewer", "first-reviewer"]);
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
