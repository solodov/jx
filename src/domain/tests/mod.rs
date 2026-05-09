use std::sync::{Arc, Mutex};

use crate::{
    github::{
        AuthenticatedUser, PullRequestCreate, PullRequestHead, PullRequestRecord,
        PullRequestUpdate, ReviewerSelection, ReviewerSyncResult,
    },
    jj::{ChangeSummary, StatusRemoteFacts, StatusWorkspaceFacts, TrunkSummary},
    repository::{
        GitHubRemote, GitHubRepository, OriginRemote, RepoConfig, RepoPolicyConfig, TokenSource,
        WorkflowConfig, ORIGIN_REMOTE_NAME,
    },
};

use super::*;

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
    assert_eq!(report.bookmark.branch, "example-user/02-a1b2c3d4");
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
    assert_eq!(report.bookmark.branch, "example-user/ABC-123-02-a1b2c3d4");
    assert_eq!(report.bookmark.action, BookmarkAction::Create);
}

#[test]
fn bookmark_planner_reuses_exact_existing_selected_bookmark() {
    // Verifies: Bookmark planner reuses an exact existing bookmark on the selected change.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["example-user/02-a1b2c3d4".to_owned()];
    workspace.local_bookmarks_at_target = workspace.local_bookmarks.clone();

    let plan = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: None,
        workspace: &workspace,
    })
    .expect("existing bookmark is reused");

    assert_eq!(plan.branch, "example-user/02-a1b2c3d4");
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
fn bookmark_planner_formats_multi_digit_stack_indices() {
    // Verifies: Bookmark planner formats multi-digit stack indices.
    let mut workspace = workspace_facts();
    workspace.stack_index = 12;
    workspace.target_change.short_commit_id = "deadbeef".to_owned();

    let default = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: None,
        workspace: &workspace,
    })
    .expect("default bookmark plans");
    let task = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: Some("ABC-123"),
        workspace: &workspace,
    })
    .expect("task bookmark plans");

    assert_eq!(default.branch, "example-user/12-deadbeef");
    assert_eq!(task.branch, "example-user/ABC-123-12-deadbeef");
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
fn bookmark_planner_rejects_task_specific_duplicate_on_another_change() {
    // Verifies: Bookmark planner rejects a task-specific duplicate on another change.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["example-user/ABC-123-01-deadbeef".to_owned()];

    let error = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: Some("ABC-123"),
        workspace: &workspace,
    })
    .expect_err("same task already has a bookmark");

    assert!(matches!(
        error,
        WorkflowError::BookmarkExistsOnDifferentChange { branch }
            if branch == "example-user/ABC-123-01-deadbeef"
    ));
}

