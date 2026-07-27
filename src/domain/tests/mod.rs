use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use crate::{
    github::{
        AuthenticatedUser, PullRequestCreate, PullRequestHead, PullRequestRecord,
        PullRequestUpdate, RepositoryFork, ReviewerSelection, ReviewerSyncResult,
    },
    jj::{
        ChangeSummary, PushedBookmarkSummary, StatusRemoteFacts, StatusWorkspaceFacts,
        TrackedPushOutcome, TrunkSummary, WorkspaceVisibility,
    },
    repository::{
        GitHubRemote, GitHubRepository, OriginRemote, PullRequestEventPredicate,
        PullRequestEventQuery, PullRequestEventQueryTerm, RepoConfig, RepoEvent, RepoEventHandler,
        RepoEventHandlerConfig, RepoEventHandlerRun, RepoPolicyConfig, ReviewerPathRule,
        StackMetadata, StackMetadataNode, TokenSource, WorkflowConfig, ORIGIN_REMOTE_NAME,
    },
};

use super::*;

#[test]
fn pull_request_stack_snapshot_layers_live_prs_over_metadata() {
    // Verifies: Snapshot nodes preserve durable stack edges while refreshing PR fields from GitHub.
    let metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![
            StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Stored root".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                draft: false,
                merged: true,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                parent_branch: Some("topic/root".to_owned()),
                pull_request: Some(11),
                parent_pull_request: Some(10),
                title: "Stored child".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
        ],
    };
    let live_child = PullRequestRecord {
        number: 11,
        title: "Live child".to_owned(),
        body: None,
        head_branch: "topic/child".to_owned(),
        base_branch: "topic/root".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/11".to_owned()),
        draft: true,
        merged: false,
        reviewers: ReviewerSelection::default(),
    };

    let snapshot = PullRequestStackSnapshot::from_metadata(
        &metadata,
        &["topic/child".to_owned()],
        &[live_child],
        PullRequestStackSelection::branch("topic/child"),
    );

    assert_eq!(snapshot.current_branch.as_deref(), Some("topic/child"));
    assert_eq!(snapshot.current_pull_request, Some(11));
    assert_eq!(snapshot.nodes.len(), 2);
    assert_eq!(snapshot.nodes[0].title, "Stored root");
    assert!(snapshot.nodes[0].merged);
    assert!(!snapshot.nodes[0].is_local);
    assert_eq!(snapshot.nodes[1].title, "Live child");
    assert_eq!(
        snapshot.nodes[1].parent_branch.as_deref(),
        Some("topic/root")
    );
    assert_eq!(snapshot.nodes[1].parent_pull_request, Some(10));
    assert!(snapshot.nodes[1].draft);
    assert!(snapshot.nodes[1].is_local);
    assert!(snapshot.nodes[1].is_current);
}

#[test]
fn pull_request_stack_snapshot_adds_live_prs_missing_from_metadata() {
    // Verifies: Live PR records can form stack nodes before durable metadata has been written.
    let metadata = StackMetadata::default();
    let root = PullRequestRecord {
        number: 10,
        title: "Root".to_owned(),
        body: None,
        head_branch: "topic/root".to_owned(),
        base_branch: "main".to_owned(),
        html_url: None,
        draft: false,
        merged: false,
        reviewers: ReviewerSelection::default(),
    };
    let child = PullRequestRecord {
        number: 11,
        title: "Child".to_owned(),
        body: None,
        head_branch: "topic/child".to_owned(),
        base_branch: "topic/root".to_owned(),
        html_url: None,
        draft: false,
        merged: false,
        reviewers: ReviewerSelection::default(),
    };

    let snapshot = PullRequestStackSnapshot::from_metadata(
        &metadata,
        &["topic/root".to_owned(), "topic/child".to_owned()],
        &[child, root],
        PullRequestStackSelection::pull_request(10),
    );

    assert_eq!(snapshot.current_branch.as_deref(), Some("topic/root"));
    assert_eq!(snapshot.nodes[0].branch, "topic/root");
    assert_eq!(snapshot.nodes[1].branch, "topic/child");
    assert_eq!(
        snapshot.nodes[1].parent_branch.as_deref(),
        Some("topic/root")
    );
    assert_eq!(snapshot.nodes[1].parent_pull_request, Some(10));
    assert!(snapshot.nodes.iter().all(|node| node.is_local));
}

#[test]
fn pull_request_stack_snapshot_refreshes_stored_node_by_pull_request_number() {
    // Verifies: Durable nodes can be refreshed from PR numbers even when branch lookup is unavailable.
    let metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![StackMetadataNode {
            branch: "topic/root".to_owned(),
            base_branch: "main".to_owned(),
            parent_branch: None,
            pull_request: Some(10),
            parent_pull_request: None,
            title: "Stored root".to_owned(),
            url: None,
            draft: false,
            merged: false,
            work_ids: Vec::new(),
            fixes_work_ids: Vec::new(),
        }],
    };
    let live_root = PullRequestRecord {
        number: 10,
        title: "Live root".to_owned(),
        body: None,
        head_branch: "deleted/root".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
        draft: true,
        merged: true,
        reviewers: ReviewerSelection::default(),
    };

    let snapshot = PullRequestStackSnapshot::from_metadata(
        &metadata,
        &[],
        std::slice::from_ref(&live_root),
        PullRequestStackSelection::pull_request(10),
    );
    let refreshed = refresh_stack_metadata_pull_requests(&[live_root], &metadata);

    assert_eq!(snapshot.nodes.len(), 1);
    assert_eq!(snapshot.nodes[0].branch, "topic/root");
    assert_eq!(snapshot.nodes[0].title, "Live root");
    assert_eq!(snapshot.nodes[0].pull_request_number(), Some(10));
    assert!(snapshot.nodes[0].draft);
    assert!(snapshot.nodes[0].merged);
    assert!(snapshot.nodes[0].is_current);
    assert_eq!(refreshed.nodes[0].branch, "topic/root");
    assert_eq!(refreshed.nodes[0].title, "Live root");
    assert_eq!(
        refreshed.nodes[0].url,
        snapshot.nodes[0].pull_request.as_ref().unwrap().url
    );
    assert!(refreshed.nodes[0].draft);
    assert!(refreshed.nodes[0].merged);
}

#[test]
fn local_stack_refresh_preserves_completed_parent_context() {
    // Verifies: a submitted parent disappearing from local jj branches does not erase open-child stack context.
    let metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![
            StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Merged root".to_owned(),
                url: None,
                draft: false,
                merged: true,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                parent_branch: Some("topic/root".to_owned()),
                pull_request: Some(11),
                parent_pull_request: Some(10),
                title: "Open child".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
        ],
    };
    let local_child = crate::jj::LocalStackBranch {
        branch: "topic/child".to_owned(),
        base_branch: "main".to_owned(),
        parent_branch: None,
        title: "Open child local title".to_owned(),
        commit_id: "1111222233334444".to_owned(),
    };

    let refreshed = apply_local_stack_branches(&[local_child], &metadata);
    let child = refreshed
        .nodes
        .iter()
        .find(|node| node.branch == "topic/child")
        .expect("child remains tracked");

    assert_eq!(child.base_branch, "main");
    assert_eq!(child.parent_branch.as_deref(), Some("topic/root"));
    assert_eq!(child.parent_pull_request, Some(10));
}

#[test]
fn pull_request_record_refresh_preserves_fix_merge_transition() {
    // Verifies: sync/refresh cannot consume the status-observed transition that runs work-item handlers.
    let mut metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![StackMetadataNode {
            branch: "topic/root".to_owned(),
            base_branch: "main".to_owned(),
            parent_branch: None,
            pull_request: Some(10),
            parent_pull_request: None,
            title: "Stored root".to_owned(),
            url: None,
            draft: false,
            merged: false,
            work_ids: vec!["ABC-123".to_owned()],
            fixes_work_ids: vec!["ABC-123".to_owned()],
        }],
    };
    let live_root = PullRequestRecord {
        number: 10,
        title: "Live root".to_owned(),
        body: None,
        head_branch: "topic/root".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
        draft: false,
        merged: true,
        reviewers: ReviewerSelection::default(),
    };

    let refreshed =
        refresh_stack_metadata_pull_requests(std::slice::from_ref(&live_root), &metadata);
    let upserted = upsert_stack_metadata_pull_requests(&[live_root], &metadata);

    assert!(!refreshed.nodes[0].merged);
    assert!(!upserted.nodes[0].merged);

    metadata.nodes[0].fixes_work_ids.clear();
    let refreshed_without_fix_intent = refresh_stack_metadata_pull_requests(
        &[PullRequestRecord {
            number: 10,
            title: "Live root".to_owned(),
            body: None,
            head_branch: "topic/root".to_owned(),
            base_branch: "main".to_owned(),
            html_url: None,
            draft: false,
            merged: true,
            reviewers: ReviewerSelection::default(),
        }],
        &metadata,
    );
    assert!(refreshed_without_fix_intent.nodes[0].merged);
}

