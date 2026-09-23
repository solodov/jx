use super::*;
use crate::repository::{
    AutoMergePrerequisiteCheckConfig, IgnoredCheckConfig, ReviewGateCheckConfig,
};

#[test]
fn only_pending_required_checks_block_review_even_with_an_existing_failure() {
    let mut status = pull_request_status(12, "Waiting for CI", false);
    status.check_status = PullRequestCheckStatus::Failing;
    for check_status in [
        PullRequestCheckStatus::Passing,
        PullRequestCheckStatus::Failing,
        PullRequestCheckStatus::Pending,
        PullRequestCheckStatus::Missing,
        PullRequestCheckStatus::Unknown,
    ] {
        for required in [false, true] {
            status.checks = vec![
                check("already failed", PullRequestCheckStatus::Failing, true),
                check("tests", check_status, required),
            ];
            assert_eq!(
                review_request_has_pending_checks(&status),
                required && check_status == PullRequestCheckStatus::Pending,
            );
        }
    }
    status.checks.clear();
    assert!(!review_request_has_pending_checks(&status));
}

#[test]
fn normalized_review_checks_exclude_ignored_gates_prerequisites_and_superseded_runs() {
    let mut status = pull_request_status(12, "CI complete", false);
    status.checks = vec![
        check("ignored", PullRequestCheckStatus::Pending, true),
        check("review-gate", PullRequestCheckStatus::Pending, true),
        check("Settings", PullRequestCheckStatus::Pending, true),
        check("tests", PullRequestCheckStatus::Pending, true),
        check("tests", PullRequestCheckStatus::Passing, true),
        check("optional", PullRequestCheckStatus::Pending, false),
    ];
    let status = apply_review_request_status_policy(
        status,
        &RepoStackStatusConfig {
            ignored_checks: vec![IgnoredCheckConfig {
                name: "^ignored$".to_owned(),
            }],
            review_gate_checks: vec![ReviewGateCheckConfig {
                name: "^review-gate$".to_owned(),
            }],
            auto_merge_prerequisite_checks: vec![AutoMergePrerequisiteCheckConfig {
                name: "^Settings$".to_owned(),
            }],
            ..Default::default()
        },
        &RepoReviewConfig::default(),
    );
    assert_eq!(status.check_status, PullRequestCheckStatus::Passing);
    assert!(!review_request_has_pending_checks(&status));
}

fn check(name: &str, status: PullRequestCheckStatus, required: bool) -> PullRequestCheck {
    PullRequestCheck {
        name: name.to_owned(),
        status,
        required,
    }
}
