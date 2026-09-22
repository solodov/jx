use super::*;
use crate::github::{
    PullRequestAutoMergeStatus, PullRequestCheck, PullRequestCheckStatus, PullRequestReviewStatus,
};
use crate::repository::{
    AutoMergeLabelConfig, IgnoredCheckConfig, IgnoredReviewerConfig, RepoReviewConfig,
    RepoStackStatusConfig, ReviewGateCheckConfig,
};

#[test]
fn approval_requires_a_non_ignored_reviewer_with_or_without_review_gates() {
    for with_gates in [false, true] {
        for check_source in ["none", "passing", "ignored"] {
            for (approvers, eligible) in [
                (vec![], false),
                (vec!["automation[bot]"], false),
                (vec!["reviewer"], true),
                (vec!["automation[bot]", "reviewer"], true),
            ] {
                let mut config = approval_config(with_gates);
                if check_source == "ignored" {
                    config.ignored_checks.push(IgnoredCheckConfig {
                        name: ".*".to_owned(),
                    });
                }
                let mut status = pull_request_status(31, "Approval policy", false);
                status.approved_reviewers =
                    approvers.iter().map(|login| (*login).to_owned()).collect();
                status.requested_reviewers =
                    ReviewerSelection::new(["waiting-reviewer"], Vec::<String>::new());
                if check_source != "none" {
                    status.checks = passing_checks();
                }
                let expected = if eligible && (!with_gates || check_source == "passing") {
                    PullRequestReviewStatus::Approved
                } else {
                    PullRequestReviewStatus::ReviewRequested
                };
                let filtered = apply_pull_request_status_policy(status.clone(), &config);
                assert_eq!(
                    filtered.review_status, expected,
                    "gates={with_gates}, checks={check_source}, approvers={approvers:?}"
                );
                assert_eq!(
                    filtered.approved_reviewers,
                    if eligible { vec!["reviewer"] } else { vec![] }
                );
                if !eligible {
                    assert!(!pull_request_status_is_stack_green(&filtered));
                }
                let inbox = apply_review_request_status_policy(
                    status,
                    &config,
                    &RepoReviewConfig::default(),
                );
                assert_eq!(inbox.review_status, filtered.review_status);
                assert_eq!(inbox.approved_reviewers, filtered.approved_reviewers);
            }
        }
    }
}

#[test]
fn approval_requirement_also_applies_without_any_repository_policy() {
    let mut status = pull_request_status(31, "No review evidence", false);
    let config = RepoStackStatusConfig::default();
    let filtered = apply_pull_request_status_policy(status.clone(), &config);
    assert_eq!(
        filtered.review_status,
        PullRequestReviewStatus::ReviewRequested
    );
    assert!(!pull_request_status_is_stack_green(&filtered));

    status.approved_reviewers = vec!["reviewer".to_owned()];
    let filtered = apply_pull_request_status_policy(status, &config);
    assert_eq!(filtered.review_status, PullRequestReviewStatus::Approved);
    assert!(pull_request_status_is_stack_green(&filtered));
}

#[test]
fn gates_cannot_supply_an_approval_or_override_blocking_review_decisions() {
    for with_gates in [false, true] {
        for decision in [
            PullRequestReviewStatus::Approved,
            PullRequestReviewStatus::ReviewRequired,
            PullRequestReviewStatus::ChangesRequested,
            PullRequestReviewStatus::ReviewRequested,
            PullRequestReviewStatus::NotReviewed,
            PullRequestReviewStatus::Unknown,
        ] {
            for eligible in [false, true] {
                let mut status = pull_request_status(31, "Review decision", false);
                status.review_status = decision;
                status.approved_reviewers = vec![if eligible {
                    "reviewer"
                } else {
                    "automation[bot]"
                }
                .to_owned()];
                status.checks = passing_checks();
                let filtered =
                    apply_pull_request_status_policy(status, &approval_config(with_gates));
                let expected = match decision {
                    PullRequestReviewStatus::ChangesRequested
                    | PullRequestReviewStatus::ReviewRequired => decision,
                    _ if with_gates => {
                        if eligible {
                            PullRequestReviewStatus::Approved
                        } else {
                            PullRequestReviewStatus::ReviewRequested
                        }
                    }
                    PullRequestReviewStatus::Approved if !eligible => {
                        PullRequestReviewStatus::ReviewRequested
                    }
                    _ => decision,
                };
                assert_eq!(
                    filtered.review_status, expected,
                    "gates={with_gates}, decision={decision:?}, eligible={eligible}"
                );
            }
        }
    }
}

#[test]
fn eligible_approval_still_requires_all_configured_gates_to_pass() {
    for check_status in [
        None,
        Some(PullRequestCheckStatus::Failing),
        Some(PullRequestCheckStatus::Pending),
        Some(PullRequestCheckStatus::Unknown),
        Some(PullRequestCheckStatus::Passing),
    ] {
        let mut config = approval_config(true);
        config.review_gate_checks.push(ReviewGateCheckConfig {
            name: "^second gate$".to_owned(),
        });
        let mut status = pull_request_status(31, "Gate status", false);
        status.approved_reviewers = vec!["reviewer".to_owned()];
        status.checks = passing_checks();
        if let Some(check_status) = check_status {
            status.checks.push(PullRequestCheck {
                name: "second gate".to_owned(),
                status: check_status,
                required: true,
            });
        }
        let filtered = apply_pull_request_status_policy(status, &config);
        assert_eq!(
            filtered.review_status,
            if check_status == Some(PullRequestCheckStatus::Passing) {
                PullRequestReviewStatus::Approved
            } else {
                PullRequestReviewStatus::ReviewRequested
            }
        );
        assert_eq!(filtered.check_status, PullRequestCheckStatus::Passing);
    }
}

#[test]
fn ignored_approval_does_not_mark_a_pr_ready_for_auto_merge() {
    let mut config = approval_config(true);
    config.auto_merge_labels.push(AutoMergeLabelConfig {
        label: "auto-merge".to_owned(),
        when: Vec::new(),
    });
    let mut status = pull_request_status(31, "Bot approval", false);
    status.approved_reviewers = vec!["automation[bot]".to_owned()];
    status.checks = passing_checks();
    let filtered = apply_pull_request_status_policy(status.clone(), &config);
    assert!(!pull_request_status_is_stack_green(&filtered));
    assert_eq!(
        filtered.auto_merge_status,
        PullRequestAutoMergeStatus::NotConfigured
    );

    status.approved_reviewers.push("reviewer".to_owned());
    let filtered = apply_pull_request_status_policy(status, &config);
    assert!(pull_request_status_is_stack_green(&filtered));
    assert_eq!(
        filtered.auto_merge_status,
        PullRequestAutoMergeStatus::Missing
    );
}

fn approval_config(with_gates: bool) -> RepoStackStatusConfig {
    RepoStackStatusConfig {
        ignored_reviewers: vec![IgnoredReviewerConfig {
            name: "^automation".to_owned(),
        }],
        review_gate_checks: if with_gates {
            vec![ReviewGateCheckConfig {
                name: "^approval gate$".to_owned(),
            }]
        } else {
            Vec::new()
        },
        ..Default::default()
    }
}

fn passing_checks() -> Vec<PullRequestCheck> {
    ["approval gate", "ci/build"]
        .into_iter()
        .map(|name| PullRequestCheck {
            name: name.to_owned(),
            status: PullRequestCheckStatus::Passing,
            required: true,
        })
        .collect()
}