#[test]
fn pull_request_stack_prunes_only_fully_merged_components() {
    // Verifies: completed stack trees are removed while merged ancestors remain for open descendants.
    let metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![
            StackMetadataNode {
                branch: "merged/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Merged root".to_owned(),
                url: None,
                draft: false,
                merged: true,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "merged/child".to_owned(),
                base_branch: "merged/root".to_owned(),
                parent_branch: Some("merged/root".to_owned()),
                pull_request: Some(11),
                parent_pull_request: Some(10),
                title: "Merged child".to_owned(),
                url: None,
                draft: false,
                merged: true,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "mixed/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(20),
                parent_pull_request: None,
                title: "Mixed root".to_owned(),
                url: None,
                draft: false,
                merged: true,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "mixed/child".to_owned(),
                base_branch: "mixed/root".to_owned(),
                parent_branch: Some("mixed/root".to_owned()),
                pull_request: Some(21),
                parent_pull_request: Some(20),
                title: "Mixed child".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
        ],
    };

    let pruned = prune_merged_stack_metadata_trees(&metadata);

    assert_eq!(
        pruned
            .nodes
            .iter()
            .map(|node| node.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["mixed/root", "mixed/child"]
    );
}

#[test]
fn pull_request_stack_status_maintenance_retains_recently_closed_nodes() {
    // Verifies: freshly closed PRs remain as reminder rows instead of disappearing immediately.
    let metadata = closed_status_maintenance_metadata();
    let now = utc_datetime("2026-06-05T12:00:00Z");
    let mut closed = pull_request_status(10, "Closed root", false);
    closed.closed = true;
    closed.closed_at = Some("2026-06-04T18:00:00Z".to_owned());

    let maintained = maintain_stack_metadata_pull_request_statuses_at(
        &[closed, pull_request_status(11, "Open child", false)],
        &metadata,
        now,
    );

    assert_eq!(maintained.nodes.len(), 2);
    assert_eq!(maintained.nodes[0].branch, "closed/root");
    assert_eq!(maintained.nodes[1].branch, "open/child");
    assert_eq!(
        maintained.nodes[1].parent_branch.as_deref(),
        Some("closed/root")
    );
    assert_eq!(maintained.nodes[1].parent_pull_request, Some(10));
}

#[test]
fn pull_request_stack_status_maintenance_prunes_expired_closed_nodes() {
    // Verifies: closed reminder rows expire later while still-open descendants remain visible.
    let metadata = closed_status_maintenance_metadata();
    let now = utc_datetime("2026-06-05T12:00:00Z");
    let mut closed = pull_request_status(10, "Closed root", false);
    closed.closed = true;
    closed.closed_at = Some("2026-06-01T12:00:00Z".to_owned());

    let maintained = maintain_stack_metadata_pull_request_statuses_at(
        &[closed, pull_request_status(11, "Open child", false)],
        &metadata,
        now,
    );

    assert_eq!(maintained.nodes.len(), 1);
    assert_eq!(maintained.nodes[0].branch, "open/child");
    assert_eq!(maintained.nodes[0].parent_branch, None);
    assert_eq!(maintained.nodes[0].parent_pull_request, None);
}

fn closed_status_maintenance_metadata() -> StackMetadata {
    StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![
            StackMetadataNode {
                branch: "closed/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Closed root".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "open/child".to_owned(),
                base_branch: "closed/root".to_owned(),
                parent_branch: Some("closed/root".to_owned()),
                pull_request: Some(11),
                parent_pull_request: Some(10),
                title: "Open child".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
        ],
    }
}

#[test]
fn pull_request_status_policy_filters_ignored_labels_and_reviewers() {
    // Verifies: repo-scoped presentation policy removes noisy labels and reviewer tokens.
    let mut status = pull_request_status(31, "Review facts", false);
    status.labels = vec![
        crate::github::PullRequestLabel {
            name: "useful-signal".to_owned(),
            color: "0e8a16".to_owned(),
        },
        crate::github::PullRequestLabel {
            name: "generated-noise".to_owned(),
            color: "5319e7".to_owned(),
        },
    ];
    status.requested_reviewers = ReviewerSelection::new(
        ["human-reviewer", "ignored-bot"],
        ["platform", "ignored-team"],
    );
    status.suggested_reviewers = vec!["suggested-reviewer".to_owned(), "ignored-bot".to_owned()];
    status.approved_reviewers = vec!["human-reviewer".to_owned(), "ignored-bot".to_owned()];
    status.commented_reviewers = vec!["commenter".to_owned(), "ignored-bot".to_owned()];
    status.addressed_reviewers = vec!["addressed".to_owned(), "ignored-bot".to_owned()];
    status.reviewer_responses = vec![
        crate::github::PullRequestReviewerResponse {
            reviewer: "addressed".to_owned(),
            responded_at: "2026-01-01T00:00:00Z".to_owned(),
            body_text: "fixed".to_owned(),
        },
        crate::github::PullRequestReviewerResponse {
            reviewer: "ignored-bot".to_owned(),
            responded_at: "2026-01-01T00:00:00Z".to_owned(),
            body_text: "fixed".to_owned(),
        },
    ];
    status.dismissed_reviewers = vec!["dismissed".to_owned(), "ignored-bot".to_owned()];
    status.check_status = crate::github::PullRequestCheckStatus::Failing;
    status.checks = vec![
        crate::github::PullRequestCheck {
            name: "ci/build".to_owned(),
            status: crate::github::PullRequestCheckStatus::Passing,
        },
        crate::github::PullRequestCheck {
            name: "generated-check".to_owned(),
            status: crate::github::PullRequestCheckStatus::Failing,
        },
    ];
    status.review_activity = vec![
        crate::github::PullRequestReviewActivity {
            reviewer: "commenter".to_owned(),
            reviewed_at: "2026-01-01T00:00:00Z".to_owned(),
        },
        crate::github::PullRequestReviewActivity {
            reviewer: "ignored-bot".to_owned(),
            reviewed_at: "2026-01-01T00:00:00Z".to_owned(),
        },
    ];

    let filtered = apply_pull_request_status_policy(
        status,
        &crate::repository::RepoStackStatusConfig {
            review_gate_checks: Vec::new(),
            auto_merge_prerequisite_checks: Vec::new(),
            ignored_checks: vec![crate::repository::IgnoredCheckConfig {
                name: "^generated-.*".to_owned(),
            }],
            ignored_labels: vec![crate::repository::IgnoredLabelConfig {
                name: "generated-noise".to_owned(),
            }],
            ignored_labels_when_merged: Vec::new(),
            hidden_labels: Vec::new(),
            auto_merge_labels: Vec::new(),
            ignored_reviewers: vec![
                crate::repository::IgnoredReviewerConfig {
                    name: "^ignored-.*$".to_owned(),
                },
                crate::repository::IgnoredReviewerConfig {
                    name: "^team/ignored-team$".to_owned(),
                },
            ],
            title_rewrites: Vec::new(),
            review_wait_threshold_seconds: None,
        },
    );

    assert_eq!(
        filtered
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["useful-signal"]
    );
    assert_eq!(filtered.requested_reviewers.users, ["human-reviewer"]);
    assert_eq!(filtered.requested_reviewers.teams, ["platform"]);
    assert_eq!(filtered.suggested_reviewers, ["suggested-reviewer"]);
    assert_eq!(
        filtered
            .checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<Vec<_>>(),
        ["ci/build"]
    );
    assert_eq!(
        filtered.check_status,
        crate::github::PullRequestCheckStatus::Passing
    );
    assert_eq!(filtered.approved_reviewers, ["human-reviewer"]);
    assert_eq!(filtered.commented_reviewers, ["commenter"]);
    assert_eq!(filtered.addressed_reviewers, ["addressed"]);
    assert_eq!(filtered.reviewer_responses.len(), 1);
    assert_eq!(filtered.reviewer_responses[0].reviewer, "addressed");
    assert_eq!(filtered.dismissed_reviewers, ["dismissed"]);
    assert_eq!(filtered.review_activity.len(), 1);
    assert_eq!(filtered.review_activity[0].reviewer, "commenter");
}

#[test]
fn review_request_status_policy_filters_review_only_labels() {
    // Verifies: jx review can hide extra labels without mutating stack status policy.
    let mut status = pull_request_status(31, "Review facts", false);
    status.labels = vec![
        crate::github::PullRequestLabel {
            name: "useful-signal".to_owned(),
            color: "0e8a16".to_owned(),
        },
        crate::github::PullRequestLabel {
            name: "stack-noise".to_owned(),
            color: "5319e7".to_owned(),
        },
        crate::github::PullRequestLabel {
            name: "review-noise".to_owned(),
            color: "5319e7".to_owned(),
        },
    ];
    status.reviewer_responses = vec![
        crate::github::PullRequestReviewerResponse {
            reviewer: "example-reviewer".to_owned(),
            responded_at: "2026-01-01T00:00:00Z".to_owned(),
            body_text: "/automation merge".to_owned(),
        },
        crate::github::PullRequestReviewerResponse {
            reviewer: "example-reviewer".to_owned(),
            responded_at: "2026-01-01T01:00:00Z".to_owned(),
            body_text: "Ready for another look".to_owned(),
        },
    ];

    let filtered = apply_review_request_status_policy(
        status,
        &crate::repository::RepoStackStatusConfig {
            ignored_labels: vec![crate::repository::IgnoredLabelConfig {
                name: "stack-noise".to_owned(),
            }],
            ..Default::default()
        },
        &crate::repository::RepoReviewConfig {
            ignored_labels: vec![crate::repository::IgnoredLabelConfig {
                name: "review-noise".to_owned(),
            }],
            hidden_labels: Vec::new(),
            ignored_author_response_comments: vec![
                crate::repository::IgnoredAuthorResponseCommentConfig {
                    pattern: "^/automation merge$".to_owned(),
                },
            ],
        },
    );

    assert_eq!(
        filtered
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["useful-signal"]
    );
    assert_eq!(filtered.reviewer_responses.len(), 1);
    assert_eq!(
        filtered.reviewer_responses[0].body_text,
        "Ready for another look"
    );
}

#[test]
fn pull_request_status_policy_filters_snapshot_conditioned_labels() {
    // Verifies: hidden-label rules use current PR snapshot facts rather than hard-coded labels.
    let mut ready_default = pull_request_status(32, "Ready default", false);
    ready_default.labels = vec![
        crate::github::PullRequestLabel {
            name: "run-ci".to_owned(),
            color: "0e8a16".to_owned(),
        },
        crate::github::PullRequestLabel {
            name: "kept".to_owned(),
            color: "5319e7".to_owned(),
        },
    ];
    let mut draft_default = ready_default.clone();
    draft_default.draft = true;
    let mut ready_stack = ready_default.clone();
    ready_stack.base_branch = "topic/root".to_owned();
    let config = crate::repository::RepoStackStatusConfig {
        hidden_labels: vec![crate::repository::HiddenLabelConfig {
            label: "run-ci".to_owned(),
            when: vec![
                crate::repository::HiddenLabelCondition::NotDraft,
                crate::repository::HiddenLabelCondition::TargetsDefaultBranch,
            ],
        }],
        ..Default::default()
    };

    let ready_default = apply_pull_request_status_policy(ready_default, &config);
    let draft_default = apply_pull_request_status_policy(draft_default, &config);
    let ready_stack = apply_pull_request_status_policy(ready_stack, &config);

    assert_eq!(
        ready_default
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["kept"]
    );
    assert_eq!(
        draft_default
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["run-ci", "kept"]
    );
    assert_eq!(
        ready_stack
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["run-ci", "kept"]
    );
}

#[test]
fn pull_request_status_policy_uses_review_gate_checks_as_effective_approval() {
    // Verifies: repo-defined gate checks decide approval unless GitHub still
    // requires protected review.
    let config = crate::repository::RepoStackStatusConfig {
        review_gate_checks: vec![
            crate::repository::ReviewGateCheckConfig {
                name: "approval gate".to_owned(),
            },
            crate::repository::ReviewGateCheckConfig {
                name: "committer gate".to_owned(),
            },
        ],
        ..Default::default()
    };
    let mut gate_approved = pull_request_status(32, "Gate approved", false);
    gate_approved.review_status = crate::github::PullRequestReviewStatus::NotReviewed;
    gate_approved.checks = vec![
        crate::github::PullRequestCheck {
            name: "approval gate".to_owned(),
            status: crate::github::PullRequestCheckStatus::Passing,
        },
        crate::github::PullRequestCheck {
            name: "committer gate".to_owned(),
            status: crate::github::PullRequestCheckStatus::Passing,
        },
        crate::github::PullRequestCheck {
            name: "ci/build".to_owned(),
            status: crate::github::PullRequestCheckStatus::Passing,
        },
    ];
    let mut github_required_review = pull_request_status(34, "Requires review", false);
    github_required_review.review_status = crate::github::PullRequestReviewStatus::ReviewRequired;
    github_required_review.checks = gate_approved.checks.clone();
    let mut stale_github_approval = pull_request_status(33, "Stale approval", false);
    stale_github_approval.review_status = crate::github::PullRequestReviewStatus::Approved;
    stale_github_approval.checks = vec![
        crate::github::PullRequestCheck {
            name: "approval gate".to_owned(),
            status: crate::github::PullRequestCheckStatus::Passing,
        },
        crate::github::PullRequestCheck {
            name: "committer gate".to_owned(),
            status: crate::github::PullRequestCheckStatus::Failing,
        },
        crate::github::PullRequestCheck {
            name: "ci/build".to_owned(),
            status: crate::github::PullRequestCheckStatus::Passing,
        },
    ];

    let gate_approved = apply_pull_request_status_policy(gate_approved, &config);
    let github_required_review = apply_pull_request_status_policy(github_required_review, &config);
    let stale_github_approval = apply_pull_request_status_policy(stale_github_approval, &config);

    assert_eq!(
        gate_approved.review_status,
        crate::github::PullRequestReviewStatus::Approved
    );
    assert_eq!(
        gate_approved.check_status,
        crate::github::PullRequestCheckStatus::Passing
    );
    assert_eq!(
        gate_approved
            .checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<Vec<_>>(),
        ["ci/build"]
    );
    assert_eq!(
        github_required_review.review_status,
        crate::github::PullRequestReviewStatus::ReviewRequired
    );
    assert_eq!(
        stale_github_approval.review_status,
        crate::github::PullRequestReviewStatus::ReviewRequested
    );
    assert_eq!(
        stale_github_approval.check_status,
        crate::github::PullRequestCheckStatus::Passing
    );
}

#[test]
fn pull_request_status_policy_filters_merged_only_labels_after_merge() {
    // Verifies: labels that only matter before merge stay visible on open PRs but disappear from merged rows.
    let mut open_status = pull_request_status(32, "Open labels", false);
    open_status.labels = vec![
        crate::github::PullRequestLabel {
            name: "run-ci".to_owned(),
            color: "0e8a16".to_owned(),
        },
        crate::github::PullRequestLabel {
            name: "kept".to_owned(),
            color: "5319e7".to_owned(),
        },
    ];
    let mut merged_status = pull_request_status(33, "Merged labels", true);
    merged_status.labels = open_status.labels.clone();
    let config = crate::repository::RepoStackStatusConfig {
        review_gate_checks: Vec::new(),
        auto_merge_prerequisite_checks: Vec::new(),
        ignored_checks: Vec::new(),
        ignored_labels: Vec::new(),
        ignored_labels_when_merged: vec![crate::repository::IgnoredLabelConfig {
            name: "run-ci".to_owned(),
        }],
        hidden_labels: Vec::new(),
        auto_merge_labels: Vec::new(),
        ignored_reviewers: Vec::new(),
        title_rewrites: Vec::new(),
        review_wait_threshold_seconds: None,
    };

    let open_filtered = apply_pull_request_status_policy(open_status, &config);
    let merged_filtered = apply_pull_request_status_policy(merged_status, &config);

    assert_eq!(
        open_filtered
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["run-ci", "kept"]
    );
    assert_eq!(
        merged_filtered
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["kept"]
    );
}

#[test]
fn pull_request_status_policy_reports_configured_auto_merge_state() {
    // Verifies: configured workflow labels become semantic auto-merge state instead of label chips.
    let config = crate::repository::RepoStackStatusConfig {
        auto_merge_labels: vec![crate::repository::AutoMergeLabelConfig {
            label: "auto-merge".to_owned(),
            when: vec![crate::repository::HiddenLabelCondition::TargetsDefaultBranch],
        }],
        ..Default::default()
    };
    let mut armed = pull_request_status(34, "Armed", false);
    armed.labels = vec![
        crate::github::PullRequestLabel {
            name: "auto-merge".to_owned(),
            color: "fbca04".to_owned(),
        },
        crate::github::PullRequestLabel {
            name: "kept".to_owned(),
            color: "5319e7".to_owned(),
        },
    ];
    let ready_missing = pull_request_status(35, "Ready missing", false);
    let mut pending_missing = pull_request_status(36, "Pending missing", false);
    pending_missing.check_status = crate::github::PullRequestCheckStatus::Pending;
    let mut review_required_missing = pull_request_status(37, "Review pending missing", false);
    review_required_missing.review_status = crate::github::PullRequestReviewStatus::ReviewRequired;

    let armed = apply_pull_request_status_policy(armed, &config);
    let ready_missing = apply_pull_request_status_policy(ready_missing, &config);
    let pending_missing = apply_pull_request_status_policy(pending_missing, &config);
    let review_required_missing =
        apply_pull_request_status_policy(review_required_missing, &config);

    assert_eq!(
        armed.auto_merge_status,
        crate::github::PullRequestAutoMergeStatus::Armed
    );
    assert_eq!(
        armed
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["kept"]
    );
    assert_eq!(
        ready_missing.auto_merge_status,
        crate::github::PullRequestAutoMergeStatus::Missing
    );
    assert_eq!(
        pending_missing.auto_merge_status,
        crate::github::PullRequestAutoMergeStatus::NotConfigured
    );
    assert_eq!(
        review_required_missing.auto_merge_status,
        crate::github::PullRequestAutoMergeStatus::NotConfigured
    );
}

#[test]
fn stack_green_status_requires_known_mergeable_state() {
    // Verifies: sync protection waits for GitHub to confirm mergeability before preserving a green stack.
    let mergeable = pull_request_status(39, "Mergeable", false);
    let mut conflicting = pull_request_status(40, "Conflicting", false);
    conflicting.merge_status = crate::github::PullRequestMergeStatus::Conflicting;
    let mut unknown = pull_request_status(41, "Unknown", false);
    unknown.merge_status = crate::github::PullRequestMergeStatus::Unknown;

    assert!(pull_request_status_has_green_stack_checks(&mergeable));
    assert!(pull_request_status_is_stack_green(&mergeable));
    assert!(!pull_request_status_has_green_stack_checks(&conflicting));
    assert!(!pull_request_status_is_stack_green(&conflicting));
    assert!(!pull_request_status_has_green_stack_checks(&unknown));
    assert!(!pull_request_status_is_stack_green(&unknown));
}

#[test]
fn pull_request_status_policy_moves_auto_merge_prerequisites_out_of_test_health() {
    // Verifies: manual merge prerequisites change auto-merge state without making Chk look like failed tests.
    let config = crate::repository::RepoStackStatusConfig {
        auto_merge_labels: vec![crate::repository::AutoMergeLabelConfig {
            label: "auto-merge".to_owned(),
            when: vec![crate::repository::HiddenLabelCondition::TargetsDefaultBranch],
        }],
        auto_merge_prerequisite_checks: vec![crate::repository::AutoMergePrerequisiteCheckConfig {
            name: "^Settings( - .*)?$".to_owned(),
        }],
        ..Default::default()
    };
    let mut status = pull_request_status(38, "Settings required", false);
    status.check_status = crate::github::PullRequestCheckStatus::Failing;
    status.labels = vec![crate::github::PullRequestLabel {
        name: "auto-merge".to_owned(),
        color: "fbca04".to_owned(),
    }];
    status.checks = vec![
        crate::github::PullRequestCheck {
            name: "Settings".to_owned(),
            status: crate::github::PullRequestCheckStatus::Failing,
        },
        crate::github::PullRequestCheck {
            name: "Settings - PRODUCTION".to_owned(),
            status: crate::github::PullRequestCheckStatus::Failing,
        },
        crate::github::PullRequestCheck {
            name: "ci/build".to_owned(),
            status: crate::github::PullRequestCheckStatus::Passing,
        },
    ];

    let filtered = apply_pull_request_status_policy(status, &config);

    assert_eq!(
        filtered.check_status,
        crate::github::PullRequestCheckStatus::Passing
    );
    assert_eq!(
        filtered
            .checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<Vec<_>>(),
        ["ci/build"]
    );
    assert_eq!(
        filtered.auto_merge_status,
        crate::github::PullRequestAutoMergeStatus::PrerequisitesRequired
    );
}

#[test]
fn pull_request_stack_status_maintenance_attaches_branch_only_statuses() {
    // Verifies: status refresh can repair stack nodes created before their PR number was cached.
    let metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![StackMetadataNode {
            branch: "topic/branch-only-status".to_owned(),
            base_branch: "main".to_owned(),
            parent_branch: None,
            pull_request: None,
            parent_pull_request: None,
            title: "Local title".to_owned(),
            url: None,
            draft: false,
            merged: false,
            work_ids: Vec::new(),
            fixes_work_ids: Vec::new(),
        }],
    };
    let mut status = pull_request_status(451, "Example branch-only status", false);
    status.head_branch = "topic/branch-only-status".to_owned();
    status.draft = true;

    let maintained = maintain_stack_metadata_pull_request_statuses(&[status], &metadata);

    assert_eq!(maintained.nodes[0].pull_request, Some(451));
    assert_eq!(maintained.nodes[0].title, "Example branch-only status");
    assert!(maintained.nodes[0].draft);
}

