use super::*;
use crate::commands::review::{load_review_dashboard_snapshot, ReviewCleanupMode};

#[test]
fn approved_reviews_ignore_generic_replies_in_snapshots_and_history() {
    for source in [None, Some("manual"), Some("automatic")] {
        for from_history in [false, true] {
            for changed_head in [false, true] {
                let mut status = approved_status();
                let body = if changed_head {
                    "/merge"
                } else {
                    "Could you take another look?"
                };
                let history = if from_history {
                    vec![attention_event("author_response", "2026-01-01T12:45:00Z")]
                } else {
                    status.reviewer_responses.push(PullRequestReviewerResponse {
                        reviewer: "example-reviewer".to_owned(),
                        responded_at: "2026-01-01T12:45:00Z".to_owned(),
                        body_text: body.to_owned(),
                    });
                    Vec::new()
                };
                if changed_head {
                    status.latest_commit_oid = Some("new-head".to_owned());
                }
                let actions = source
                    .map(|source| vec![dismissal(source, "2026-01-01T12:30:00Z")])
                    .unwrap_or_default();
                assert_live_visibility(status, history, actions, false);
            }
        }
    }
}

#[test]
fn approved_mentions_must_be_newer_than_review_and_dismissal() {
    for source in [None, Some("manual"), Some("automatic")] {
        for mentioned_at in [
            "2026-01-01T11:45:00Z",
            "2026-01-01T12:00:00Z",
            "2026-01-01T12:15:00Z",
            "2026-01-01T12:30:00Z",
            "2026-01-01T12:45:00Z",
        ] {
            let mut status = approved_status();
            status.reviewer_mentions.push(PullRequestReviewerMention {
                reviewer: "example-reviewer".to_owned(),
                mentioned_at: mentioned_at.to_owned(),
            });
            // Head changes must not reactivate a mention already handled by dismissal.
            status.latest_commit_oid = Some("new-head".to_owned());
            let actions = source
                .map(|source| vec![dismissal(source, "2026-01-01T12:30:00Z")])
                .unwrap_or_default();
            let handled_at = if source.is_some() {
                "2026-01-01T12:30:00Z"
            } else {
                "2026-01-01T12:00:00Z"
            };
            assert_live_visibility(status, Vec::new(), actions, mentioned_at > handled_at);
        }
    }
}

#[test]
fn approved_reviews_resurface_for_fresh_requests_and_historical_mentions() {
    let mut requested = approved_status();
    requested.requested_reviewers =
        ReviewerSelection::new(["example-reviewer"], Vec::<String>::new());
    assert_live_visibility(requested, Vec::new(), Vec::new(), true);

    for (source, dismissed_at) in [
        (None, "2026-01-01T12:00:00Z"),
        (Some("manual"), "2026-01-01T11:30:00Z"),
        (Some("automatic"), "2026-01-01T11:30:00Z"),
        (Some("manual"), "2026-01-01T12:30:00Z"),
        (Some("automatic"), "2026-01-01T12:30:00Z"),
    ] {
        for kind in ["reviewer_requested", "reviewer_mentioned"] {
            for signal_at in [
                "2026-01-01T11:45:00Z",
                "2026-01-01T12:00:00Z",
                "2026-01-01T12:30:00Z",
                "2026-01-01T12:45:00Z",
            ] {
                let handled_at = dismissed_at.max("2026-01-01T12:00:00Z");
                let actions = source
                    .map(|source| vec![dismissal(source, dismissed_at)])
                    .unwrap_or_default();
                assert_live_visibility(
                    approved_status(),
                    vec![attention_event(kind, signal_at)],
                    actions,
                    signal_at > handled_at,
                );
            }
        }
    }
}

#[test]
fn approved_reviews_ignore_attention_for_other_reviewers() {
    let mut status = approved_status();
    status.requested_reviewers = ReviewerSelection::new(["someone-else"], Vec::<String>::new());
    status.reviewer_mentions.push(PullRequestReviewerMention {
        reviewer: "someone-else".to_owned(),
        mentioned_at: "2026-01-01T12:45:00Z".to_owned(),
    });
    let history = ["reviewer_requested", "reviewer_mentioned"]
        .into_iter()
        .map(|kind| PullRequestHistoryRecord {
            new_json: Some(serde_json::json!({ "reviewer": "someone-else" })),
            ..attention_event(kind, "2026-01-01T12:45:00Z")
        })
        .collect();
    assert_live_visibility(status, history, Vec::new(), false);
}

