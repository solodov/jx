use super::*;

#[test]
fn remote_status_loads_context_and_renders_github_freshness() {
    // Verifies: Remote status loads context and renders GitHub freshness.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), pull needed: GitHub has 3 new commits\n"
        );
}

#[test]
fn rs_alias_runs_remote_status() {
    // Verifies: The short alias keeps remote-status distinct from jj status.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "rs"], &environment, &services).expect("rs succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), pull needed: GitHub has 3 new commits\n"
        );
}

#[test]
fn remote_status_format_json_renders_current_repository_report() {
    // Verifies: JSON remote status keeps the same top-level shape for single-repo output.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "remote-status", "--format", "json"],
        &environment,
        &services,
    )
    .expect("remote-status json succeeds");
    let value: serde_json::Value = serde_json::from_str(&result.stdout).expect("valid json");

    assert_eq!(value["command"], "remote-status");
    assert_eq!(value["version"], 1);
    assert_eq!(
        value["repositories"][0]["root"],
        workspace.path().display().to_string()
    );
    assert_eq!(
        value["repositories"][0]["repository"],
        "example-owner/example-repo"
    );
    assert_eq!(
        value["repositories"][0]["url"],
        "https://github.com/example-owner/example-repo"
    );
    assert_eq!(value["repositories"][0]["remotes"][0]["name"], "origin");
    assert_eq!(
        value["repositories"][0]["remotes"][0]["state"],
        "github-ahead"
    );
    assert_eq!(value["repositories"][0]["remotes"][0]["githubAheadBy"], 3);
}

#[test]
fn remote_status_format_json_renders_global_repository_keys() {
    // Verifies: Global JSON output includes layout keys and absolute roots for org table consumers.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let alpha = workspace.create_jj_workspace("projects/alpha");
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/alpha.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "remote-status", "--all", "--format", "json"],
        &environment,
        &services,
    )
    .expect("global remote-status json succeeds");
    let value: serde_json::Value = serde_json::from_str(&result.stdout).expect("valid json");

    assert_eq!(value["repositories"].as_array().expect("repos").len(), 1);
    assert_eq!(value["repositories"][0]["key"], "alpha");
    assert_eq!(
        value["repositories"][0]["root"],
        alpha.display().to_string()
    );
    assert_eq!(
        value["repositories"][0]["repository"],
        "example-owner/alpha"
    );
}

#[test]
fn remote_status_jobs_rejects_zero_parallelism() {
    // Verifies: Global remote-status keeps a positive batch size so progress cannot stall.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let error = run_with_args_and_services(
        ["jx", "remote-status", "--all", "--jobs", "0"],
        &environment,
        &services,
    )
    .expect_err("zero jobs is rejected");

    assert!(matches!(error, CommandError::Usage(_)));
}

#[test]
fn remote_status_shows_local_commits_as_remote_behind() {
    // Verifies: A synchronized remote still reports unpublished local workspace commits.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        status: StatusReport {
            remotes: vec![domain::RemoteStatusReport {
                name: "origin".to_owned(),
                url: "https://github.com/example-owner/example-repo.git".to_owned(),
                github_url: "https://github.com/example-owner/example-repo".to_owned(),
                branch: "main".to_owned(),
                local_trunk_sha: "1111222233334444".to_owned(),
                local_trunk_short_sha: "11112222".to_owned(),
                local_ahead_by: 2,
                comparison: StatusComparison {
                    state: StatusState::UpToDate,
                    github_ahead_by: 0,
                    github_behind_by: 0,
                },
            }],
            fork: None,
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), push needed: local has 2 unpublished commits\n"
        );
}