#[test]
fn pull_request_stack_status_maintenance_prunes_unresolved_branch_only_nodes() {
    // Verifies: stack status cache cleanup keeps only rows backed by a GitHub pull request.
    let metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![
            StackMetadataNode {
                branch: "topic/stale-local".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: None,
                parent_pull_request: None,
                title: "Stale local change".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "topic/live-child".to_owned(),
                base_branch: "topic/stale-local".to_owned(),
                parent_branch: Some("topic/stale-local".to_owned()),
                pull_request: Some(452),
                parent_pull_request: None,
                title: "Live child".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
        ],
    };
    let mut status = pull_request_status(452, "Live child", false);
    status.head_branch = "topic/live-child".to_owned();

    let maintained = maintain_stack_metadata_pull_request_statuses(&[status], &metadata);

    assert_eq!(maintained.nodes.len(), 1);
    assert_eq!(maintained.nodes[0].branch, "topic/live-child");
    assert_eq!(maintained.nodes[0].pull_request, Some(452));
    assert_eq!(maintained.nodes[0].parent_branch, None);
    assert_eq!(maintained.nodes[0].parent_pull_request, None);
}

#[test]
fn pull_request_stack_status_maintenance_retains_recently_merged_components() {
    // Verifies: freshly merged PRs remain as progress markers instead of disappearing immediately.
    let metadata = status_maintenance_metadata();
    let now = utc_datetime("2026-06-05T12:00:00Z");

    let maintained = maintain_stack_metadata_pull_request_statuses_at(
        &[
            merged_pull_request_status(10, "Merged root", "2026-06-04T18:00:00Z"),
            merged_pull_request_status(11, "Merged child", "2026-06-04T18:00:00Z"),
            merged_pull_request_status(20, "Live mixed root", "2026-06-01T12:00:00Z"),
            pull_request_status(21, "Live mixed child", false),
        ],
        &metadata,
        now,
    );

    assert_eq!(
        maintained
            .nodes
            .iter()
            .map(|node| (node.branch.as_str(), node.title.as_str(), node.merged))
            .collect::<Vec<_>>(),
        vec![
            ("merged/root", "Merged root", true),
            ("merged/child", "Merged child", true),
            ("mixed/root", "Live mixed root", true),
            ("mixed/child", "Live mixed child", false),
        ]
    );
}

#[test]
fn pull_request_stack_status_maintenance_prunes_stale_fully_merged_components() {
    // Verifies: completed stacks still age out once they are no longer useful progress context.
    let metadata = status_maintenance_metadata();
    let now = utc_datetime("2026-06-05T12:00:00Z");

    let maintained = maintain_stack_metadata_pull_request_statuses_at(
        &[
            merged_pull_request_status(10, "Merged root", "2026-06-01T12:00:00Z"),
            merged_pull_request_status(11, "Merged child", "2026-06-01T12:00:00Z"),
            merged_pull_request_status(20, "Live mixed root", "2026-06-01T12:00:00Z"),
            pull_request_status(21, "Live mixed child", false),
        ],
        &metadata,
        now,
    );

    assert_eq!(
        maintained
            .nodes
            .iter()
            .map(|node| (node.branch.as_str(), node.title.as_str(), node.merged))
            .collect::<Vec<_>>(),
        vec![
            ("mixed/root", "Live mixed root", true),
            ("mixed/child", "Live mixed child", false),
        ]
    );
}

fn status_maintenance_metadata() -> StackMetadata {
    StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![
            StackMetadataNode {
                branch: "merged/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Cached root".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "merged/child".to_owned(),
                base_branch: "merged/root".to_owned(),
                parent_branch: Some("merged/root".to_owned()),
                pull_request: Some(11),
                parent_pull_request: Some(10),
                title: "Cached child".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "mixed/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(20),
                parent_pull_request: None,
                title: "Mixed root".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "mixed/child".to_owned(),
                base_branch: "mixed/root".to_owned(),
                parent_branch: Some("mixed/root".to_owned()),
                pull_request: Some(21),
                parent_pull_request: Some(20),
                title: "Mixed child".to_owned(),
                url: None,
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
        ],
    }
}

fn merged_pull_request_status(
    number: u64,
    title: &str,
    merged_at: &str,
) -> PullRequestStatusRecord {
    let mut status = pull_request_status(number, title, true);
    status.closed = true;
    status.merged_at = Some(merged_at.to_owned());
    status
}

fn utc_datetime(value: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(value)
        .expect("synthetic timestamp is valid")
        .with_timezone(&chrono::Utc)
}

fn pull_request_status(number: u64, title: &str, merged: bool) -> PullRequestStatusRecord {
    PullRequestStatusRecord {
        number,
        title: title.to_owned(),
        url: Some(format!(
            "https://github.com/example-owner/example-repo/pull/{number}"
        )),
        created_at: None,
        head_branch: format!("topic/{number}"),
        base_branch: "main".to_owned(),
        default_branch: Some("main".to_owned()),
        author: None,
        draft: false,
        merged,
        closed: false,
        merged_at: None,
        closed_at: None,
        check_status: crate::github::PullRequestCheckStatus::Passing,
        checks: Vec::new(),
        merge_status: crate::github::PullRequestMergeStatus::Mergeable,
        review_status: crate::github::PullRequestReviewStatus::Approved,
        auto_merge_status: crate::github::PullRequestAutoMergeStatus::NotConfigured,
        requested_reviewers: ReviewerSelection::default(),
        suggested_reviewers: Vec::new(),
        approved_reviewers: Vec::new(),
        changes_requested_reviewers: Vec::new(),
        commented_reviewers: Vec::new(),
        addressed_reviewers: Vec::new(),
        reviewer_responses: Vec::new(),
        reviewer_mentions: Vec::new(),
        dismissed_reviewers: Vec::new(),
        review_activity: Vec::new(),
        timeline_events: Vec::new(),
        labels: Vec::new(),
        latest_commit_oid: None,
    }
}

#[test]
fn check_readiness_validates_github_access_and_plans_bookmark_candidate() {
    // Verifies: Check readiness validates GitHub access and plans bookmark candidate.
    let github = FakeGitHub::default();
    let report = pollster::block_on(check_readiness(&context(), workspace_facts(), &github))
        .expect("check succeeds");

    assert_eq!(report.repository.github_slug, "example-owner/example-repo");
    assert_eq!(report.workspace.trunk_branch, "main");
    assert_eq!(report.workspace.stack_index, 2);
    assert_eq!(report.github.login, "example-user");
    assert!(report.github.can_push);
    assert_eq!(report.bookmark.branch, "example-user/02-zzzzzzzz");
    assert_eq!(report.bookmark.action, BookmarkAction::Create);
}

#[test]
fn check_readiness_rejects_missing_push_access() {
    // Verifies: Check readiness rejects missing push access.
    let github = FakeGitHub {
        access: RepositoryAccess {
            can_push: false,
            ..FakeGitHub::default().access
        },
        ..FakeGitHub::default()
    };

    let error = pollster::block_on(check_readiness(&context(), workspace_facts(), &github))
        .expect_err("push access is required");

    assert!(matches!(error, WorkflowError::MissingPushAccess { .. }));
}

#[test]
fn bookmark_report_plans_task_specific_bookmark_for_pr() {
    // Verifies: Bookmark report plans a task-specific bookmark for PR publishing.
    let github = FakeGitHub::default();
    let report = pollster::block_on(bookmark_report(
        &context(),
        workspace_facts(),
        &github,
        Some("ABC-123".to_owned()),
    ))
    .expect("bookmark plans");

    assert_eq!(report.task_id.as_deref(), Some("ABC-123"));
    assert_eq!(report.bookmark.branch, "example-user/abc-123-02-zzzzzzzz");
    assert_eq!(report.bookmark.action, BookmarkAction::Create);
}

#[test]
fn bookmark_planner_reuses_exact_existing_selected_bookmark() {
    // Verifies: Bookmark planner reuses an exact existing bookmark on the selected change.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["example-user/02-zzzzzzzz".to_owned()];
    workspace.local_bookmarks_at_target = workspace.local_bookmarks.clone();

    let plan = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: None,
        workspace: &workspace,
    })
    .expect("existing bookmark is reused");

    assert_eq!(plan.branch, "example-user/02-zzzzzzzz");
    assert_eq!(plan.action, BookmarkAction::Reuse);
}

#[test]
fn bookmark_planner_reuses_existing_task_bookmark_without_task_id() {
    // Verifies: Bookmark planner reuses existing task bookmark without task ID.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["example-user/ABC-123-01-deadbeef".to_owned()];
    workspace.local_bookmarks_at_target = workspace.local_bookmarks.clone();

    let plan = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: None,
        workspace: &workspace,
    })
    .expect("selected PR head is reused");

    assert_eq!(plan.branch, "example-user/ABC-123-01-deadbeef");
    assert_eq!(plan.action, BookmarkAction::Reuse);
}

#[test]
fn bookmark_planner_reuses_existing_task_bookmark_case_insensitively() {
    // Verifies: Existing mixed-case task bookmarks remain reusable after new names became lowercase.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["example-user/ABC-123-02-deadbeef".to_owned()];
    workspace.local_bookmarks_at_target = workspace.local_bookmarks.clone();

    let plan = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: Some("abc-123"),
        workspace: &workspace,
    })
    .expect("existing task bookmark is reused");

    assert_eq!(plan.branch, "example-user/ABC-123-02-deadbeef");
    assert_eq!(plan.action, BookmarkAction::Reuse);
}

#[test]
fn bookmark_planner_formats_multi_digit_stack_indices() {
    // Verifies: Bookmark planner formats multi-digit stack indices.
    let mut workspace = workspace_facts();
    workspace.stack_index = 12;
    workspace.target_change.change_id = "mnopqrstuv".to_owned();
    workspace.target_change.short_commit_id = "deadbeef".to_owned();

    let default = plan_bookmark(BookmarkPlanRequest {
        github_login: "Example-User",
        task_id: None,
        workspace: &workspace,
    })
    .expect("default bookmark plans");
    let task = plan_bookmark(BookmarkPlanRequest {
        github_login: "Example-User",
        task_id: Some("ABC-123"),
        workspace: &workspace,
    })
    .expect("task bookmark plans");

    assert_eq!(default.branch, "example-user/12-mnopqrst");
    assert_eq!(task.branch, "example-user/abc-123-12-mnopqrst");
}

