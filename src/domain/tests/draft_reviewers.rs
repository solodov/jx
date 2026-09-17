use super::*;

#[test]
fn draft_plans_never_default_to_configured_or_existing_review_requests() {
    for existing in [None, Some(existing_reviewed_draft())] {
        let github = FakeGitHub {
            open_pull_request: existing,
            ..FakeGitHub::default()
        };
        let plan = plan_draft_with_configured_reviewers(&github);

        assert!(plan.draft);
        assert!(plan.reviewers.is_empty());
        assert!(plan
            .reviewer_candidates
            .iter()
            .any(|candidate| { candidate.target == ReviewerTarget::user("configured-reviewer") }));
    }
}

#[test]
fn new_drafts_are_created_without_requesting_configured_reviewers() {
    let github = FakeGitHub::default();
    let plan = plan_draft_with_configured_reviewers(&github);
    let report = pollster::block_on(publish_pull_request(
        &context(),
        plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &github,
    ))
    .expect("new draft publishes");

    assert_eq!(report.action, PullRequestAction::Created);
    assert!(report.pull_request.draft);
    assert!(report.reviewers.is_none());
    assert!(github
        .reviewer_calls
        .lock()
        .expect("reviewer calls")
        .is_empty());
}

#[test]
fn normal_and_metadata_only_draft_publishing_skip_reviewer_sync() {
    for metadata_only in [false, true] {
        let existing = existing_reviewed_draft();
        let github = FakeGitHub {
            open_pull_request: Some(existing.clone()),
            ..FakeGitHub::default()
        };
        let plan = plan_draft_with_configured_reviewers(&github);
        assert_eq!(plan.effective_reviewers(), &existing.reviewers);
        let report = if metadata_only {
            pollster::block_on(publish_pull_request_metadata_only(
                &context(),
                plan,
                bookmark_update(),
                push_outcome(),
                &github,
            ))
        } else {
            pollster::block_on(publish_pull_request(
                &context(),
                plan,
                bookmark_update(),
                push_outcome(),
                PullRequestPublishOptions::default(),
                &github,
            ))
        }
        .expect("draft publishes");

        assert!(report.reviewers.is_none());
        assert!(github
            .reviewer_calls
            .lock()
            .expect("reviewer calls")
            .is_empty());
    }
}

#[test]
fn explicit_draft_requests_are_not_blocked_by_the_publisher() {
    for metadata_only in [false, true] {
        let github = FakeGitHub {
            open_pull_request: Some(existing_reviewed_draft()),
            ..FakeGitHub::default()
        };
        let mut plan = plan_draft_with_configured_reviewers(&github);
        let selected = ReviewerSelection::new(["alice", "bob"], ["platform"]);
        plan.reviewers = selected.clone();
        let report = if metadata_only {
            pollster::block_on(publish_pull_request_metadata_only(
                &context(),
                plan,
                bookmark_update(),
                push_outcome(),
                &github,
            ))
        } else {
            pollster::block_on(publish_pull_request(
                &context(),
                plan,
                bookmark_update(),
                push_outcome(),
                PullRequestPublishOptions::default(),
                &github,
            ))
        }
        .expect("explicit draft review publishes");

        assert!(report.reviewers.is_some());
        assert_eq!(
            github
                .reviewer_calls
                .lock()
                .expect("reviewer calls")
                .as_slice(),
            &[(7, selected)],
        );
    }
}

#[test]
fn reviewer_event_predicates_include_preserved_draft_requests() {
    let github = FakeGitHub {
        open_pull_request: Some(existing_reviewed_draft()),
        ..FakeGitHub::default()
    };
    let plan = plan_draft_with_configured_reviewers(&github);
    let context = context_with_event_handlers(vec![add_label_handler(
        "draft-feedback",
        RepoEvent::PullRequestUpdated,
        query([draft(), has_reviewers()]),
        ["feedback"],
    )]);
    pollster::block_on(publish_pull_request(
        &context,
        plan,
        bookmark_update(),
        push_outcome(),
        PullRequestPublishOptions::default(),
        &github,
    ))
    .expect("draft publishes with accurate reviewer predicates");

    assert_eq!(
        github.label_calls.lock().expect("label calls").as_slice(),
        &[(7, vec!["feedback".to_owned()])],
    );
    assert!(github
        .reviewer_calls
        .lock()
        .expect("reviewer calls")
        .is_empty());
}

fn plan_draft_with_configured_reviewers(github: &FakeGitHub) -> PullRequestPlan {
    pollster::block_on(pull_request_plan(
        &context_with_path_reviewers(&["configured-reviewer"]),
        workspace_facts(),
        github,
        "example-user",
        None,
        Vec::new(),
        PullRequestReadiness::Draft,
    ))
    .expect("draft plan builds")
}

fn existing_reviewed_draft() -> PullRequestRecord {
    PullRequestRecord {
        number: 7,
        title: "Existing draft".to_owned(),
        body: None,
        head_branch: "example-user/02-a1b2c3d4".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
        draft: true,
        merged: false,
        reviewers: ReviewerSelection::new(["bob"], ["platform"]),
    }
}
