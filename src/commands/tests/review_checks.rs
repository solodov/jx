use super::*;
use crate::commands::review::load_review_dashboard_snapshot;

#[test]
fn live_cached_and_dashboard_inboxes_wait_only_for_required_checks_in_matching_repositories() {
    let workspace = review_workspace();
    configure_pending_checks(&workspace);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut statuses = BTreeMap::new();
    for (number, checks) in [
        (
            11,
            vec![check("tests", PullRequestCheckStatus::Pending, true)],
        ),
        (
            12,
            vec![check("tests", PullRequestCheckStatus::Passing, true)],
        ),
        (
            13,
            vec![check("tests", PullRequestCheckStatus::Failing, true)],
        ),
        (
            14,
            vec![
                check("tests", PullRequestCheckStatus::Pending, true),
                check("lint", PullRequestCheckStatus::Failing, true),
            ],
        ),
        (
            15,
            vec![check("tests", PullRequestCheckStatus::Unknown, true)],
        ),
        (16, vec![]),
        (
            17,
            vec![
                check("tests", PullRequestCheckStatus::Passing, true),
                check("optional", PullRequestCheckStatus::Pending, false),
            ],
        ),
        (
            18,
            vec![
                check("tests", PullRequestCheckStatus::Passing, true),
                check("ignored", PullRequestCheckStatus::Pending, true),
            ],
        ),
        (
            19,
            vec![
                check("tests", PullRequestCheckStatus::Passing, true),
                check("review-gate", PullRequestCheckStatus::Pending, true),
            ],
        ),
        (
            20,
            vec![
                check("tests", PullRequestCheckStatus::Passing, true),
                check("Settings", PullRequestCheckStatus::Pending, true),
            ],
        ),
        (
            21,
            vec![check("tests", PullRequestCheckStatus::Pending, true)],
        ),
    ] {
        let mut status = review_status_record(number, &format!("CI PR {number}"), "author", false);
        status.checks = checks;
        statuses.insert(number, status);
    }
    let services = FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: statuses
            .keys()
            .map(|number| {
                review_request(
                    if *number == 21 { "other" } else { "Faire" },
                    "backend",
                    *number,
                )
            })
            .collect(),
        pull_request_statuses: statuses,
        ..FakeServices::default()
    };
    seed_checks_inbox(&environment, &services);
    let expected = BTreeSet::from([12, 13, 15, 16, 17, 18, 19, 20, 21]);
    let live = run_with_args_and_services(
        ["jx", "review", "--format", "json"],
        &environment,
        &services,
    )
    .unwrap();
    assert_eq!(visible_numbers(&live.stdout), expected);
    let offline = FakeServices::default();
    let cached = run_with_args_and_services(
        ["jx", "review", "--cached", "--format", "json"],
        &environment,
        &offline,
    )
    .unwrap();
    assert_eq!(visible_numbers(&cached.stdout), expected);

    let snapshot = load_review_dashboard_snapshot(
        ReviewRequest {
            action: ReviewAction::Show,
            repo_filters: Vec::new(),
            interactive: true,
            refresh_seconds: 300,
            format: ReviewFormat::Human,
            cached: true,
        },
        &environment,
        &offline,
    )
    .unwrap();
    for width in [120, 60] {
        let frame = snapshot
            .render(DashboardRenderOptions {
                color: false,
                terminal_width: Some(width),
            })
            .unwrap();
        assert_eq!(
            frame
                .rows
                .iter()
                .map(|row| row.context.pr_number)
                .collect::<BTreeSet<_>>(),
            expected
        );
    }
    assert_eq!(offline.review_request_calls.get(), 0);
    assert_eq!(offline.github_user_display_name_calls.get(), 0);
    assert!(offline.pull_request_status_calls.borrow().is_empty());
}

#[test]
fn pending_prs_keep_refreshing_and_return_when_checks_finish_without_becoming_dismissed() {
    let workspace = review_workspace();
    configure_pending_checks(&workspace);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut services = single_pr_services();
    for (state, visible) in [
        (PullRequestCheckStatus::Pending, false),
        (PullRequestCheckStatus::Passing, true),
        (PullRequestCheckStatus::Pending, false),
        (PullRequestCheckStatus::Failing, true),
    ] {
        services.pull_request_statuses.get_mut(&12).unwrap().checks =
            vec![check("tests", state, true)];
        let result = run_with_args_and_services(
            ["jx", "review", "--format", "json"],
            &environment,
            &services,
        )
        .unwrap();
        assert_eq!(visible_numbers(&result.stdout).contains(&12), visible);
        assert!(PullRequestStore::open(&environment)
            .unwrap()
            .action_dismissed_pull_requests()
            .unwrap()
            .is_empty());
    }
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![12], vec![12], vec![12], vec![12]]
    );
}