#[test]
fn bookmark_planner_rejects_invalid_task_id() {
    // Verifies: Bookmark planner rejects invalid task ID.
    let error = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: Some("ABC/123"),
        workspace: &workspace_facts(),
    })
    .expect_err("invalid task id is rejected");

    assert!(matches!(error, WorkflowError::InvalidTaskId { .. }));
}

#[test]
fn bookmark_planner_allows_same_task_bookmark_on_another_change() {
    // Verifies: Task-scoped PR planning can create a new PR head for separate same-task work.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["example-user/ABC-123-01-deadbeef".to_owned()];
    workspace.local_bookmarks_at_target = Vec::new();

    let plan = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: Some("ABC-123"),
        workspace: &workspace,
    })
    .expect("same-task bookmark on another change does not block new PR head");

    assert_eq!(plan.branch, "example-user/abc-123-02-zzzzzzzz");
    assert_eq!(plan.action, BookmarkAction::Create);
}

#[test]
fn bookmark_planner_reuses_selected_bookmark_when_task_id_differs() {
    // Verifies: An existing PR head on the selected change wins over a generated task bookmark.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["example-user/02-a1b2c3d4".to_owned()];
    workspace.local_bookmarks_at_target = workspace.local_bookmarks.clone();

    let plan = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: Some("ABC-123"),
        workspace: &workspace,
    })
    .expect("selected bookmark is reused");

    assert_eq!(plan.branch, "example-user/02-a1b2c3d4");
    assert_eq!(plan.action, BookmarkAction::Reuse);
}

#[test]
fn bookmark_planner_rejects_ambiguous_selected_bookmarks() {
    // Verifies: Bookmark planner rejects ambiguous selected-change bookmarks.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec![
        "example-user/02-a1b2c3d4".to_owned(),
        "example-user/ABC-123-01-deadbeef".to_owned(),
    ];
    workspace.local_bookmarks_at_target = workspace.local_bookmarks.clone();

    let error = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: None,
        workspace: &workspace,
    })
    .expect_err("multiple selected bookmarks are ambiguous");

    assert!(matches!(
        error,
        WorkflowError::AmbiguousSelectedBookmarks { .. }
    ));
}

#[test]
fn bookmark_planner_rejects_generated_name_on_another_change() {
    // Verifies: Bookmark planner rejects generated name on another change.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["example-user/02-zzzzzzzz".to_owned()];

    let error = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: None,
        workspace: &workspace,
    })
    .expect_err("generated bookmark already exists elsewhere");

    assert!(matches!(
        error,
        WorkflowError::BookmarkExistsOnDifferentChange { branch }
            if branch == "example-user/02-zzzzzzzz"
    ));
}

#[test]
fn push_plan_reuses_requested_bookmark_or_first_selected_bookmark() {
    // Verifies: Push planning preserves an explicit bookmark selection and otherwise reuses
    // the selected change's first local bookmark.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks_at_target = vec![
        "example-user/selected".to_owned(),
        "example-user/other".to_owned(),
    ];
    workspace.local_bookmarks = workspace.local_bookmarks_at_target.clone();

    let requested = push_plan(&context(), workspace.clone(), Some("example-user/other"))
        .expect("requested bookmark is planned");
    let default = push_plan(&context(), workspace, Some("a1b2c3d4"))
        .expect("first selected bookmark is planned");

    assert_eq!(requested.bookmark.branch, "example-user/other");
    assert_eq!(requested.bookmark.action, BookmarkAction::Reuse);
    assert_eq!(default.bookmark.branch, "example-user/selected");
    assert_eq!(default.bookmark.action, BookmarkAction::Reuse);
}

#[test]
fn push_plan_generates_ticket_or_change_bookmark_when_selected_change_has_none() {
    // Verifies: Push planning creates readable bookmark names only when no local bookmark
    // already identifies the selected change.
    let generic = push_plan(&context(), workspace_facts(), None).expect("generic push plans");
    let mut ticket_workspace = workspace_facts();
    ticket_workspace.target_change.description = "task-12345 make checkout faster".to_owned();

    let ticket = push_plan(&context(), ticket_workspace, None).expect("ticket push plans");

    assert_eq!(generic.bookmark.branch, "push-zzzzzzzz");
    assert_eq!(generic.bookmark.action, BookmarkAction::Create);
    assert_eq!(ticket.bookmark.branch, "ps/task-12345-02-zzzzzzzz");
    assert_eq!(ticket.bookmark.action, BookmarkAction::Create);
}

#[test]
fn push_plan_rejects_generated_bookmark_on_another_change() {
    // Verifies: Push planning refuses to reuse a generated name that already points elsewhere.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["push-zzzzzzzz".to_owned()];

    let error = push_plan(&context(), workspace, None)
        .expect_err("generated bookmark collision is rejected");

    assert!(matches!(
        error,
        WorkflowError::PushBookmarkExistsOnDifferentChange { branch }
            if branch == "push-zzzzzzzz"
    ));
}

#[test]
fn status_compares_github_branch_to_local_trunk_sha() {
    // Verifies: Status compares GitHub branch to local trunk SHA.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let github = FakeGitHub {
        comparison: CommitComparison {
            status: ComparisonStatus::Ahead,
            ahead_by: 3,
            behind_by: 0,
        },
        compare_calls: calls.clone(),
        ..FakeGitHub::default()
    };

    let report = pollster::block_on(status_report(&context(), status_workspace_facts(), &github))
        .expect("status succeeds");

    assert_eq!(
        *calls.lock().expect("compare calls"),
        vec![("1111222233334444".to_owned(), "main".to_owned())]
    );
    assert_eq!(report.remotes[0].name, "origin");
    assert_eq!(report.remotes[0].comparison.state, StatusState::GithubAhead);
    assert_eq!(report.remotes[0].comparison.github_ahead_by, 3);
    assert_eq!(report.remotes[0].local_ahead_by, 2);
}

#[test]
fn stack_trunk_status_uses_branch_head_without_exact_compare() {
    // Verifies: Stack status only needs a cheap changed/not-changed trunk signal.
    let compare_calls = Arc::new(Mutex::new(Vec::new()));
    let branch_head_calls = Arc::new(Mutex::new(Vec::new()));
    let github = FakeGitHub {
        branch_head_sha: "aaaabbbbccccdddd".to_owned(),
        branch_head_calls: branch_head_calls.clone(),
        compare_calls: compare_calls.clone(),
        ..FakeGitHub::default()
    };

    let report = pollster::block_on(stack_trunk_status_report(
        &context(),
        status_workspace_facts(),
        &github,
    ))
    .expect("stack trunk status succeeds");

    assert_eq!(
        *branch_head_calls.lock().expect("branch head calls"),
        vec!["main".to_owned()]
    );
    assert!(compare_calls.lock().expect("compare calls").is_empty());
    assert_eq!(report.name, "origin");
    assert_eq!(report.comparison.state, StatusState::GithubAhead);
    assert_eq!(report.comparison.github_ahead_by, 0);
    assert!(!report.comparison.counts_exact);
}

#[test]
fn status_reports_configured_github_remotes_in_context_order() {
    // Verifies: Status reports all configured GitHub remotes in context order.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let github = FakeGitHub {
        comparison: CommitComparison {
            status: ComparisonStatus::Identical,
            ahead_by: 0,
            behind_by: 0,
        },
        compare_calls: calls.clone(),
        ..FakeGitHub::default()
    };
    let mut context = context();
    context.github_remotes.push(GitHubRemote {
        name: "upstream".to_owned(),
        url: "git@github.com:upstream-owner/example-repo.git".to_owned(),
        github: GitHubRepository {
            owner: "upstream-owner".to_owned(),
            name: "example-repo".to_owned(),
        },
    });
    let mut workspace = status_workspace_facts();
    workspace.remotes.push(StatusRemoteFacts {
        remote: "upstream".to_owned(),
        branch: "main".to_owned(),
        trunk_git_commit_sha: "aaaabbbbccccdddd".to_owned(),
        trunk_short_commit_id: "aaaabbbb".to_owned(),
        local_ahead_by: 1,
    });

    let report =
        pollster::block_on(status_report(&context, workspace, &github)).expect("status succeeds");

    assert_eq!(
        report
            .remotes
            .iter()
            .map(|remote| remote.name.as_str())
            .collect::<Vec<_>>(),
        vec!["origin", "upstream"]
    );
    assert_eq!(
        *calls.lock().expect("compare calls"),
        vec![
            ("1111222233334444".to_owned(), "main".to_owned()),
            ("aaaabbbbccccdddd".to_owned(), "main".to_owned()),
        ]
    );
}

#[test]
fn remote_status_report_compares_origin_fork_with_source() {
    // Verifies: Remote-status adds fork/source freshness without changing remote freshness.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut comparisons = BTreeMap::new();
    comparisons.insert(
        ("1111222233334444".to_owned(), "main".to_owned()),
        CommitComparison {
            status: ComparisonStatus::Identical,
            ahead_by: 0,
            behind_by: 0,
        },
    );
    comparisons.insert(
        ("main".to_owned(), "example-owner:main".to_owned()),
        CommitComparison {
            status: ComparisonStatus::Behind,
            ahead_by: 0,
            behind_by: 7,
        },
    );
    let github = FakeGitHub {
        repository_fork: Some(RepositoryFork {
            source: GitHubRepository {
                owner: "source-owner".to_owned(),
                name: "example-repo".to_owned(),
            },
            source_default_branch: Some("main".to_owned()),
        }),
        comparisons,
        compare_calls: calls.clone(),
        ..FakeGitHub::default()
    };

    let report = pollster::block_on(remote_status_report(
        &context(),
        status_workspace_facts(),
        &github,
    ))
    .expect("remote status succeeds");
    let fork = report.fork.expect("fork status is present");

    assert_eq!(fork.source.slug(), "source-owner/example-repo");
    assert_eq!(fork.comparison.state, ForkStatusState::SourceAhead);
    assert_eq!(fork.comparison.source_ahead_by, 7);
    assert_eq!(fork.comparison.fork_ahead_by, 0);
    assert_eq!(
        *calls.lock().expect("compare calls"),
        vec![
            ("1111222233334444".to_owned(), "main".to_owned()),
            ("main".to_owned(), "example-owner:main".to_owned()),
        ]
    );
}

#[test]
fn status_maps_all_github_comparison_states() {
    // Verifies: Status maps all GitHub comparison states to stable freshness states.
    let cases = [
        (ComparisonStatus::Identical, StatusState::UpToDate, 0, 0),
        (ComparisonStatus::Ahead, StatusState::GithubAhead, 3, 0),
        (ComparisonStatus::Behind, StatusState::LocalAhead, 0, 2),
        (ComparisonStatus::Diverged, StatusState::Diverged, 4, 5),
    ];

    for (github_status, expected_state, ahead_by, behind_by) in cases {
        let github = FakeGitHub {
            comparison: CommitComparison {
                status: github_status,
                ahead_by,
                behind_by,
            },
            ..FakeGitHub::default()
        };

        let report =
            pollster::block_on(status_report(&context(), status_workspace_facts(), &github))
                .expect("status succeeds");
        let comparison = &report.remotes[0].comparison;

        assert_eq!(comparison.state, expected_state);
        assert_eq!(comparison.github_ahead_by, ahead_by);
        assert_eq!(comparison.github_behind_by, behind_by);
    }
}

#[test]
fn status_rejects_unknown_github_comparison() {
    // Verifies: Status rejects unknown GitHub comparison.
    let github = FakeGitHub {
        comparison: CommitComparison {
            status: ComparisonStatus::Unknown,
            ahead_by: 0,
            behind_by: 0,
        },
        ..FakeGitHub::default()
    };

    let error = pollster::block_on(status_report(&context(), status_workspace_facts(), &github))
        .expect_err("unknown comparison is unavailable");

    assert!(matches!(error, WorkflowError::UnavailableComparison { .. }));
}

#[test]
fn status_surfaces_auth_failure_and_missing_comparison_targets() {
    // Verifies: Status surfaces auth failure and missing comparison targets.
    let cases = [
        FakeCompareFailure::Authentication,
        FakeCompareFailure::MissingBranch,
        FakeCompareFailure::UnknownLocalSha,
    ];

    for failure in cases {
        let github = FakeGitHub {
            compare_failure: Some(failure.clone()),
            ..FakeGitHub::default()
        };

        let error =
            pollster::block_on(status_report(&context(), status_workspace_facts(), &github))
                .expect_err("GitHub comparison failure is surfaced");

        match failure {
            FakeCompareFailure::Authentication => assert!(matches!(
                error,
                WorkflowError::GitHub(GitHubError::AuthenticationFailed {
                    operation: "compare commits",
                    ..
                })
            )),
            FakeCompareFailure::MissingBranch | FakeCompareFailure::UnknownLocalSha => {
                assert!(matches!(
                    error,
                    WorkflowError::GitHub(GitHubError::ComparisonTargetNotFound { .. })
                ));
                assert!(error.to_string().contains("Run `jx fetch`"));
            }
        }
    }
}

#[test]
fn pull_request_work_ids_prefer_title_prefix_over_task_id() {
    // Verifies: stack metadata records the title ticket when it differs from workspace task context.
    assert_eq!(
        pull_request_work_ids("XYZ-9: Update endpoint", Some("ABC-123"), &[]),
        ["XYZ-9".to_owned()]
    );
    assert_eq!(
        pull_request_work_ids("[XYZ-9] Update endpoint", Some("ABC-123"), &[]),
        ["XYZ-9".to_owned()]
    );
    assert_eq!(
        pull_request_work_ids("Update endpoint", Some("ABC-123"), &["DEF-456".to_owned()]),
        ["ABC-123".to_owned(), "DEF-456".to_owned()]
    );
}

