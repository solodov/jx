use super::*;
use crate::commands::review::load_review_dashboard_snapshot;

#[test]
fn cached_dismiss_parses_but_other_review_subcommands_still_reject_cached() {
    let matches = cli()
        .try_get_matches_from(["jx", "review", "--cached", "dismiss", "api-alpha#12"])
        .unwrap();
    let CommandRequest::Review(request) = CommandRequest::from_matches(&matches).unwrap() else {
        panic!("expected review request");
    };
    assert!(request.cached);
    assert!(matches!(
        request.action,
        ReviewAction::Dismiss {
            until: ReviewDismissUntil::Attention,
            ..
        }
    ));
    for args in [
        vec!["dismissed"],
        vec!["undismiss", "12"],
        vec!["history", "12"],
    ] {
        let matches = cli()
            .try_get_matches_from(["jx", "review", "--cached"].into_iter().chain(args))
            .unwrap();
        assert!(CommandRequest::from_matches(&matches).is_err());
    }
}

#[test]
fn cached_dismiss_and_dashboard_reload_remove_rows_and_empty_groups_without_network() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    seed_inbox(&environment);
    let services = FakeServices::default();
    let request = ReviewRequest {
        action: ReviewAction::Show,
        repo_filters: Vec::new(),
        interactive: true,
        refresh_seconds: 300,
        format: ReviewFormat::Human,
        cached: true,
    };
    let options = DashboardRenderOptions {
        color: false,
        terminal_width: Some(120),
    };
    let before = load_review_dashboard_snapshot(request.clone(), &environment, &services).unwrap();
    assert_eq!(before.render(options).unwrap().rows.len(), 3);

    run_with_args_and_services(
        ["jx", "review", "--cached", "dismiss", "api-beta#21"],
        &environment,
        &services,
    )
    .unwrap();
    run_with_args_and_services(
        ["jx", "review", "--cached", "dismiss", "api-alpha#12"],
        &environment,
        &services,
    )
    .unwrap();

    let after = load_review_dashboard_snapshot(request, &environment, &services).unwrap();
    for width in [120, 60] {
        let frame = after
            .render(DashboardRenderOptions {
                terminal_width: Some(width),
                ..options
            })
            .unwrap();
        assert_eq!(frame.rows.len(), 1);
        assert_eq!(frame.rows[0].context.pr_number, 13);
        assert!(!frame.text.contains("api-beta"));
        assert!(!frame.text.contains("Title 12"));
        assert!(!frame.text.contains("Title 21"));
    }
    assert_no_network(&services);
}

#[test]
fn cached_dismiss_reports_missing_inbox_or_snapshot_without_fetching() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();
    let args = ["jx", "review", "--cached", "dismiss", "api-alpha#12"];
    let error = run_with_args_and_services(args, &environment, &services).unwrap_err();
    assert!(error.to_string().contains("no cached review inbox"));

    let store = PullRequestStore::open(&environment).unwrap();
    store
        .record_review_inbox_snapshot(&PullRequestReviewRequests {
            viewer: AuthenticatedUser {
                login: "example-reviewer".to_owned(),
            },
            requests: vec![review_request("example-owner", "api-alpha", 12)],
        })
        .unwrap();
    let error = run_with_args_and_services(args, &environment, &services).unwrap_err();
    assert!(error.to_string().contains("no review pull request matched"));
    assert!(store.action_dismissed_pull_requests().unwrap().is_empty());
    assert_no_network(&services);
}

#[test]
fn failed_cached_dismiss_does_not_hide_the_pr() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    seed_inbox(&environment);
    let store = PullRequestStore::open(&environment).unwrap();
    let repository = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "api-alpha".to_owned(),
    };
    let mut status = review_status_record(12, "Missing head", "author", false);
    status.latest_commit_oid = None;
    store
        .record_pull_request_snapshots(&repository, &[status])
        .unwrap();
    let services = FakeServices::default();
    let error = run_with_args_and_services(
        ["jx", "review", "--cached", "dismiss", "api-alpha#12"],
        &environment,
        &services,
    )
    .unwrap_err();
    assert!(error.to_string().contains("latest commit oid"));
    let output =
        run_with_args_and_services(["jx", "review", "--cached"], &environment, &services).unwrap();
    assert!(output.stdout.contains("Missing head"));
    assert!(store.action_dismissed_pull_requests().unwrap().is_empty());
    assert_no_network(&services);
}

fn seed_inbox(environment: &RuntimeEnvironment) {
    let store = PullRequestStore::open(environment).unwrap();
    store
        .record_review_inbox_snapshot(&PullRequestReviewRequests {
            viewer: AuthenticatedUser {
                login: "example-reviewer".to_owned(),
            },
            requests: vec![
                review_request("example-owner", "api-alpha", 12),
                review_request("example-owner", "api-alpha", 13),
                review_request("example-owner", "api-beta", 21),
            ],
        })
        .unwrap();
    for (name, numbers) in [("api-alpha", vec![12, 13]), ("api-beta", vec![21])] {
        let repository = GitHubRepository {
            owner: "example-owner".to_owned(),
            name: name.to_owned(),
        };
        let statuses = numbers
            .into_iter()
            .map(|number| review_status_record(number, &format!("Title {number}"), "author", false))
            .collect::<Vec<_>>();
        store
            .record_pull_request_snapshots(&repository, &statuses)
            .unwrap();
    }
}

fn assert_no_network(services: &FakeServices) {
    assert_eq!(services.review_request_calls.get(), 0);
    assert!(services.pull_request_status_calls.borrow().is_empty());
    assert_eq!(services.github_user_display_name_calls.get(), 0);
}