#[test]
fn temporary_ci_filter_does_not_prevent_manual_dismissal_or_clear_it_when_checks_finish() {
    let workspace = review_workspace();
    configure_pending_checks(&workspace);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut services = single_pr_services();
    seed_checks_inbox(&environment, &services);
    let offline = FakeServices::default();
    run_with_args_and_services(
        ["jx", "review", "--cached", "dismiss", "Faire/backend#12"],
        &environment,
        &offline,
    )
    .unwrap();

    // Management views bypass the temporary filter, so the stored dismissal remains discoverable.
    let store = PullRequestStore::open(&environment).unwrap();
    let repository = GitHubRepository {
        owner: "Faire".to_owned(),
        name: "backend".to_owned(),
    };
    let pending = store
        .latest_pull_requests_with_history(&repository, &[12])
        .unwrap()
        .pop()
        .unwrap();
    services.pull_requests_with_history.insert(12, pending);
    let dismissed = run_with_args_and_services(
        ["jx", "review", "--format", "json", "dismissed"],
        &environment,
        &services,
    )
    .unwrap();
    assert_eq!(visible_numbers(&dismissed.stdout), BTreeSet::from([12]));
    services.pull_request_statuses.get_mut(&12).unwrap().checks =
        vec![check("tests", PullRequestCheckStatus::Passing, true)];
    seed_checks_inbox(&environment, &services);
    let result = run_with_args_and_services(
        ["jx", "review", "--cached", "--format", "json"],
        &environment,
        &offline,
    )
    .unwrap();
    assert!(visible_numbers(&result.stdout).is_empty());
    assert_eq!(
        PullRequestStore::open(&environment)
            .unwrap()
            .action_dismissed_pull_requests()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn pending_checks_are_visible_by_default_and_after_an_explicit_repo_opt_out() {
    let workspace = review_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = single_pr_services();
    let result = run_with_args_and_services(
        ["jx", "review", "--format", "json"],
        &environment,
        &services,
    )
    .unwrap();
    assert_eq!(visible_numbers(&result.stdout), BTreeSet::from([12]));
    configure_pending_checks(&workspace);
    workspace.write_home_file(
        ".config/jx/99-review.toml",
        "[[repo.rules]]\nrepo='Faire/backend'\n[repo.rules.review]\nhide_pending_checks=false\n",
    );
    let result = run_with_args_and_services(
        ["jx", "review", "--format", "json"],
        &environment,
        &services,
    )
    .unwrap();
    assert_eq!(visible_numbers(&result.stdout), BTreeSet::from([12]));
}

fn configure_pending_checks(workspace: &TestWorkspace) {
    workspace.write_home_file(
        ".config/jx/10-review.toml",
        r#"
[[repo.rules]]
repo = "Faire/*"
[repo.rules.review]
hide_pending_checks = true
[repo.rules.stack_status]
ignored_checks = ["^ignored$"]
review_gate_checks = ["^review-gate$"]
auto_merge_prerequisite_checks = ["^Settings$"]
"#,
    );
}

fn single_pr_services() -> FakeServices {
    let mut status = review_status_record(12, "Waiting for CI", "author", false);
    status.checks = vec![check("tests", PullRequestCheckStatus::Pending, true)];
    FakeServices {
        github_login: "example-reviewer".to_owned(),
        review_requests: vec![review_request("Faire", "backend", 12)],
        pull_request_statuses: BTreeMap::from([(12, status)]),
        ..FakeServices::default()
    }
}

fn seed_checks_inbox(environment: &RuntimeEnvironment, services: &FakeServices) {
    let store = PullRequestStore::open(environment).unwrap();
    store
        .record_review_inbox_snapshot(&PullRequestReviewRequests {
            viewer: AuthenticatedUser {
                login: services.github_login.clone(),
            },
            requests: services.review_requests.clone(),
        })
        .unwrap();
    for request in &services.review_requests {
        store
            .record_pull_request_snapshots(
                &request.repository,
                &[services.pull_request_statuses[&request.number].clone()],
            )
            .unwrap();
    }
}

fn visible_numbers(output: &str) -> BTreeSet<u64> {
    let value: serde_json::Value = serde_json::from_str(output).unwrap();
    value["pullRequests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pr| pr["number"].as_u64().unwrap())
        .collect()
}

fn check(name: &str, status: PullRequestCheckStatus, required: bool) -> PullRequestCheck {
    PullRequestCheck {
        name: name.to_owned(),
        status,
        required,
    }
}