#[test]
fn prepare_pull_request_change_prepends_task_id_to_commit_title() {
    // Verifies: PR preparation updates the selected commit title before PR planning.
    let context = context_with_event_handlers(vec![prepend_task_id_handler(
        "prepend-task",
        query([has_task()]),
    )]);
    let mut workspace = workspace_facts();
    workspace.target_change.description = "Example title\n\nDetailed body".to_owned();

    let report = prepare_pull_request_change(
        &context,
        &workspace,
        Some("ABC-123"),
        PullRequestPublishOptions::default(),
    );

    assert!(report.changed);
    assert_eq!(
        report.description,
        "ABC-123: Example title\n\nDetailed body"
    );
    assert_eq!(
        report.event_effects,
        vec![PullRequestEventEffect {
            event: RepoEvent::PullRequestPrepare,
            handler_id: Some("prepend-task".to_owned()),
            kind: PullRequestEventEffectKind::UpdatedTitle {
                title: "ABC-123: Example title".to_owned(),
            },
        }]
    );
}

#[test]
fn prepare_pull_request_change_normalizes_existing_matching_task_title_prefix() {
    // Verifies: Prepend-task-id fixes common matching task title shapes idempotently.
    let context = context_with_event_handlers(vec![prepend_task_id_handler(
        "prepend-task",
        query([has_task()]),
    )]);
    let cases = [
        ("ABC-123 Example title", "ABC-123: Example title", true),
        ("ABC-123 - Example title", "ABC-123: Example title", true),
        ("[ABC-123]: Example title", "ABC-123: Example title", true),
        ("ABC-123: Example title", "ABC-123: Example title", false),
        ("XYZ-9: Example title", "XYZ-9: Example title", false),
        ("Update XYZ-9 behavior", "Update XYZ-9 behavior", false),
    ];

    for (input, expected, changed) in cases {
        let mut workspace = workspace_facts();
        workspace.target_change.description = input.to_owned();

        let report = prepare_pull_request_change(
            &context,
            &workspace,
            Some("ABC-123"),
            PullRequestPublishOptions::default(),
        );

        assert_eq!(report.changed, changed, "{input}");
        assert_eq!(report.description, expected, "{input}");
    }
}

#[test]
fn prepare_pull_request_change_skips_without_task_id() {
    // Verifies: `has:task` gates commit-title rewriting when no task id is resolved.
    let context = context_with_event_handlers(vec![prepend_task_id_handler(
        "prepend-task",
        query([has_task()]),
    )]);
    let mut workspace = workspace_facts();
    workspace.target_change.description = "Example title".to_owned();

    let report = prepare_pull_request_change(
        &context,
        &workspace,
        None,
        PullRequestPublishOptions::default(),
    );

    assert!(!report.changed);
    assert_eq!(report.description, "Example title");
    assert!(report.event_effects.is_empty());
}

#[test]
fn pull_request_plan_derives_metadata_bookmark_stack_base_and_reviewers() {
    // Verifies: Pull request plan uses the nearest stack bookmark as the PR base.
    let github = FakeGitHub::default();
    let mut workspace = workspace_facts();
    workspace.target_change.description = "Example title\n\nDetailed body".to_owned();

    let plan = pollster::block_on(pull_request_plan(
        &context_with_path_reviewers(&["example-reviewer", "second-reviewer"]),
        workspace,
        &github,
        Some("ABC-123".to_owned()),
        vec!["bug".to_owned(), "help wanted".to_owned()],
        PullRequestReadiness::Draft,
    ))
    .expect("PR plan is derived");

    assert_eq!(plan.title, "Example title");
    assert_eq!(plan.body, "Detailed body");
    assert_eq!(plan.target_commit_id, "a1b2c3d4e5f6");
    assert_eq!(plan.changed_files, ["src/main.rs".to_owned()]);
    assert!(plan.draft);
    assert_eq!(plan.base, "example-user/01-ancestor");
    assert_eq!(
        plan.head.label(),
        "example-owner:example-user/abc-123-02-zzzzzzzz"
    );
    assert_eq!(plan.bookmark.branch, "example-user/abc-123-02-zzzzzzzz");
    assert_eq!(plan.labels, ["bug".to_owned(), "help wanted".to_owned()]);
    assert_eq!(
        plan.reviewer_candidates
            .iter()
            .map(|candidate| candidate.target.display_name())
            .collect::<Vec<_>>(),
        vec!["example-reviewer", "second-reviewer"]
    );
    assert_eq!(
        plan.reviewers.users,
        ["example-reviewer".to_owned(), "second-reviewer".to_owned()]
    );
}

#[test]
fn pull_request_plan_leaves_repo_level_reviewers_completion_only() {
    // Verifies: Repo-level reviewer lists do not become automatic publish candidates.
    let github = FakeGitHub::default();

    let plan = pollster::block_on(pull_request_plan(
        &context_with_reviewers(&["possible-reviewer"]),
        workspace_facts(),
        &github,
        None,
        Vec::new(),
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");

    assert!(plan.reviewer_candidates.is_empty());
    assert_eq!(plan.reviewers, ReviewerSelection::default());
}

#[test]
fn pull_request_plan_merges_existing_requested_reviewers() {
    // Verifies: existing PR reviewers stay selected while computed reviewers remain available.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Existing PR".to_owned(),
            body: None,
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::new(["already-reviewer"], ["platform"]),
        }),
        ..FakeGitHub::default()
    };

    let plan = pollster::block_on(pull_request_plan(
        &context_with_path_reviewers(&["computed-reviewer"]),
        workspace_facts(),
        &github,
        None,
        Vec::new(),
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");

    assert_eq!(
        plan.reviewers,
        ReviewerSelection::new(["already-reviewer", "computed-reviewer"], ["platform"])
    );
    let existing = plan
        .reviewer_candidates
        .iter()
        .find(|candidate| candidate.target.display_name() == "already-reviewer")
        .expect("existing reviewer is offered");
    assert_eq!(existing.reasons, ["already requested".to_owned()]);
}

#[test]
fn pull_request_plan_offers_existing_review_activity() {
    // Verifies: Reviewers who already approved or commented stay visible even after GitHub clears requests.
    let mut status = pull_request_status(7, "Existing PR", false);
    status.approved_reviewers = vec!["approved-reviewer".to_owned()];
    status.commented_reviewers = vec!["comment-reviewer".to_owned()];
    status.addressed_reviewers = vec!["addressed-reviewer".to_owned()];
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Existing PR".to_owned(),
            body: None,
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::new(["requested-reviewer"], Vec::<String>::new()),
        }),
        pull_request_statuses: vec![status],
        ..FakeGitHub::default()
    };

    let plan = pollster::block_on(pull_request_plan(
        &context(),
        workspace_facts(),
        &github,
        None,
        Vec::new(),
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");

    assert_eq!(
        plan.reviewer_candidates
            .iter()
            .map(|candidate| (
                candidate.target.display_name().to_owned(),
                candidate.reasons.clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "requested-reviewer".to_owned(),
                vec!["already requested".to_owned()]
            ),
            (
                "approved-reviewer".to_owned(),
                vec!["already approved".to_owned()]
            ),
            ("comment-reviewer".to_owned(), vec!["commented".to_owned()]),
            (
                "addressed-reviewer".to_owned(),
                vec!["comments addressed".to_owned()]
            ),
        ]
    );
    assert_eq!(
        plan.reviewers,
        ReviewerSelection::new(["requested-reviewer"], Vec::<String>::new())
    );
}

#[test]
fn pull_request_plan_offers_suggested_reviewers_for_ready_draft_pr() {
    // Verifies: GitHub suggestions become prompt candidates only when an existing draft is marked ready.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Existing draft PR".to_owned(),
            body: None,
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: true,
            merged: false,
            reviewers: ReviewerSelection::new(["already-reviewer"], Vec::<String>::new()),
        }),
        suggested_reviewers: vec![
            "suggested-reviewer".to_owned(),
            "already-reviewer".to_owned(),
        ],
        ..FakeGitHub::default()
    };

    let plan = pollster::block_on(pull_request_plan(
        &context_with_path_reviewers(&["computed-reviewer"]),
        workspace_facts(),
        &github,
        None,
        Vec::new(),
        PullRequestReadiness::Ready,
    ))
    .expect("PR plan is derived");

    assert!(!plan.draft);
    assert_eq!(
        github
            .suggested_reviewer_calls
            .lock()
            .expect("suggested reviewer calls")
            .as_slice(),
        &[7]
    );
    assert_eq!(
        plan.reviewer_candidates
            .iter()
            .map(|candidate| candidate.target.display_name())
            .collect::<Vec<_>>(),
        [
            "computed-reviewer",
            "already-reviewer",
            "suggested-reviewer"
        ]
    );
    assert_eq!(
        plan.reviewers,
        ReviewerSelection::new(
            ["already-reviewer", "computed-reviewer"],
            Vec::<String>::new()
        )
    );
    let suggested = plan
        .reviewer_candidates
        .iter()
        .find(|candidate| candidate.target.display_name() == "suggested-reviewer")
        .expect("suggested reviewer is offered");
    assert_eq!(suggested.reasons, ["suggested by GitHub".to_owned()]);
    let already = plan
        .reviewer_candidates
        .iter()
        .find(|candidate| candidate.target.display_name() == "already-reviewer")
        .expect("existing reviewer is offered");
    assert_eq!(
        already.reasons,
        [
            "already requested".to_owned(),
            "suggested by GitHub".to_owned()
        ]
    );
}

#[test]
fn pull_request_plan_uses_trunk_branch_when_no_ancestor_bookmark_exists() {
    // Verifies: Pull request planning still uses trunk when no stack ancestor bookmark exists.
    let github = FakeGitHub::default();
    let mut workspace = workspace_facts();
    workspace.nearest_ancestor_bookmark = None;

    let plan = pollster::block_on(pull_request_plan(
        &context(),
        workspace,
        &github,
        None,
        Vec::new(),
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");

    assert_eq!(plan.base, "main");
}

#[test]
fn pull_request_plan_rejects_empty_or_undescribed_changes() {
    // Verifies: Pull request plan rejects empty or undescribed changes.
    let github = FakeGitHub::default();
    let mut workspace = workspace_facts();
    workspace.target_change.is_empty = true;

    let empty = pollster::block_on(pull_request_plan(
        &context(),
        workspace,
        &github,
        None,
        Vec::new(),
        PullRequestReadiness::Preserve,
    ))
    .expect_err("empty changes do not create PR state");

    assert!(matches!(empty, WorkflowError::EmptyPullRequestChange));

    let mut workspace = workspace_facts();
    workspace.target_change.description = " \n\t ".to_owned();

    let missing_description = pollster::block_on(pull_request_plan(
        &context(),
        workspace,
        &github,
        None,
        Vec::new(),
        PullRequestReadiness::Preserve,
    ))
    .expect_err("description is required");

    assert!(matches!(
        missing_description,
        WorkflowError::MissingPullRequestDescription
    ));
}

#[test]
fn pull_request_description_omits_title_from_body_and_preserves_body_formatting() {
    // Verifies: PR bodies do not repeat the title while keeping meaningful body indentation.
    let (title, body) = pull_request_description_from_text(
        "\n  Example title  \n\n  Indented first line\nSecond line\n\n",
    )
    .expect("description parses");

    assert_eq!(title, "Example title");
    assert_eq!(body, "  Indented first line\nSecond line");
}

#[test]
fn pull_request_description_accepts_title_only_descriptions() {
    // Verifies: Title-only descriptions produce an empty PR body instead of repeating the title.
    let (title, body) =
        pull_request_description_from_text("Example title").expect("description parses");

    assert_eq!(title, "Example title");
    assert_eq!(body, "");
}

#[test]
fn publish_pull_request_creates_pr_and_syncs_configured_reviewers() {
    // Verifies: Publish pull request creates PR and syncs matched path reviewers.
    let github = FakeGitHub {
        reviewer_result: ReviewerSyncResult {
            requested_users: vec!["example-reviewer".to_owned()],
            ..ReviewerSyncResult::default()
        },
        ..FakeGitHub::default()
    };
    let create_calls = github.create_calls.clone();
    let reviewer_calls = github.reviewer_calls.clone();
    let context = context_with_path_reviewers(&["example-reviewer"]);
    let plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &github,
        Some("ABC-123".to_owned()),
        Vec::new(),
        PullRequestReadiness::Draft,
    ))
    .expect("PR plan is derived");

    let report = pollster::block_on(publish_pull_request(
        &context,
        plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &github,
    ))
    .expect("PR is created");

    assert_eq!(report.action, PullRequestAction::Created);
    assert_eq!(report.pull_request.number, 42);
    assert_eq!(report.base, "example-user/01-ancestor");
    assert!(report.reviewers.is_some());
    let create_calls = create_calls.lock().expect("create calls");
    assert_eq!(create_calls.len(), 1);
    assert!(create_calls[0].draft);
    drop(create_calls);
    assert_eq!(reviewer_calls.lock().expect("reviewer calls").len(), 1);
    assert_eq!(
        reviewer_calls.lock().expect("reviewer calls")[0].1.users,
        ["example-reviewer".to_owned()]
    );
}

