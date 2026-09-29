use super::*;
use crate::commands::review::{load_review_dashboard_snapshot, ReviewCleanupMode};

#[test]
fn author_responses_must_be_newer_than_dismissal_and_its_response_watermark() {
    for from_history in [false, true] {
        for (responded_at, watermark, visible) in [
            ("2026-01-01T12:45:00Z", Some("2026-01-01T12:30:00Z"), false),
            ("2026-01-01T13:10:00Z", Some("2026-01-01T12:30:00Z"), false),
            ("2026-01-01T13:15:00Z", Some("2026-01-01T12:30:00Z"), true),
            ("2026-01-01T13:15:00Z", None, true),
            ("2026-01-01T13:15:00Z", Some("2026-01-01T13:30:00Z"), false),
        ] {
            let workspace = review_workspace();
            let environment =
                RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
            let mut status = review_status_record(12, "Dismissed response", "author", false);
            status.review_activity = vec![PullRequestReviewActivity {
                reviewer: "example-reviewer".to_owned(),
                reviewed_at: "2026-01-01T12:00:00Z".to_owned(),
            }];
            let history = if from_history {
                vec![PullRequestHistoryRecord {
                    kind: "author_response".to_owned(),
                    changed_at_unix: chrono::DateTime::parse_from_rfc3339(responded_at)
                        .unwrap()
                        .timestamp(),
                    old_json: None,
                    new_json: Some(serde_json::json!({
                        "reviewer": "example-reviewer",
                        "respondedAt": responded_at,
                        "bodyText": "Fixed it",
                    })),
                    details_json: serde_json::json!({}),
                }]
            } else {
                status.reviewer_responses = vec![response(responded_at, "Fixed it")];
                Vec::new()
            };
            let mut action = review_dismiss_action(
                "manual",
                serde_json::json!({
                    "dismissedHeadOid": "commit-12",
                    "dismissedViewerResponseAt": watermark,
                }),
            );
            action.changed_at_unix = chrono::DateTime::parse_from_rfc3339("2026-01-01T13:10:00Z")
                .unwrap()
                .timestamp();
            let services = FakeServices {
                github_login: "example-reviewer".to_owned(),
                review_requests: vec![review_request("example-owner", "api-alpha", 12)],
                pull_requests_with_history: BTreeMap::from([(
                    12,
                    PullRequestWithHistory {
                        status,
                        history,
                        actions: vec![action],
                    },
                )]),
                ..FakeServices::default()
            };
            let output =
                run_with_args_and_services(["jx", "review"], &environment, &services).unwrap();
            assert_eq!(
                output.stdout.contains("Dismissed response"),
                visible,
                "history={from_history}, response={responded_at}, watermark={watermark:?}"
            );
            if !visible {
                assert_no_review_dismissal_state_or_log(&workspace);
            }
        }
    }
}

#[test]
fn ignored_response_history_cannot_undo_cached_dismissal_or_post_fetch_cleanup() {
    for draft in [false, true] {
        for has_watermark in [false, true] {
            let workspace = review_workspace();
            workspace.write_file(".jx/config.toml", "[repo.review]\nignored_author_response_comments = ['^/trunk\\s+(merge|cancel)\\s*$']\n");
            let environment =
                RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
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
            let mut status = review_status_record(12, "Dismissed response", "author", draft);
            status.review_activity = vec![PullRequestReviewActivity {
                reviewer: "example-reviewer".to_owned(),
                reviewed_at: "2026-01-01T12:00:00Z".to_owned(),
            }];
            if has_watermark {
                status
                    .reviewer_responses
                    .push(response("2026-01-01T12:03:00Z", "/poke"));
            }
            status
                .reviewer_responses
                .push(response("2026-01-01T12:06:00Z", "/trunk merge"));
            store
                .record_pull_request_snapshots(&repository, &[status.clone()])
                .unwrap();
            let services = FakeServices::default();
            run_with_args_and_services(
                ["jx", "review", "--cached", "dismiss", "api-alpha#12"],
                &environment,
                &services,
            )
            .unwrap();
            assert_cached_visibility(&environment, &services, false);

            let after_dismissal = chrono::Utc::now() + chrono::Duration::minutes(1);
            status
                .reviewer_responses
                .push(response(&after_dismissal.to_rfc3339(), "/trunk cancel"));
            store
                .record_pull_request_snapshots(&repository, &[status.clone()])
                .unwrap();
            // Keep the ignored command only in history, not in the latest snapshot.
            status
                .reviewer_responses
                .retain(|response| !response.body_text.starts_with("/trunk"));
            store
                .record_pull_request_snapshots(&repository, &[status.clone()])
                .unwrap();
            assert_cached_visibility(&environment, &services, false);
            let stored = store
                .latest_pull_requests_with_history(&repository, &[12])
                .unwrap()
                .remove(0);
            assert_eq!(stored.actions.last().unwrap().action, "dismiss");
            assert!(
                stored.history.iter().any(|event| event
                    .new_json
                    .as_ref()
                    .and_then(|value| value.get("bodyText"))
                    .and_then(serde_json::Value::as_str)
                    == Some("/trunk cancel")),
                "ignore rules must not erase audit history"
            );

            let reply_at = after_dismissal + chrono::Duration::minutes(1);
            status
                .reviewer_responses
                .push(response(&reply_at.to_rfc3339(), "Fixed the tests"));
            store
                .record_pull_request_snapshots(&repository, &[status.clone()])
                .unwrap();
            status.reviewer_responses.pop();
            store
                .record_pull_request_snapshots(&repository, &[status])
                .unwrap();
            assert_cached_visibility(&environment, &services, true);
            let stored = store
                .latest_pull_requests_with_history(&repository, &[12])
                .unwrap()
                .remove(0);
            let action = stored.actions.last().unwrap();
            assert_eq!(action.action, "undismiss");
            assert_eq!(action.reason.as_deref(), Some("author_response"));
            assert_eq!(services.review_request_calls.get(), 0);
            assert!(services.pull_request_status_calls.borrow().is_empty());
            assert_eq!(services.github_user_display_name_calls.get(), 0);
        }
    }
}

fn response(responded_at: &str, body: &str) -> PullRequestReviewerResponse {
    PullRequestReviewerResponse {
        reviewer: "example-reviewer".to_owned(),
        responded_at: responded_at.to_owned(),
        body_text: body.to_owned(),
    }
}

fn assert_cached_visibility(
    environment: &RuntimeEnvironment,
    services: &FakeServices,
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
                cached: true,
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
            "cleanup={cleanup:?}, frame={}",
            frame.text
        );
    }
}