#[test]
fn bookmark_planner_rejects_conflicting_selected_bookmark_when_task_id_is_requested() {
    // Verifies: Bookmark planner rejects a conflicting selected-change bookmark when a task ID
    // is requested.
    let mut workspace = workspace_facts();
    workspace.local_bookmarks = vec!["example-user/02-a1b2c3d4".to_owned()];
    workspace.local_bookmarks_at_target = workspace.local_bookmarks.clone();

    let error = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: Some("ABC-123"),
        workspace: &workspace,
    })
    .expect_err("selected bookmark would become duplicate PR head");

    assert!(matches!(
        error,
        WorkflowError::ConflictingSelectedBookmark { existing, requested }
            if existing == "example-user/02-a1b2c3d4"
                && requested == "example-user/ABC-123-02-a1b2c3d4"
    ));
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
    workspace.local_bookmarks = vec!["example-user/02-a1b2c3d4".to_owned()];

    let error = plan_bookmark(BookmarkPlanRequest {
        github_login: "example-user",
        task_id: None,
        workspace: &workspace,
    })
    .expect_err("generated bookmark already exists elsewhere");

    assert!(matches!(
        error,
        WorkflowError::BookmarkExistsOnDifferentChange { branch }
            if branch == "example-user/02-a1b2c3d4"
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
    ticket_workspace.target_change.description = "fd-12345 make checkout faster".to_owned();

    let ticket = push_plan(&context(), ticket_workspace, None).expect("ticket push plans");

    assert_eq!(generic.bookmark.branch, "push-zzzzzzzz");
    assert_eq!(generic.bookmark.action, BookmarkAction::Create);
    assert_eq!(ticket.bookmark.branch, "ps/FD-12345-02-a1b2c3d4");
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
fn pull_request_plan_derives_metadata_bookmark_base_and_reviewers() {
    // Verifies: Pull request plan derives metadata bookmark base and reviewers.
    let github = FakeGitHub::default();
    let mut workspace = workspace_facts();
    workspace.target_change.description = "Example title\n\nDetailed body".to_owned();

    let plan = pollster::block_on(pull_request_plan(
        &context_with_reviewers(&["example-reviewer", "second-reviewer"]),
        workspace,
        &github,
        Some("ABC-123".to_owned()),
        vec!["bug".to_owned(), "help wanted".to_owned()],
        true,
    ))
    .expect("PR plan is derived");

    assert_eq!(plan.title, "Example title");
    assert_eq!(plan.body, "Example title\n\nDetailed body");
    assert_eq!(plan.target_commit_id, "a1b2c3d4e5f6");
    assert_eq!(plan.changed_files, ["src/main.rs".to_owned()]);
    assert!(plan.draft);
    assert_eq!(plan.base, "example-user/01-ancestor");
    assert_eq!(
        plan.head.label(),
        "example-owner:example-user/ABC-123-02-a1b2c3d4"
    );
    assert_eq!(plan.bookmark.branch, "example-user/ABC-123-02-a1b2c3d4");
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
fn pull_request_plan_uses_origin_branch_when_no_ancestor_bookmark_exists() {
    // Verifies: Pull request plan uses origin branch when no ancestor bookmark exists.
    let github = FakeGitHub::default();
    let mut workspace = workspace_facts();
    workspace.nearest_ancestor_bookmark = None;

    let plan = pollster::block_on(pull_request_plan(
        &context(),
        workspace,
        &github,
        None,
        Vec::new(),
        false,
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
        false,
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
        false,
    ))
    .expect_err("description is required");

    assert!(matches!(
        missing_description,
        WorkflowError::MissingPullRequestDescription
    ));
}

#[test]
fn publish_pull_request_creates_pr_and_syncs_configured_reviewers() {
    // Verifies: Publish pull request creates PR and syncs configured reviewers.
    let github = FakeGitHub {
        reviewer_result: ReviewerSyncResult {
            requested_users: vec!["example-reviewer".to_owned()],
            ..ReviewerSyncResult::default()
        },
        ..FakeGitHub::default()
    };
    let create_calls = github.create_calls.clone();
    let reviewer_calls = github.reviewer_calls.clone();
    let context = context_with_reviewers(&["example-reviewer"]);
    let plan = pollster::block_on(pull_request_plan(
        &context,
        workspace_facts(),
        &github,
        Some("ABC-123".to_owned()),
        Vec::new(),
        true,
    ))
    .expect("PR plan is derived");

    let report = pollster::block_on(publish_pull_request(
        &context,
        plan,
        bookmark_update(),
        push_outcome(),
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
        false,
    ))
    .expect("PR plan is derived");

    let create_report = pollster::block_on(publish_pull_request(
        &context,
        create_plan,
        bookmark_update(),
        push_outcome(),
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
        false,
    ))
    .expect("PR plan is derived");

    let update_report = pollster::block_on(publish_pull_request(
        &context,
        update_plan,
        bookmark_update(),
        push_outcome(),
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
        false,
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

fn context() -> RepositoryContext {
    let origin_github = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "example-repo".to_owned(),
    };
    RepositoryContext {
        workspace_root: "/workspace".into(),
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

fn bookmark_update() -> BookmarkUpdate {
    BookmarkUpdate {
        branch: "example-user/ABC-123-02-a1b2c3d4".to_owned(),
        created: true,
    }
}

fn push_outcome() -> PushOutcome {
    PushOutcome {
        branch: "example-user/ABC-123-02-a1b2c3d4".to_owned(),
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
type CreateCalls = Arc<Mutex<Vec<PullRequestCreate>>>;
type UpdateCalls = Arc<Mutex<Vec<(u64, PullRequestUpdate)>>>;
type LabelCalls = Arc<Mutex<Vec<(u64, Vec<String>)>>>;
type ReviewerCalls = Arc<Mutex<Vec<(u64, ReviewerSelection)>>>;

#[derive(Clone)]
struct FakeGitHub {
    user: AuthenticatedUser,
    access: RepositoryAccess,
    comparison: CommitComparison,
    compare_failure: Option<FakeCompareFailure>,
    compare_calls: CompareCalls,
    open_pull_request: Option<PullRequestRecord>,
    create_calls: CreateCalls,
    update_calls: UpdateCalls,
    label_calls: LabelCalls,
    label_result: LabelApplyResult,
    reviewer_calls: ReviewerCalls,
    reviewer_result: ReviewerSyncResult,
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
            compare_failure: None,
            compare_calls: Arc::new(Mutex::new(Vec::new())),
            open_pull_request: None,
            create_calls: Arc::new(Mutex::new(Vec::new())),
            update_calls: Arc::new(Mutex::new(Vec::new())),
            label_calls: Arc::new(Mutex::new(Vec::new())),
            label_result: LabelApplyResult::default(),
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

        Ok(self.comparison.clone())
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

        Ok(PullRequestRecord {
            number,
            title: request.title.unwrap_or_else(|| "updated title".to_owned()),
            body: request.body,
            head_branch: "example-user/02-a1b2c3d4".to_owned(),
            base_branch: request.base.unwrap_or_else(|| "main".to_owned()),
            html_url: Some(format!(
                "https://github.com/example-owner/example-repo/pull/{number}"
            )),
            draft: false,
        })
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
            .push((number, labels));
        Ok(self.label_result.clone())
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