#[test]
fn publish_pull_request_applies_requested_labels_to_created_or_updated_pr() {
    // Verifies: CLI-supplied labels are applied after either PR create or update.
    let create_github = FakeGitHub {
        label_result: LabelApplyResult {
            labels: vec!["bug".to_owned(), "help wanted".to_owned()],
        },
        ..FakeGitHub::default()
    };
    let create_label_calls = create_github.label_calls.clone();
    let context = context();
    let create_plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &create_github,
        None,
        vec!["bug".to_owned(), "help wanted".to_owned()],
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");

    let create_report = pollster::block_on(publish_pull_request(
        &context,
        create_plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &create_github,
    ))
    .expect("PR is created");

    assert_eq!(create_report.action, PullRequestAction::Created);
    assert_eq!(
        create_report.labels.expect("labels applied").labels,
        vec!["bug".to_owned(), "help wanted".to_owned()]
    );
    assert_eq!(
        create_label_calls.lock().expect("label calls").as_slice(),
        &[(42, vec!["bug".to_owned(), "help wanted".to_owned()])]
    );

    let update_github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Old title".to_owned(),
            body: Some("Old body".to_owned()),
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let update_label_calls = update_github.label_calls.clone();
    let update_plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &update_github,
        None,
        vec!["release-note".to_owned()],
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");

    let update_report = pollster::block_on(publish_pull_request(
        &context,
        update_plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &update_github,
    ))
    .expect("PR is updated");

    assert_eq!(update_report.action, PullRequestAction::Updated);
    assert_eq!(
        update_label_calls.lock().expect("label calls").as_slice(),
        &[(7, vec!["release-note".to_owned()])]
    );
}

#[test]
fn publish_pull_request_runs_created_event_handlers_against_cli_labels() {
    // Verifies: Created-PR handlers can match CLI labels, add labels, and request browser opening.
    let github = FakeGitHub::default();
    let label_calls = github.label_calls.clone();
    let context = context_with_event_handlers(vec![
        add_label_handler(
            "queue-unreviewed",
            RepoEvent::PullRequestCreated,
            query([label("seed"), not(has_reviewers()), not(draft())]),
            ["queued"],
        ),
        open_pull_request_handler(
            "open-queued",
            RepoEvent::PullRequestCreated,
            query([label("queued")]),
        ),
    ]);
    let plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &github,
        None,
        vec!["seed".to_owned()],
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");

    let report = pollster::block_on(publish_pull_request(
        &context,
        plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &github,
    ))
    .expect("PR is created");

    assert_eq!(
        label_calls.lock().expect("label calls").as_slice(),
        &[
            (42, vec!["seed".to_owned()]),
            (42, vec!["queued".to_owned()])
        ]
    );
    assert_eq!(
        report.event_effects,
        vec![
            PullRequestEventEffect {
                event: RepoEvent::PullRequestCreated,
                handler_id: Some("queue-unreviewed".to_owned()),
                kind: PullRequestEventEffectKind::AddLabels {
                    labels: vec!["queued".to_owned()],
                },
            },
            PullRequestEventEffect {
                event: RepoEvent::PullRequestCreated,
                handler_id: Some("open-queued".to_owned()),
                kind: PullRequestEventEffectKind::OpenPullRequest {
                    url: "https://github.com/example-owner/example-repo/pull/42".to_owned(),
                },
            },
        ]
    );
}

#[test]
fn publish_pull_request_runs_updated_event_handlers_against_existing_and_cli_labels() {
    // Verifies: Updated-PR handlers match existing GitHub labels plus new CLI labels.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Old title".to_owned(),
            body: Some("Old body".to_owned()),
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        pull_request_labels: vec!["existing".to_owned()],
        ..FakeGitHub::default()
    };
    let label_calls = github.label_calls.clone();
    let context = context_with_event_handlers(vec![
        add_label_handler(
            "from-existing-label",
            RepoEvent::PullRequestUpdated,
            query([label("existing")]),
            ["matched-existing"],
        ),
        add_label_handler(
            "from-cli-label",
            RepoEvent::PullRequestUpdated,
            query([label("cli")]),
            ["matched-cli"],
        ),
    ]);
    let plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &github,
        None,
        vec!["cli".to_owned()],
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");

    pollster::block_on(publish_pull_request(
        &context,
        plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &github,
    ))
    .expect("PR is updated");

    assert_eq!(
        label_calls.lock().expect("label calls").as_slice(),
        &[
            (7, vec!["cli".to_owned()]),
            (7, vec!["matched-existing".to_owned()]),
            (7, vec!["matched-cli".to_owned()]),
        ]
    );
}

#[test]
fn publish_pull_request_can_disable_event_handlers() {
    // Verifies: Command options can suppress configured event handlers without dropping CLI labels.
    let github = FakeGitHub::default();
    let label_calls = github.label_calls.clone();
    let context = context_with_event_handlers(vec![add_label_handler(
        "disabled-handler",
        RepoEvent::PullRequestCreated,
        PullRequestEventQuery::default(),
        ["configured"],
    )]);
    let plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &github,
        None,
        vec!["cli".to_owned()],
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");

    let report = pollster::block_on(publish_pull_request(
        &context,
        plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions {
            event_handlers: false,
        },
        &github,
    ))
    .expect("PR is created");

    assert_eq!(
        label_calls.lock().expect("label calls").as_slice(),
        &[(42, vec!["cli".to_owned()])]
    );
    assert!(report.event_effects.is_empty());
}

#[test]
fn publish_pull_request_updates_existing_pr_without_unconfigured_reviewers() {
    // Verifies: Publish pull request updates existing PR without unconfigured reviewers.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Old title".to_owned(),
            body: Some("Old body".to_owned()),
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let update_calls = github.update_calls.clone();
    let reviewer_calls = github.reviewer_calls.clone();
    let context = context();
    let plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &github,
        None,
        Vec::new(),
        PullRequestReadiness::Preserve,
    ))
    .expect("PR plan is derived");
    assert_eq!(
        plan.existing_pull_request.as_ref().map(|pr| pr.number),
        Some(7)
    );

    let report = pollster::block_on(publish_pull_request(
        &context,
        plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &github,
    ))
    .expect("PR is updated");
    let update_calls = update_calls.lock().expect("update calls");

    assert_eq!(report.action, PullRequestAction::Updated);
    assert!(report.reviewers.is_none());
    assert_eq!(update_calls.len(), 1);
    assert_eq!(update_calls[0].0, 7);
    assert_eq!(update_calls[0].1.title.as_deref(), Some("example change"));
    assert_eq!(
        update_calls[0].1.base.as_deref(),
        Some("example-user/01-ancestor")
    );
    assert!(reviewer_calls.lock().expect("reviewer calls").is_empty());
}

#[test]
fn publish_pull_request_applies_requested_readiness_to_existing_prs() {
    // Verifies: explicit ready/draft intent changes existing PR readiness after metadata updates.
    let draft_github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Old title".to_owned(),
            body: Some("Old body".to_owned()),
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: true,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let mark_ready_calls = draft_github.mark_ready_calls.clone();
    let context = context();
    let ready_plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &draft_github,
        None,
        Vec::new(),
        PullRequestReadiness::Ready,
    ))
    .expect("PR plan is derived");

    let ready_report = pollster::block_on(publish_pull_request(
        &context,
        ready_plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &draft_github,
    ))
    .expect("PR is marked ready");

    assert!(!ready_report.pull_request.draft);
    assert_eq!(
        mark_ready_calls
            .lock()
            .expect("mark ready calls")
            .as_slice(),
        &[7]
    );

    let ready_github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 8,
            title: "Old title".to_owned(),
            body: Some("Old body".to_owned()),
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/8".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let convert_draft_calls = ready_github.convert_draft_calls.clone();
    let draft_plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &ready_github,
        None,
        Vec::new(),
        PullRequestReadiness::Draft,
    ))
    .expect("PR plan is derived");

    let draft_report = pollster::block_on(publish_pull_request(
        &context,
        draft_plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &ready_github,
    ))
    .expect("PR is marked draft");

    assert!(draft_report.pull_request.draft);
    assert_eq!(
        convert_draft_calls
            .lock()
            .expect("convert draft calls")
            .as_slice(),
        &[8]
    );
}

#[test]
fn publish_pull_request_metadata_only_skips_code_update_and_applies_allowed_metadata() {
    // Verifies: metadata-only publish avoids title/body/base updates while applying safe PR metadata.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Old title".to_owned(),
            body: Some("Old body".to_owned()),
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "legacy-base".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: true,
            merged: false,
            reviewers: ReviewerSelection::new(["existing-reviewer"], std::iter::empty::<&str>()),
        }),
        pull_request_labels: vec!["existing".to_owned()],
        label_result: LabelApplyResult {
            labels: vec!["needs-review".to_owned()],
        },
        reviewer_result: ReviewerSyncResult {
            requested_users: vec!["reviewer-one".to_owned()],
            requested_teams: vec!["platform".to_owned()],
            removed_users: Vec::new(),
            removed_teams: Vec::new(),
        },
        ..FakeGitHub::default()
    };
    let create_calls = github.create_calls.clone();
    let update_calls = github.update_calls.clone();
    let mark_ready_calls = github.mark_ready_calls.clone();
    let label_calls = github.label_calls.clone();
    let reviewer_calls = github.reviewer_calls.clone();
    let context = context();
    let mut plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &github,
        None,
        vec!["existing".to_owned(), "needs-review".to_owned()],
        PullRequestReadiness::Ready,
    ))
    .expect("PR plan is derived");
    let reviewers = ReviewerSelection::new(["reviewer-one"], ["platform"]);
    plan.reviewers = reviewers.clone();
    let push = PushOutcome {
        branch: plan.bookmark.branch.clone(),
        pushed_refs: 0,
        pushed_commits: Vec::new(),
    };

    let report = pollster::block_on(publish_pull_request_metadata_only(
        &context,
        plan,
        bookmark_update(),
        push,
        &github,
    ))
    .expect("PR metadata is updated");

    assert_eq!(report.action, PullRequestAction::Updated);
    assert_eq!(report.pull_request.title, "Old title");
    assert_eq!(report.pull_request.body.as_deref(), Some("Old body"));
    assert_eq!(report.pull_request.base_branch, "legacy-base");
    assert!(!report.pull_request.draft);
    assert!(report.event_effects.is_empty());
    assert!(create_calls.lock().expect("create calls").is_empty());
    assert!(update_calls.lock().expect("update calls").is_empty());
    assert_eq!(
        mark_ready_calls.lock().expect("ready calls").as_slice(),
        &[7]
    );
    assert_eq!(
        label_calls.lock().expect("label calls").as_slice(),
        &[(7, vec!["needs-review".to_owned()])]
    );
    assert_eq!(
        reviewer_calls.lock().expect("reviewer calls").as_slice(),
        &[(7, reviewers)]
    );
}

#[test]
fn sync_pull_requests_updates_description_without_touching_labels_reviewers_or_base() {
    // Verifies: Sync updates only existing PR title/body from the pushed local commit description.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Old title".to_owned(),
            body: Some("Old body".to_owned()),
            head_branch: "example-user/current".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let update_calls = github.update_calls.clone();
    let label_calls = github.label_calls.clone();
    let reviewer_calls = github.reviewer_calls.clone();
    let create_calls = github.create_calls.clone();
    let push = TrackedPushOutcome {
        pushed_refs: 1,
        bookmarks: vec![PushedBookmarkSummary {
            branch: "example-user/current".to_owned(),
            old_short_commit_id: Some("11112222".to_owned()),
            new_short_commit_id: Some("a1b2c3d4".to_owned()),
            old_short_change_id: Some("changeoo".to_owned()),
            new_short_change_id: Some("changecc".to_owned()),
            old_description: Some("Old title".to_owned()),
            new_description: Some("New title".to_owned()),
            pull_request_description: Some("New title\n\nNew body".to_owned()),
            pull_request_base: Some("main".to_owned()),
            new_workspace_visibility: WorkspaceVisibility::default(),
        }],
        pushed_commits: Vec::new(),
    };

    let pull_requests = pollster::block_on(sync_pull_requests(
        &context(),
        &push,
        &StackMetadata::default(),
        &github,
    ))
    .expect("pull requests sync");

    assert_eq!(pull_requests[0].number, 7);
    assert_eq!(pull_requests[0].title, "New title");
    assert_eq!(pull_requests[0].body.as_deref(), Some("New body"));
    assert_eq!(
        update_calls.lock().expect("update calls").as_slice(),
        &[(
            7,
            PullRequestUpdate {
                title: Some("New title".to_owned()),
                body: Some("New body".to_owned()),
                base: None,
            }
        )]
    );
    assert!(label_calls.lock().expect("label calls").is_empty());
    assert!(reviewer_calls.lock().expect("reviewer calls").is_empty());
    assert!(create_calls.lock().expect("create calls").is_empty());
}

#[test]
fn sync_pull_requests_updates_stack_base_without_rewriting_matching_description() {
    // Verifies: stacked PR sync can retarget a child PR while leaving matching title/body alone.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "New title".to_owned(),
            body: Some("New body".to_owned()),
            head_branch: "example-user/current".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let update_calls = github.update_calls.clone();
    let push = TrackedPushOutcome {
        pushed_refs: 1,
        bookmarks: vec![PushedBookmarkSummary {
            branch: "example-user/current".to_owned(),
            old_short_commit_id: Some("11112222".to_owned()),
            new_short_commit_id: Some("a1b2c3d4".to_owned()),
            old_short_change_id: Some("changeoo".to_owned()),
            new_short_change_id: Some("changecc".to_owned()),
            old_description: Some("Old title".to_owned()),
            new_description: Some("New title".to_owned()),
            pull_request_description: Some("New title\n\nNew body".to_owned()),
            pull_request_base: Some("example-user/parent".to_owned()),
            new_workspace_visibility: WorkspaceVisibility::default(),
        }],
        pushed_commits: Vec::new(),
    };

    let pull_requests = pollster::block_on(sync_pull_requests(
        &context(),
        &push,
        &StackMetadata::default(),
        &github,
    ))
    .expect("pull requests sync");

    assert_eq!(pull_requests[0].base_branch, "example-user/parent");
    assert_eq!(
        update_calls.lock().expect("update calls").as_slice(),
        &[(
            7,
            PullRequestUpdate {
                title: None,
                body: None,
                base: Some("example-user/parent".to_owned()),
            }
        )]
    );
}