#[test]
fn approval_suppresses_replies_even_after_automatic_undismissal_or_on_drafts() {
    for draft in [false, true] {
        let mut status = approved_status();
        status.draft = draft;
        status.reviewer_responses.push(PullRequestReviewerResponse {
            reviewer: "example-reviewer".to_owned(),
            responded_at: "2026-01-01T12:45:00Z".to_owned(),
            body_text: "Ready to merge".to_owned(),
        });
        let mut action = dismissal("automatic", "2026-01-01T12:30:00Z");
        action.reason = Some("draft".to_owned());
        assert_live_visibility(status.clone(), Vec::new(), vec![action.clone()], false);
        let undismissal = PullRequestActionRecord {
            action: "undismiss".to_owned(),
            source: "automatic".to_owned(),
            reason: Some("author_response".to_owned()),
            changed_at_unix: timestamp("2026-01-01T12:50:00Z"),
            details_json: serde_json::json!({}),
        };
        assert_live_visibility(status, Vec::new(), vec![action, undismissal], false);
    }
}

#[test]
fn lost_approvals_and_manual_undismissals_still_resurface() {
    for has_dismissal in [false, true] {
        let mut status = approved_status();
        status.approved_reviewers.clear();
        status.dismissed_reviewers = vec!["example-reviewer".to_owned()];
        let actions = if has_dismissal {
            vec![dismissal("automatic", "2026-01-01T12:30:00Z")]
        } else {
            Vec::new()
        };
        assert_live_visibility(status, Vec::new(), actions, true);
    }
    assert_live_visibility(
        approved_status(),
        Vec::new(),
        vec![review_action(
            "undismiss",
            "manual",
            None,
            serde_json::json!({}),
        )],
        true,
    );
}

#[test]
fn unapproved_reviews_still_resurface_on_generic_author_replies() {
    for changes_requested in [false, true] {
        let mut status = approved_status();
        status.approved_reviewers.clear();
        if changes_requested {
            status.changes_requested_reviewers = vec!["example-reviewer".to_owned()];
        } else {
            status.commented_reviewers = vec!["example-reviewer".to_owned()];
        }
        status.reviewer_responses.push(PullRequestReviewerResponse {
            reviewer: "example-reviewer".to_owned(),
            responded_at: "2026-01-01T12:45:00Z".to_owned(),
            body_text: "Fixed it".to_owned(),
        });
        assert_live_visibility(status, Vec::new(), Vec::new(), true);
    }
}

#[test]
fn cached_approval_attention_survives_refresh_and_is_cleared_by_review_or_dismissal() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repository = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "api-alpha".to_owned(),
    };
    let store = PullRequestStore::open(&environment).unwrap();
    store
        .record_review_inbox_snapshot(&PullRequestReviewRequests {
            viewer: AuthenticatedUser {
                login: "example-reviewer".to_owned(),
            },
            requests: vec![review_request("example-owner", "api-alpha", 12)],
        })
        .unwrap();
    let services = FakeServices::default();
    let mut status = approved_status();
    status.reviewer_responses.push(PullRequestReviewerResponse {
        reviewer: "example-reviewer".to_owned(),
        responded_at: "2026-01-01T12:45:00Z".to_owned(),
        body_text: "/merge".to_owned(),
    });
    store
        .record_pull_request_snapshots(&repository, &[status.clone()])
        .unwrap();
    assert_dashboard_visibility(&environment, &services, true, false);

    status.reviewer_mentions.push(PullRequestReviewerMention {
        reviewer: "example-reviewer".to_owned(),
        mentioned_at: "2026-01-01T13:00:00Z".to_owned(),
    });
    store
        .record_pull_request_snapshots(&repository, &[status.clone()])
        .unwrap();
    assert_dashboard_visibility(&environment, &services, true, true);
    run_with_args_and_services(
        ["jx", "review", "--cached", "dismiss", "api-alpha#12"],
        &environment,
        &services,
    )
    .unwrap();
    assert_dashboard_visibility(&environment, &services, true, false);

    status.latest_commit_oid = Some("new-head".to_owned());
    store
        .record_pull_request_snapshots(&repository, &[status.clone()])
        .unwrap();
    assert_dashboard_visibility(&environment, &services, true, false);
    let next_mention = chrono::Utc::now() + chrono::Duration::minutes(1);
    status.reviewer_mentions[0].mentioned_at = next_mention.to_rfc3339();
    store
        .record_pull_request_snapshots(&repository, &[status.clone()])
        .unwrap();
    assert_dashboard_visibility(&environment, &services, true, true);
    // A later snapshot may drop the comment; history must retain the attention signal.
    status.reviewer_mentions.clear();
    store
        .record_pull_request_snapshots(&repository, &[status.clone()])
        .unwrap();
    assert_dashboard_visibility(&environment, &services, true, true);
    status.review_activity[0].reviewed_at =
        (next_mention + chrono::Duration::minutes(1)).to_rfc3339();
    store
        .record_pull_request_snapshots(&repository, &[status])
        .unwrap();
    assert_dashboard_visibility(&environment, &services, true, false);

    assert_eq!(services.review_request_calls.get(), 0);
    assert!(services.pull_request_status_calls.borrow().is_empty());
}