#[test]
fn remote_status_renders_one_line_per_github_remote() {
    // Verifies: Remote status output stays remote-oriented when multiple remotes are configured.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
[remote "upstream"]
    url = https://github.com/upstream-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        status: StatusReport {
            remotes: vec![
                domain::RemoteStatusReport {
                    name: "origin".to_owned(),
                    url: "ssh://git@github.com/example-owner/example-repo.git".to_owned(),
                    github_url: "https://github.com/example-owner/example-repo".to_owned(),
                    branch: "main".to_owned(),
                    local_trunk_sha: "1111222233334444".to_owned(),
                    local_trunk_short_sha: "11112222".to_owned(),
                    local_ahead_by: 2,
                    comparison: StatusComparison {
                        state: StatusState::GithubAhead,
                        github_ahead_by: 1,
                        github_behind_by: 0,
                    },
                },
                domain::RemoteStatusReport {
                    name: "upstream".to_owned(),
                    url: "https://github.com/upstream-owner/example-repo.git".to_owned(),
                    github_url: "https://github.com/upstream-owner/example-repo".to_owned(),
                    branch: "main".to_owned(),
                    local_trunk_sha: "aaaabbbbccccdddd".to_owned(),
                    local_trunk_short_sha: "aaaabbbb".to_owned(),
                    local_ahead_by: 1,
                    comparison: StatusComparison {
                        state: StatusState::LocalAhead,
                        github_ahead_by: 0,
                        github_behind_by: 2,
                    },
                },
            ],
            fork: None,
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
            result.stdout,
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), diverged: pull 1 commit, push 2 commits\nremote: upstream (\x1b]8;;https://github.com/upstream-owner/example-repo/tree/main\x1b\\https://github.com/upstream-owner/example-repo.git\x1b]8;;\x1b\\), push needed: local has 3 unpublished commits\n"
        );
}

#[test]
fn remote_status_renders_fork_freshness_after_remotes() {
    // Verifies: Single-repo remote status shows the fork's source relationship explicitly.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut status = FakeServices::default().status;
    status.fork = Some(fork_status(ForkStatusState::SourceAhead, 7, 0));
    let services = FakeServices {
        status,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect("remote-status succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "remote: origin (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\https://github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), pull needed: GitHub has 3 new commits\nfork: {} vs source {}, source has 7 new commits\n",
            osc8_link(
                "https://github.com/example-owner/example-repo/tree/main",
                "example-owner/example-repo/main"
            ),
            osc8_link(
                "https://github.com/source-owner/example-repo/tree/main",
                "source-owner/example-repo/main"
            )
        )
    );
}

#[test]
fn remote_status_all_groups_fork_freshness() {
    // Verifies: Global remote status groups fork/source deltas separately from local remotes.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let forked = workspace.create_jj_workspace("projects/forked");
    TestWorkspace::write_git_config_at(
        &forked,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/forked.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        status: StatusReport {
            remotes: vec![domain::RemoteStatusReport {
                name: "origin".to_owned(),
                url: "https://github.com/example-owner/example-repo.git".to_owned(),
                github_url: "https://github.com/example-owner/example-repo".to_owned(),
                branch: "main".to_owned(),
                local_trunk_sha: "1111222233334444".to_owned(),
                local_trunk_short_sha: "11112222".to_owned(),
                local_ahead_by: 0,
                comparison: StatusComparison {
                    state: StatusState::UpToDate,
                    github_ahead_by: 0,
                    github_behind_by: 0,
                },
            }],
            fork: Some(fork_status(ForkStatusState::SourceAhead, 7, 0)),
        },
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "remote-status", "--all"], &environment, &services)
            .expect("global remote-status succeeds");

    assert_eq!(
        result.stdout,
        "Remote status: 1 repository checked, 1 needs attention\n\nFork behind source:\n  ~/projects/forked  source-owner/example-repo/main has 7 new commits\n"
    );
}

#[test]
fn remote_status_global_renderer_sorts_entries_by_directory() {
    // Verifies: All-project remote-status output follows stable filesystem order when checks complete out of order.
    let entries = vec![
        GlobalStatusEntry {
            key: Some("alpha".to_owned()),
            root: PathBuf::from("/workspace/src/alpha"),
            display_root: "alpha".to_owned(),
            repository: None,
            result: Err("alpha failed".to_owned()),
        },
        GlobalStatusEntry {
            key: Some("beta".to_owned()),
            root: PathBuf::from("/workspace/projects/beta"),
            display_root: "beta".to_owned(),
            repository: None,
            result: Err("beta failed".to_owned()),
        },
    ];

    let output = render_global_status(&entries, entries.len(), Path::new("/workspace"), false)
        .expect("global status renders");

    assert_eq!(
        output,
        "Remote status: 2 repositories checked, 2 need attention\n\nSetup needed:\n  beta   beta failed\n  alpha  alpha failed\n"
    );
}