#[test]
fn sync_pull_requests_adds_stack_context_from_metadata() {
    // Verifies: sync renders stack context from durable local state without editing authored body text.
    let context = context();
    let stack_metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![
            StackMetadataNode {
                branch: "example-user/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(6),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/6".to_owned()),
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "example-user/child".to_owned(),
                base_branch: "example-user/root".to_owned(),
                parent_branch: Some("example-user/root".to_owned()),
                pull_request: Some(7),
                parent_pull_request: Some(6),
                title: "Child".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "example-user/draft".to_owned(),
                base_branch: "example-user/child".to_owned(),
                parent_branch: Some("example-user/child".to_owned()),
                pull_request: Some(8),
                parent_pull_request: Some(7),
                title: "Draft".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/8".to_owned()),
                draft: true,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
        ],
    };
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Child".to_owned(),
            body: Some("Authored body".to_owned()),
            head_branch: "example-user/child".to_owned(),
            base_branch: "example-user/root".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let update_calls = github.update_calls.clone();
    let push = TrackedPushOutcome {
        pushed_refs: 1,
        bookmarks: vec![PushedBookmarkSummary {
            branch: "example-user/child".to_owned(),
            old_short_commit_id: Some("old".to_owned()),
            new_short_commit_id: Some("new".to_owned()),
            old_short_change_id: Some("changeoo".to_owned()),
            new_short_change_id: Some("changecc".to_owned()),
            old_description: None,
            new_description: None,
            pull_request_description: Some("Child\n\nAuthored body".to_owned()),
            pull_request_base: Some("example-user/root".to_owned()),
            new_workspace_visibility: WorkspaceVisibility::default(),
        }],
        pushed_commits: Vec::new(),
    };

    pollster::block_on(sync_pull_requests(
        &context,
        &push,
        &stack_metadata,
        &github,
    ))
    .expect("pull requests sync");

    assert_eq!(
        update_calls.lock().expect("update calls").as_slice(),
        &[(
            7,
            PullRequestUpdate {
                title: None,
                body: Some(
                    "Authored body\n\n<!-- jx-stack:start -->\n### Pull request stack\n\n◯ [#6 Root](https://github.com/example-owner/example-repo/pull/6)\n└ ◉ **[#7 Child](https://github.com/example-owner/example-repo/pull/7)** — this PR\n&nbsp;&nbsp;└ ◌ [#8 Draft](https://github.com/example-owner/example-repo/pull/8) — draft\n<!-- jx-stack:end -->"
                        .to_owned()
                ),
                base: None,
            }
        )]
    );
}

#[test]
fn sync_pull_requests_falls_back_to_metadata_number_when_head_lookup_misses() {
    // Verifies: immediate post-publish sync does not depend on GitHub head search indexing the new PR.
    let context = context();
    let stack_metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: Vec::new(),
        nodes: vec![
            StackMetadataNode {
                branch: "example-user/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(6),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/6".to_owned()),
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
            StackMetadataNode {
                branch: "example-user/child".to_owned(),
                base_branch: "example-user/root".to_owned(),
                parent_branch: Some("example-user/root".to_owned()),
                pull_request: Some(7),
                parent_pull_request: Some(6),
                title: "Child".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            },
        ],
    };
    let child = PullRequestRecord {
        number: 7,
        title: "Child".to_owned(),
        body: Some("Authored body".to_owned()),
        head_branch: "example-user/child".to_owned(),
        base_branch: "example-user/root".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
        draft: false,
        merged: false,
        reviewers: ReviewerSelection::default(),
    };
    let github = FakeGitHub {
        open_pull_request: None,
        pull_requests_by_number: BTreeMap::from([(7, child)]),
        ..FakeGitHub::default()
    };
    let update_calls = github.update_calls.clone();
    let push = TrackedPushOutcome {
        pushed_refs: 1,
        bookmarks: vec![PushedBookmarkSummary {
            branch: "example-user/child".to_owned(),
            old_short_commit_id: Some("old".to_owned()),
            new_short_commit_id: Some("new".to_owned()),
            old_short_change_id: Some("changeoo".to_owned()),
            new_short_change_id: Some("changecc".to_owned()),
            old_description: None,
            new_description: None,
            pull_request_description: Some("Child\n\nAuthored body".to_owned()),
            pull_request_base: Some("example-user/root".to_owned()),
            new_workspace_visibility: WorkspaceVisibility::default(),
        }],
        pushed_commits: Vec::new(),
    };

    pollster::block_on(sync_pull_requests(
        &context,
        &push,
        &stack_metadata,
        &github,
    ))
    .expect("pull requests sync");

    assert_eq!(
        update_calls.lock().expect("update calls").as_slice(),
        &[(
            7,
            PullRequestUpdate {
                title: None,
                body: Some(
                    "Authored body\n\n<!-- jx-stack:start -->\n### Pull request stack\n\n◯ [#6 Root](https://github.com/example-owner/example-repo/pull/6)\n└ ◉ **[#7 Child](https://github.com/example-owner/example-repo/pull/7)** — this PR\n<!-- jx-stack:end -->"
                        .to_owned()
                ),
                base: None,
            }
        )]
    );
}

#[test]
fn sync_pull_requests_removes_stack_context_for_untracked_pr() {
    // Verifies: generated stack blocks are output-only and disappear when local stack state no longer includes the PR.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Child".to_owned(),
            body: Some(
                "Authored body\n\n<!-- jx-stack:start -->\nstale\n<!-- jx-stack:end -->".to_owned(),
            ),
            head_branch: "example-user/child".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let update_calls = github.update_calls.clone();
    let push = TrackedPushOutcome {
        pushed_refs: 1,
        bookmarks: vec![PushedBookmarkSummary {
            branch: "example-user/child".to_owned(),
            old_short_commit_id: Some("old".to_owned()),
            new_short_commit_id: Some("new".to_owned()),
            old_short_change_id: Some("changeoo".to_owned()),
            new_short_change_id: Some("changecc".to_owned()),
            old_description: None,
            new_description: None,
            pull_request_description: Some("Child\n\nAuthored body".to_owned()),
            pull_request_base: Some("main".to_owned()),
            new_workspace_visibility: WorkspaceVisibility::default(),
        }],
        pushed_commits: Vec::new(),
    };

    pollster::block_on(sync_pull_requests(
        &context(),
        &push,
        &StackMetadata::default(),
        &github,
    ))
    .expect("pull requests sync");

    assert_eq!(
        update_calls.lock().expect("update calls").as_slice(),
        &[(
            7,
            PullRequestUpdate {
                title: None,
                body: Some("Authored body".to_owned()),
                base: None,
            }
        )]
    );
}

#[test]
fn pull_request_description_without_stack_context_markers_hides_delimiters() {
    // Verifies: local renderers can show stack context without exposing sync-only HTML anchors.
    assert_eq!(
        pull_request_description_without_stack_context_markers(
            "Authored body\n\n<!-- jx-stack:start -->\n### Pull request stack\n\n◯ Root\n└ ◉ Child — this PR\n\n<!-- jx-stack:end -->\n\nFooter",
        ),
        "Authored body\n\n### Pull request stack\n\n◯ Root\n└ ◉ Child — this PR\n\nFooter"
    );
}

#[test]
fn sync_pull_requests_clears_body_for_title_only_descriptions() {
    // Verifies: Sync can clear stale GitHub body text when the local PR description has no body.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "Old title".to_owned(),
            body: Some("Old body".to_owned()),
            head_branch: "example-user/current".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let update_calls = github.update_calls.clone();
    let push = TrackedPushOutcome {
        pushed_refs: 1,
        bookmarks: vec![PushedBookmarkSummary {
            branch: "example-user/current".to_owned(),
            old_short_commit_id: Some("11112222".to_owned()),
            new_short_commit_id: Some("a1b2c3d4".to_owned()),
            old_short_change_id: Some("changeoo".to_owned()),
            new_short_change_id: Some("changecc".to_owned()),
            old_description: Some("Old title".to_owned()),
            new_description: Some("New title".to_owned()),
            pull_request_description: Some("New title".to_owned()),
            pull_request_base: Some("main".to_owned()),
            new_workspace_visibility: WorkspaceVisibility::default(),
        }],
        pushed_commits: Vec::new(),
    };

    let pull_requests = pollster::block_on(sync_pull_requests(
        &context(),
        &push,
        &StackMetadata::default(),
        &github,
    ))
    .expect("pull requests sync");

    assert_eq!(pull_requests[0].title, "New title");
    assert_eq!(pull_requests[0].body.as_deref(), Some(""));
    assert_eq!(
        update_calls.lock().expect("update calls").as_slice(),
        &[(
            7,
            PullRequestUpdate {
                title: Some("New title".to_owned()),
                body: Some(String::new()),
                base: None,
            }
        )]
    );
}

#[test]
fn sync_pull_requests_skips_title_only_update_when_github_body_is_absent() {
    // Verifies: GitHub's absent body and jx's empty body compare equal for stable repeated syncs.
    let github = FakeGitHub {
        open_pull_request: Some(PullRequestRecord {
            number: 7,
            title: "New title".to_owned(),
            body: None,
            head_branch: "example-user/current".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }),
        ..FakeGitHub::default()
    };
    let update_calls = github.update_calls.clone();
    let push = TrackedPushOutcome {
        pushed_refs: 1,
        bookmarks: vec![PushedBookmarkSummary {
            branch: "example-user/current".to_owned(),
            old_short_commit_id: Some("11112222".to_owned()),
            new_short_commit_id: Some("a1b2c3d4".to_owned()),
            old_short_change_id: Some("changeoo".to_owned()),
            new_short_change_id: Some("changecc".to_owned()),
            old_description: Some("New title".to_owned()),
            new_description: Some("New title".to_owned()),
            pull_request_description: Some("New title".to_owned()),
            pull_request_base: Some("main".to_owned()),
            new_workspace_visibility: WorkspaceVisibility::default(),
        }],
        pushed_commits: Vec::new(),
    };

    pollster::block_on(sync_pull_requests(
        &context(),
        &push,
        &StackMetadata::default(),
        &github,
    ))
    .expect("pull requests sync");

    assert!(update_calls.lock().expect("update calls").is_empty());
}

fn context() -> RepositoryContext {
    let origin_github = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "example-repo".to_owned(),
    };
    RepositoryContext {
        workspace_root: "/workspace".into(),
        repository_root: "/workspace".into(),
        origin: OriginRemote {
            name: ORIGIN_REMOTE_NAME,
            url: "https://github.com/example-owner/example-repo.git".to_owned(),
            github: origin_github.clone(),
        },
        github_remotes: vec![GitHubRemote {
            name: "origin".to_owned(),
            url: "https://github.com/example-owner/example-repo.git".to_owned(),
            github: origin_github,
        }],
        token_source: TokenSource::Environment("JX_GITHUB_TOKEN"),
        config: WorkflowConfig {
            paths: Vec::new(),
            layout: Default::default(),
            repo: RepoConfig::default(),
            diff: Default::default(),
            auth: Default::default(),
            shell: Default::default(),
            ui: Default::default(),
        },
    }
}

fn context_with_reviewers(reviewers: &[&str]) -> RepositoryContext {
    let mut context = context();
    context.config.repo.base = RepoPolicyConfig {
        reviewers: reviewers
            .iter()
            .map(|reviewer| ReviewerTarget::user(*reviewer))
            .collect(),
        ..RepoPolicyConfig::default()
    };
    context
}

fn context_with_path_reviewers(reviewers: &[&str]) -> RepositoryContext {
    let mut context = context();
    context.config.repo.base = RepoPolicyConfig {
        reviewer_rules: vec![ReviewerPathRule {
            paths: vec!["src/**".to_owned()],
            reviewers: reviewers
                .iter()
                .map(|reviewer| ReviewerTarget::user(*reviewer))
                .collect(),
        }],
        ..RepoPolicyConfig::default()
    };
    context
}

fn context_with_event_handlers(handlers: Vec<RepoEventHandlerConfig>) -> RepositoryContext {
    let mut context = context();
    context.config.repo.base = RepoPolicyConfig {
        event_handlers: handlers,
        ..RepoPolicyConfig::default()
    };
    context
}

fn add_label_handler<const N: usize>(
    id: &str,
    on: RepoEvent,
    when: PullRequestEventQuery,
    labels: [&str; N],
) -> RepoEventHandlerConfig {
    RepoEventHandlerConfig::Handler(RepoEventHandler {
        id: Some(id.to_owned()),
        on,
        when,
        run: RepoEventHandlerRun::AddLabels {
            labels: labels.into_iter().map(str::to_owned).collect(),
        },
    })
}

fn prepend_task_id_handler(id: &str, when: PullRequestEventQuery) -> RepoEventHandlerConfig {
    RepoEventHandlerConfig::Handler(RepoEventHandler {
        id: Some(id.to_owned()),
        on: RepoEvent::PullRequestPrepare,
        when,
        run: RepoEventHandlerRun::PrependTaskId,
    })
}

fn open_pull_request_handler(
    id: &str,
    on: RepoEvent,
    when: PullRequestEventQuery,
) -> RepoEventHandlerConfig {
    RepoEventHandlerConfig::Handler(RepoEventHandler {
        id: Some(id.to_owned()),
        on,
        when,
        run: RepoEventHandlerRun::OpenPullRequest,
    })
}

fn query<const N: usize>(terms: [PullRequestEventQueryTerm; N]) -> PullRequestEventQuery {
    PullRequestEventQuery {
        terms: terms.into_iter().collect(),
    }
}

fn draft() -> PullRequestEventQueryTerm {
    term(PullRequestEventPredicate::Draft)
}

fn has_reviewers() -> PullRequestEventQueryTerm {
    term(PullRequestEventPredicate::HasReviewers)
}

fn has_task() -> PullRequestEventQueryTerm {
    term(PullRequestEventPredicate::HasTask)
}

