use super::*;

#[test]
fn assigned_only_pull_requests_are_visible_and_deduplicated() {
    for draft in [false, true] {
        let workspace = review_workspace();
        let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
        let mut status = assigned_status(draft);
        status.assignees = vec!["Example-Reviewer".to_owned()];
        let candidate = review_request("example-owner", "api-alpha", 12);
        let services = FakeServices {
            github_login: "example-reviewer".to_owned(),
            review_requests: vec![candidate.clone(), candidate],
            pull_request_statuses: BTreeMap::from([(12, status)]),
            ..FakeServices::default()
        };

        let result = run_with_args_and_services(
            ["jx", "review", "--format", "json"],
            &environment,
            &services,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        let rows = value["pullRequests"].as_array().unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["number"], 12);
        assert_eq!(rows[0]["draft"], draft);
        assert_eq!(rows[0]["requestState"], "new");
        assert!(rows[0].get("requestedUsers").is_none());
        assert_eq!(*services.pull_request_status_calls.borrow(), vec![vec![12]]);
    }
}

#[test]
fn assignment_does_not_reopen_completed_reviews() {
    for draft in [false, true] {
        for state in ["approved", "commented", "changes_requested"] {
            let workspace = review_workspace();
            let environment =
                RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
            let mut status = assigned_status(draft);
            let reviewers = vec!["example-reviewer".to_owned()];
            match state {
                "approved" => status.approved_reviewers = reviewers,
                "commented" => status.commented_reviewers = reviewers,
                "changes_requested" => status.changes_requested_reviewers = reviewers,
                _ => unreachable!(),
            }
            let services = FakeServices {
                github_login: "example-reviewer".to_owned(),
                review_requests: vec![review_request("example-owner", "api-alpha", 12)],
                pull_request_statuses: BTreeMap::from([(12, status)]),
                ..FakeServices::default()
            };

            let result =
                run_with_args_and_services(["jx", "review"], &environment, &services).unwrap();

            assert!(
                !result.stdout.contains("Assigned work"),
                "{draft:?} {state}"
            );
            assert_no_review_dismissal_state_or_log(&workspace);
        }
    }
}

#[test]
fn drafts_assigned_to_someone_else_stay_hidden() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = assigned_status(true);
    status.assignees = vec!["someone-else".to_owned()];
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services).unwrap();

    assert!(!result.stdout.contains("Assigned work"));
}

#[test]
fn assigned_drafts_still_allow_explicit_attention_after_a_comment() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = assigned_status(true);
    status.commented_reviewers = vec!["example-reviewer".to_owned()];
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

    let result = run_with_args_and_services(["jx", "review"], &environment, &services).unwrap();

    assert!(result.stdout.contains("Assigned work"));
}

#[test]
fn self_assigned_authored_pull_requests_stay_out_of_the_review_inbox() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = assigned_status(false);
    status.author = Some("example-reviewer".to_owned());
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("example-owner", "api-alpha", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "review"], &environment, &services).unwrap();

    assert!(!result.stdout.contains("Assigned work"));
}

#[test]
fn assigned_pull_requests_respect_cached_dismissals_until_undismissed() {
    for draft in [false, true] {
        let workspace = review_workspace();
        let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
        let status = assigned_status(draft);
        let (store, repository) = seed_assigned_inbox(&environment, &status);
        let services = FakeServices::default();
        let args = ["jx", "review", "--cached"];

        let before = run_with_args_and_services(args, &environment, &services).unwrap();
        assert!(before.stdout.contains("Assigned work"));
        run_with_args_and_services(
            ["jx", "review", "--cached", "dismiss", "api-alpha#12"],
            &environment,
            &services,
        )
        .unwrap();

        store
            .record_pull_request_snapshots(&repository, &[status])
            .unwrap();
        for _ in 0..2 {
            let dismissed = run_with_args_and_services(args, &environment, &services).unwrap();
            assert!(!dismissed.stdout.contains("Assigned work"));
        }

        run_with_args_and_services(
            ["jx", "review", "undismiss", "api-alpha#12"],
            &environment,
            &services,
        )
        .unwrap();
        let restored = run_with_args_and_services(args, &environment, &services).unwrap();
        assert!(restored.stdout.contains("Assigned work"));
        assert_no_network(&services);
    }
}

#[test]
fn cached_drafts_follow_assignment_updates() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut status = assigned_status(true);
    let (store, repository) = seed_assigned_inbox(&environment, &status);
    let services = FakeServices::default();
    let args = ["jx", "review", "--cached"];

    let assigned = run_with_args_and_services(args, &environment, &services).unwrap();
    assert!(assigned.stdout.contains("Assigned work"));

    status.assignees.clear();
    store
        .record_pull_request_snapshots(&repository, &[status.clone()])
        .unwrap();
    let unassigned = run_with_args_and_services(args, &environment, &services).unwrap();
    assert!(!unassigned.stdout.contains("Assigned work"));

    status.assignees = vec!["example-reviewer".to_owned()];
    store
        .record_pull_request_snapshots(&repository, &[status])
        .unwrap();
    let reassigned = run_with_args_and_services(args, &environment, &services).unwrap();
    assert!(reassigned.stdout.contains("Assigned work"));
    assert_no_network(&services);
}

fn assigned_status(draft: bool) -> PullRequestStatusRecord {
    let mut status = review_status_record(12, "Assigned work", "example-author", draft);
    status.requested_reviewers = ReviewerSelection::default();
    status.review_status = PullRequestReviewStatus::NotReviewed;
    status.assignees = vec!["example-reviewer".to_owned()];
    status
}

fn seed_assigned_inbox(
    environment: &RuntimeEnvironment,
    status: &PullRequestStatusRecord,
) -> (PullRequestStore, GitHubRepository) {
    let store = PullRequestStore::open(environment).unwrap();
    let repository = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "api-alpha".to_owned(),
    };
    store
        .record_review_inbox_snapshot(&PullRequestReviewRequests {
            viewer: AuthenticatedUser {
                login: "example-reviewer".to_owned(),
            },
            requests: vec![review_request("example-owner", "api-alpha", 12)],
        })
        .unwrap();
    store
        .record_pull_request_snapshots(&repository, std::slice::from_ref(status))
        .unwrap();
    (store, repository)
}

fn assert_no_network(services: &FakeServices) {
    assert_eq!(services.review_request_calls.get(), 0);
    assert!(services.pull_request_status_calls.borrow().is_empty());
    assert_eq!(services.github_user_display_name_calls.get(), 0);
}