fn approved_status() -> PullRequestStatusRecord {
    let mut status = review_status_record(12, "Approved attention", "author", false);
    status.requested_reviewers = ReviewerSelection::default();
    status.approved_reviewers = vec!["example-reviewer".to_owned()];
    status.review_activity = vec![PullRequestReviewActivity {
        reviewer: "example-reviewer".to_owned(),
        reviewed_at: "2026-01-01T12:00:00Z".to_owned(),
    }];
    status
}

fn attention_event(kind: &str, at: &str) -> PullRequestHistoryRecord {
    PullRequestHistoryRecord {
        kind: kind.to_owned(),
        changed_at_unix: timestamp(at),
        old_json: None,
        new_json: Some(serde_json::json!({ "reviewer": "example-reviewer" })),
        details_json: serde_json::json!({}),
    }
}

fn dismissal(source: &str, at: &str) -> PullRequestActionRecord {
    PullRequestActionRecord {
        changed_at_unix: timestamp(at),
        ..review_action(
            "dismiss",
            source,
            Some(if source == "automatic" {
                "approved"
            } else {
                "manual"
            }),
            serde_json::json!({ "dismissedHeadOid": "commit-12" }),
        )
    }
}

fn timestamp(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp()
}

fn assert_live_visibility(
    status: PullRequestStatusRecord,
    history: Vec<PullRequestHistoryRecord>,
    actions: Vec<PullRequestActionRecord>,
    visible: bool,
) {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_requests_with_history: BTreeMap::from([(
            12,
            PullRequestWithHistory {
                status,
                history,
                actions,
            },
        )]),
        ..FakeServices::default()
    };
    assert_dashboard_visibility(&environment, &services, false, visible);
    let dismissed =
        run_with_args_and_services(["jx", "review", "dismissed"], &environment, &services).unwrap();
    assert_eq!(dismissed.stdout.contains("Approved attention"), !visible);
}

fn assert_dashboard_visibility(
    environment: &RuntimeEnvironment,
    services: &FakeServices,
    cached: bool,
    visible: bool,
) {
    for cleanup in [ReviewCleanupMode::ReadOnly, ReviewCleanupMode::Record] {
        let snapshot = load_review_dashboard_snapshot(
            ReviewRequest {
                action: ReviewAction::Show,
                repo_filters: Vec::new(),
                interactive: true,
                refresh_seconds: 300,
                format: ReviewFormat::Human,
                cached,
            },
            environment,
            services,
            cleanup,
        )
        .unwrap();
        let frame = snapshot
            .render(DashboardRenderOptions {
                color: false,
                terminal_width: Some(120),
            })
            .unwrap();
        assert_eq!(
            frame.rows.len(),
            usize::from(visible),
            "cached={cached}, cleanup={cleanup:?}, frame={}",
            frame.text
        );
    }
}