fn label(name: &str) -> PullRequestEventQueryTerm {
    term(PullRequestEventPredicate::Label(name.to_owned()))
}

fn not(mut term: PullRequestEventQueryTerm) -> PullRequestEventQueryTerm {
    term.negated = true;
    term
}

fn term(predicate: PullRequestEventPredicate) -> PullRequestEventQueryTerm {
    PullRequestEventQueryTerm {
        predicate,
        negated: false,
    }
}

fn bookmark_update() -> BookmarkUpdate {
    BookmarkUpdate {
        branch: "example-user/abc-123-02-zzzzzzzz".to_owned(),
        created: true,
    }
}

fn push_outcome() -> PushOutcome {
    PushOutcome {
        branch: "example-user/abc-123-02-zzzzzzzz".to_owned(),
        pushed_refs: 1,
        pushed_commits: Vec::new(),
    }
}

fn workspace_facts() -> WorkspaceFacts {
    WorkspaceFacts {
        workspace_root: "/workspace".into(),
        target_change: ChangeSummary {
            change_id: "zzzzzzzz".to_owned(),
            commit_id: "a1b2c3d4e5f6".to_owned(),
            short_commit_id: "a1b2c3d4".to_owned(),
            description: "example change".to_owned(),
            is_empty: false,
        },
        trunk: TrunkSummary {
            branch: "main".to_owned(),
            commit_id: "1111222233334444".to_owned(),
            short_commit_id: "11112222".to_owned(),
        },
        trunk_git_commit_sha: "1111222233334444".to_owned(),
        origin_branch: "main".to_owned(),
        local_bookmarks: Vec::new(),
        local_bookmarks_at_target: Vec::new(),
        nearest_ancestor_bookmark: Some("example-user/01-ancestor".to_owned()),
        changed_files: vec!["src/main.rs".to_owned()],
        change_lines: vec!["M src/main.rs".to_owned()],
        stack_index: 2,
    }
}

fn status_workspace_facts() -> StatusWorkspaceFacts {
    StatusWorkspaceFacts {
        remotes: vec![StatusRemoteFacts {
            remote: "origin".to_owned(),
            branch: "main".to_owned(),
            trunk_git_commit_sha: "1111222233334444".to_owned(),
            trunk_short_commit_id: "11112222".to_owned(),
            local_ahead_by: 2,
        }],
    }
}

#[derive(Debug, Clone)]
enum FakeCompareFailure {
    Authentication,
    MissingBranch,
    UnknownLocalSha,
}

impl FakeCompareFailure {
    fn to_error(&self, base: &str, head: &str) -> GitHubError {
        match self {
            Self::Authentication => GitHubError::AuthenticationFailed {
                operation: "compare commits",
                message: "bad credentials".to_owned(),
            },
            Self::MissingBranch => GitHubError::ComparisonTargetNotFound {
                base: base.to_owned(),
                head: head.to_owned(),
                message: "base branch was not found".to_owned(),
            },
            Self::UnknownLocalSha => GitHubError::ComparisonTargetNotFound {
                base: base.to_owned(),
                head: head.to_owned(),
                message: "local commit was not found".to_owned(),
            },
        }
    }
}

type CompareCalls = Arc<Mutex<Vec<(String, String)>>>;
type BranchHeadCalls = Arc<Mutex<Vec<String>>>;
type CreateCalls = Arc<Mutex<Vec<PullRequestCreate>>>;
type UpdateCalls = Arc<Mutex<Vec<(u64, PullRequestUpdate)>>>;
type ReadinessCalls = Arc<Mutex<Vec<u64>>>;
type SuggestedReviewerCalls = Arc<Mutex<Vec<u64>>>;
type LabelCalls = Arc<Mutex<Vec<(u64, Vec<String>)>>>;
type ReviewerCalls = Arc<Mutex<Vec<(u64, ReviewerSelection)>>>;

#[derive(Clone)]
struct FakeGitHub {
    user: AuthenticatedUser,
    access: RepositoryAccess,
    comparison: CommitComparison,
    comparisons: BTreeMap<(String, String), CommitComparison>,
    compare_failure: Option<FakeCompareFailure>,
    compare_calls: CompareCalls,
    branch_head_sha: String,
    branch_head_calls: BranchHeadCalls,
    repository_fork: Option<RepositoryFork>,
    open_pull_request: Option<PullRequestRecord>,
    pull_requests_by_number: BTreeMap<u64, PullRequestRecord>,
    pull_request_statuses: Vec<PullRequestStatusRecord>,
    create_calls: CreateCalls,
    update_calls: UpdateCalls,
    mark_ready_calls: ReadinessCalls,
    convert_draft_calls: ReadinessCalls,
    suggested_reviewers: Vec<String>,
    suggested_reviewer_calls: SuggestedReviewerCalls,
    label_calls: LabelCalls,
    label_result: LabelApplyResult,
    pull_request_labels: Vec<String>,
    reviewer_calls: ReviewerCalls,
    reviewer_result: ReviewerSyncResult,
}

impl FakeGitHub {
    fn readiness_pull_request(&self, number: u64, draft: bool) -> PullRequestRecord {
        let mut pull_request = self.open_pull_request.clone().unwrap_or(PullRequestRecord {
            number,
            title: "updated title".to_owned(),
            body: None,
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: "main".to_owned(),
            html_url: Some(format!(
                "https://github.com/example-owner/example-repo/pull/{number}"
            )),
            draft,
            merged: false,
            reviewers: ReviewerSelection::default(),
        });
        pull_request.number = number;
        pull_request.draft = draft;
        pull_request
    }
}

impl Default for FakeGitHub {
    fn default() -> Self {
        Self {
            user: AuthenticatedUser {
                login: "example-user".to_owned(),
            },
            access: RepositoryAccess {
                repository: GitHubRepository {
                    owner: "example-owner".to_owned(),
                    name: "example-repo".to_owned(),
                },
                default_branch: Some("main".to_owned()),
                can_read: true,
                can_push: true,
                can_admin: false,
            },
            comparison: CommitComparison {
                status: ComparisonStatus::Identical,
                ahead_by: 0,
                behind_by: 0,
            },
            comparisons: BTreeMap::new(),
            compare_failure: None,
            compare_calls: Arc::new(Mutex::new(Vec::new())),
            branch_head_sha: "1111222233334444".to_owned(),
            branch_head_calls: Arc::new(Mutex::new(Vec::new())),
            repository_fork: None,
            open_pull_request: None,
            pull_requests_by_number: BTreeMap::new(),
            pull_request_statuses: Vec::new(),
            create_calls: Arc::new(Mutex::new(Vec::new())),
            update_calls: Arc::new(Mutex::new(Vec::new())),
            mark_ready_calls: Arc::new(Mutex::new(Vec::new())),
            convert_draft_calls: Arc::new(Mutex::new(Vec::new())),
            suggested_reviewers: Vec::new(),
            suggested_reviewer_calls: Arc::new(Mutex::new(Vec::new())),
            label_calls: Arc::new(Mutex::new(Vec::new())),
            label_result: LabelApplyResult::default(),
            pull_request_labels: Vec::new(),
            reviewer_calls: Arc::new(Mutex::new(Vec::new())),
            reviewer_result: ReviewerSyncResult::default(),
        }
    }
}

#[async_trait::async_trait]
impl GitHubClient for FakeGitHub {
    async fn authenticated_user(&self) -> Result<AuthenticatedUser, GitHubError> {
        Ok(self.user.clone())
    }

    async fn repository_access(
        &self,
        _repository: &GitHubRepository,
    ) -> Result<RepositoryAccess, GitHubError> {
        Ok(self.access.clone())
    }

    async fn repository_fork(
        &self,
        _repository: &GitHubRepository,
    ) -> Result<Option<RepositoryFork>, GitHubError> {
        Ok(self.repository_fork.clone())
    }

    async fn create_repository(
        &self,
        repository: &GitHubRepository,
        private: bool,
    ) -> Result<crate::github::RepositoryCreation, GitHubError> {
        Ok(crate::github::RepositoryCreation {
            repository: repository.clone(),
            html_url: repository.https_url(),
            private,
        })
    }

    async fn branch_head_sha(
        &self,
        _repository: &GitHubRepository,
        branch: &str,
    ) -> Result<String, GitHubError> {
        self.branch_head_calls
            .lock()
            .expect("branch head calls")
            .push(branch.to_owned());
        Ok(self.branch_head_sha.clone())
    }

    async fn compare_commits(
        &self,
        _repository: &GitHubRepository,
        base: &str,
        head: &str,
    ) -> Result<CommitComparison, GitHubError> {
        self.compare_calls
            .lock()
            .expect("compare calls")
            .push((base.to_owned(), head.to_owned()));

        if let Some(failure) = &self.compare_failure {
            return Err(failure.to_error(base, head));
        }

        Ok(self
            .comparisons
            .get(&(base.to_owned(), head.to_owned()))
            .cloned()
            .unwrap_or_else(|| self.comparison.clone()))
    }

    async fn find_authored_open_pull_request_for_head(
        &self,
        _repository: &GitHubRepository,
        _head: &PullRequestHead,
        _author: &str,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        Ok(self.open_pull_request.clone())
    }

    async fn find_open_pull_request(
        &self,
        _repository: &GitHubRepository,
        _head: &PullRequestHead,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        Ok(self.open_pull_request.clone())
    }

    async fn find_pull_request_for_head(
        &self,
        _repository: &GitHubRepository,
        _head: &PullRequestHead,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        Ok(self.open_pull_request.clone())
    }

    async fn find_pull_request_by_number(
        &self,
        _repository: &GitHubRepository,
        number: u64,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        Ok(self
            .pull_requests_by_number
            .get(&number)
            .cloned()
            .or_else(|| {
                self.open_pull_request
                    .clone()
                    .filter(|pull_request| pull_request.number == number)
            }))
    }

    async fn pull_request_statuses(
        &self,
        _repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, GitHubError> {
        Ok(self
            .pull_request_statuses
            .iter()
            .filter(|status| numbers.contains(&status.number))
            .cloned()
            .collect())
    }

    async fn pull_request_suggested_reviewers(
        &self,
        _repository: &GitHubRepository,
        number: u64,
    ) -> Result<Vec<String>, GitHubError> {
        self.suggested_reviewer_calls
            .lock()
            .expect("suggested reviewer calls")
            .push(number);
        Ok(self.suggested_reviewers.clone())
    }

    async fn create_pull_request(
        &self,
        _repository: &GitHubRepository,
        request: PullRequestCreate,
    ) -> Result<PullRequestRecord, GitHubError> {
        self.create_calls
            .lock()
            .expect("create calls")
            .push(request.clone());

        Ok(PullRequestRecord {
            number: 42,
            title: request.title,
            body: request.body,
            head_branch: request.head.branch,
            base_branch: request.base,
            html_url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
            draft: request.draft,
            merged: false,
            reviewers: ReviewerSelection::default(),
        })
    }

    async fn update_pull_request(
        &self,
        _repository: &GitHubRepository,
        number: u64,
        request: PullRequestUpdate,
    ) -> Result<PullRequestRecord, GitHubError> {
        self.update_calls
            .lock()
            .expect("update calls")
            .push((number, request.clone()));

        let existing = self.open_pull_request.clone();
        Ok(PullRequestRecord {
            number,
            title: request.title.unwrap_or_else(|| {
                existing
                    .as_ref()
                    .map_or_else(|| "updated title".to_owned(), |pr| pr.title.clone())
            }),
            body: request
                .body
                .or_else(|| existing.as_ref().and_then(|pr| pr.body.clone())),
            head_branch: existing.as_ref().map_or_else(
                || "example-user/02-a1b2c3d4".to_owned(),
                |pr| pr.head_branch.clone(),
            ),
            base_branch: request.base.unwrap_or_else(|| {
                existing
                    .as_ref()
                    .map_or_else(|| "main".to_owned(), |pr| pr.base_branch.clone())
            }),
            html_url: Some(format!(
                "https://github.com/example-owner/example-repo/pull/{number}"
            )),
            draft: existing.is_some_and(|pr| pr.draft),
            merged: false,
            reviewers: ReviewerSelection::default(),
        })
    }

    async fn mark_pull_request_ready(
        &self,
        _repository: &GitHubRepository,
        number: u64,
    ) -> Result<PullRequestRecord, GitHubError> {
        self.mark_ready_calls
            .lock()
            .expect("mark ready calls")
            .push(number);
        Ok(self.readiness_pull_request(number, false))
    }

    async fn convert_pull_request_to_draft(
        &self,
        _repository: &GitHubRepository,
        number: u64,
    ) -> Result<PullRequestRecord, GitHubError> {
        self.convert_draft_calls
            .lock()
            .expect("convert draft calls")
            .push(number);
        Ok(self.readiness_pull_request(number, true))
    }

    async fn pull_request_labels(
        &self,
        _repository: &GitHubRepository,
        _number: u64,
    ) -> Result<Vec<String>, GitHubError> {
        Ok(self.pull_request_labels.clone())
    }

    async fn add_labels(
        &self,
        _repository: &GitHubRepository,
        number: u64,
        labels: Vec<String>,
    ) -> Result<LabelApplyResult, GitHubError> {
        self.label_calls
            .lock()
            .expect("label calls")
            .push((number, labels.clone()));
        if self.label_result.labels.is_empty() {
            Ok(LabelApplyResult { labels })
        } else {
            Ok(self.label_result.clone())
        }
    }

    async fn sync_reviewers(
        &self,
        _repository: &GitHubRepository,
        number: u64,
        desired: ReviewerSelection,
    ) -> Result<ReviewerSyncResult, GitHubError> {
        self.reviewer_calls
            .lock()
            .expect("reviewer calls")
            .push((number, desired));
        Ok(self.reviewer_result.clone())
    }
}
