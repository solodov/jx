use super::*;

fn origin_config() -> &'static str {
    r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#
}

#[test]
fn fork_sync_detects_source_rebases_and_pushes() {
    // Verifies: fork sync uses GitHub fork metadata, prepares upstream, fetches both remotes, rebases, then pushes origin.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "fork", "sync"], &environment, &services)
        .expect("fork sync succeeds");

    assert_eq!(
        services.ensured_git_remotes.borrow().as_slice(),
        [(
            "upstream".to_owned(),
            "git@github.com:source-owner/example-repo.git".to_owned(),
            false,
            true,
        )]
    );
    assert_eq!(
        services.fetch_remote_calls.borrow().as_slice(),
        ["upstream".to_owned(), "origin".to_owned()]
    );
    assert_eq!(
        services.fork_sync_branch_plan_requests.borrow().as_slice(),
        [("main".to_owned(), "upstream".to_owned(), "main".to_owned(),)]
    );
    assert_eq!(
        services.push_bookmark_calls.borrow().as_slice(),
        ["main".to_owned()]
    );
    assert_eq!(
        result.stdout,
        "Fork sync: example-owner/example-repo/main <- source-owner/example-repo/main\n\
Upstream remote: upstream -> git@github.com:source-owner/example-repo.git\n\
Rebased 2 commits from changeaa: main onto main@upstream\n\
Pushed main to origin\n"
    );
}

#[test]
fn fork_sync_no_push_updates_locally_only() {
    // Verifies: --no-push keeps the source rebase local and avoids the origin push.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "fork", "sync", "--no-push"], &environment, &services)
            .expect("fork sync succeeds");

    assert!(services.push_bookmark_calls.borrow().is_empty());
    assert!(result.stdout.ends_with("Push disabled (--no-push)\n"));
}

#[test]
fn fork_sync_skips_push_after_rebase_conflicts() {
    // Verifies: conflicted local rebases are reported and never pushed to the fork.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut services = FakeServices::default();
    services.fork_sync_outcome.rebased_commits = vec![RebasedCommitSummary {
        short_change_id: "changeaa".to_owned(),
        old_short_commit_id: "11112222".to_owned(),
        new_short_commit_id: "aaaabbbb".to_owned(),
        description: "example change".to_owned(),
        has_conflict: true,
        is_empty: false,
        workspace_visibility: current_workspace_visibility(),
    }];

    let result = run_with_args_and_services(["jx", "fork", "sync"], &environment, &services)
        .expect("fork sync reports conflicts as command result");

    assert_eq!(result.exit_code, 1);
    assert!(services.push_bookmark_calls.borrow().is_empty());
    assert!(result
        .stdout
        .contains("Rebased locally:\n  changeaa  example change [conflict]\n"));
    assert!(result
        .stdout
        .ends_with("Push skipped: rebased commits have conflicts\n"));
}

#[test]
fn fork_sync_uses_upstream_url_without_fork_detection() {
    // Verifies: explicit upstream URLs support non-fork local repos while preserving remote setup.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        repository_fork: None,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        [
            "jx",
            "fork",
            "sync",
            "--upstream-url",
            "https://github.com/source-owner/example-repo.git",
            "--branch",
            "trunk",
            "--upstream",
            "source",
            "--fix-remotes",
        ],
        &environment,
        &services,
    )
    .expect("fork sync succeeds with explicit upstream URL");

    assert_eq!(services.repository_fork_calls.get(), 0);
    assert_eq!(
        services.ensured_git_remotes.borrow().as_slice(),
        [(
            "source".to_owned(),
            "https://github.com/source-owner/example-repo.git".to_owned(),
            true,
            true,
        )]
    );
    assert!(result.stdout.starts_with(
        "Fork sync: example-owner/example-repo/trunk <- source-owner/example-repo/trunk\n"
    ));
}

#[test]
fn fork_sync_requires_fork_source_without_upstream_url() {
    // Verifies: fork sync does not guess a source for non-fork origins.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        repository_fork: None,
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "fork", "sync"], &environment, &services)
        .expect_err("missing fork source fails");

    assert_eq!(
        error.to_string(),
        "Repository `example-owner/example-repo` is not a GitHub fork; pass --upstream-url to sync against an explicit source"
    );
}

#[test]
fn fork_sync_can_be_cancelled_before_local_mutation() {
    // Verifies: declining the confirmation stops before applying the local branch move or push.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();
    let fork_sync_confirmer = FixedForkSyncConfirmer { confirmed: false };
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        fork_sync_confirmer: &fork_sync_confirmer,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };

    let result = run_with_args_and_progress(
        ["jx", "fork", "sync"],
        &environment,
        &services,
        &NoProgress,
        prompts,
        OutputMode::plain(),
    )
    .expect("fork sync cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
    assert!(services.applied_fork_sync_plans.borrow().is_empty());
    assert!(services.push_bookmark_calls.borrow().is_empty());
}