#[test]
fn remote_status_all_groups_repositories_by_action_needed() {
    // Verifies: Global remote-status scans configured repos with a custom concurrency limit and groups pull work by path.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let alpha = workspace.create_jj_workspace("projects/alpha");
    let beta = workspace.create_jj_workspace("projects/beta");
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/alpha.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &beta,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/beta.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let progress = RecordingProgress::default();
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };
    let result = run_with_args_and_progress(
        ["jx", "remote-status", "--all", "--jobs", "1"],
        &environment,
        &services,
        &progress,
        prompts,
        OutputMode::plain(),
    )
    .expect("global remote-status succeeds");

    assert_eq!(
        progress.messages(),
        [
            "  0% Checking remote status…",
            " 50% Checking remote status…",
            "100% Checking remote status…"
        ]
    );
    assert!(progress.finished.get());

    assert_eq!(
        result.stdout,
        "Remote status: 2 repositories checked, 2 need attention\n\nPull needed: GitHub has new commits\n  ~/projects/alpha  3 commits to pull\n  ~/projects/beta   3 commits to pull\n"
    );
}

#[test]
fn remote_status_accepts_specific_repository_argument() {
    // Verifies: A positional project key runs remote-status from that configured repository only.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let alpha = workspace.create_jj_workspace("projects/alpha");
    let beta = workspace.create_jj_workspace("projects/beta");
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/alpha.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &beta,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/beta.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "rs", "beta"], &environment, &services)
        .expect("specific remote-status succeeds");

    assert_eq!(
        result.stdout,
        "remote: origin (\x1b]8;;https://github.com/example-owner/beta/tree/main\x1b\\ssh://git@github.com/example-owner/beta.git\x1b]8;;\x1b\\), pull needed: GitHub has 3 new commits\n"
    );
}

#[test]
fn remote_status_repo_filter_accepts_globs() {
    // Verifies: --repo selects configured repository keys with glob matching.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let alpha = workspace.create_jj_workspace("projects/api-alpha");
    let beta = workspace.create_jj_workspace("projects/web-beta");
    TestWorkspace::write_git_config_at(
        &alpha,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api-alpha.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &beta,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/web-beta.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "remote-status", "--repo", "api-*"],
        &environment,
        &services,
    )
    .expect("filtered global remote-status succeeds");

    assert_eq!(
        result.stdout,
        "Remote status: 1 repository checked, 1 needs attention\n\nPull needed: GitHub has new commits\n  ~/projects/api-alpha  3 commits to pull\n"
    );
}

#[test]
fn remote_status_changed_omits_up_to_date_repositories() {
    // Verifies: --changed keeps global status focused on repos with local or remote deltas.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let changed = workspace.create_jj_workspace("projects/changed");
    let clean = workspace.create_jj_workspace("projects/clean");
    TestWorkspace::write_git_config_at(
        &changed,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/changed.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &clean,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/clean.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        clean_status_repos: vec!["clean".to_owned()],
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "remote-status", "-a", "-c"], &environment, &services)
            .expect("changed global remote-status succeeds");

    assert_eq!(
        result.stdout,
        "Remote status: 2 repositories checked, 1 needs attention\n\nPull needed: GitHub has new commits\n  ~/projects/changed  3 commits to pull\n\nSynced: 1 repository\n"
    );
}

#[test]
fn remote_status_all_renders_repository_errors_as_rows() {
    // Verifies: One misconfigured repo does not hide status for the rest of the layout.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
    );
    let ok = workspace.create_jj_workspace("projects/ok");
    let _missing = workspace.create_jj_workspace("projects/missing-origin");
    TestWorkspace::write_git_config_at(
        &ok,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/ok.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        status_uses_context_remotes: true,
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "remote-status", "--all"], &environment, &services)
            .expect("global remote-status keeps going");

    assert_eq!(
        result.stdout,
        "Remote status: 2 repositories checked, 2 need attention\n\nPull needed: GitHub has new commits\n  ~/projects/ok              3 commits to pull\n\nSetup needed:\n  ~/projects/missing-origin  The fixed `origin` remote is missing. Add an `origin` GitHub remote before running `jx`.\n"
    );
}

#[test]
fn remote_status_rejects_missing_origin_with_actionable_error() {
    // Verifies: Remote status rejects missing origin with actionable error.
    let workspace = TestWorkspace::new();
    workspace.write_git_config("");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let error = run_with_args_and_services(["jx", "remote-status"], &environment, &services)
        .expect_err("origin is required");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::MissingOrigin)
    ));
    assert!(error
        .to_string()
        .contains("fixed `origin` remote is missing"));
}
