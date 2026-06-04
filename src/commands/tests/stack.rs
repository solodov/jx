use super::*;

#[test]
fn stack_help_describes_cached_display_and_live_refresh() {
    // Verifies: Stack help distinguishes local cached display from GitHub-backed refresh.
    let help = help_output(["jx", "stack", "--help"]);

    assert!(help.contains(
        "Show, move, publish, status-check, or refresh repo-local pull request stack state"
    ));
    assert!(help.contains(".jx/stack.toml"));
    assert!(help.contains("without contacting GitHub"));
    assert!(help.contains("create or update pull requests"));
    assert!(help.contains("--onto"));
    assert!(help.contains("--trunk"));
    assert!(help.contains("--no-sync"));
    assert!(help.contains("show"));
    assert!(help.contains("refresh"));
    assert!(help.contains("status"));
    assert!(help.contains("plan"));
    assert!(help.contains("publish"));
    assert!(!help.contains("track"));
    assert!(!help.contains("reset"));
}

#[test]
fn stack_status_renders_check_and_review_summary() {
    // Verifies: stack status fetches batched GitHub health for locally tracked PR numbers.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                stack_status_node(101, "topic/root", "main", "Root change", false),
                stack_status_node(102, "topic/child", "topic/root", "Child change", true),
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/root".to_owned(), "topic/child".to_owned()],
        pull_request_statuses: BTreeMap::from([
            (101, {
                let mut status = stack_status_record(
                    101,
                    "Root change",
                    "topic/root",
                    "main",
                    PullRequestCheckStatus::Passing,
                    PullRequestReviewStatus::Approved,
                    ReviewerSelection::default(),
                );
                status.labels = vec![
                    PullRequestLabel {
                        name: "bug".to_owned(),
                        color: "d73a4a".to_owned(),
                    },
                    PullRequestLabel {
                        name: "help wanted".to_owned(),
                        color: "008672".to_owned(),
                    },
                ];
                status.approved_reviewers = vec!["reviewer-approved".to_owned()];
                status
            }),
            (102, {
                let mut status = stack_status_record(
                    102,
                    "Child change",
                    "topic/child",
                    "topic/root",
                    PullRequestCheckStatus::Pending,
                    PullRequestReviewStatus::ReviewRequested,
                    ReviewerSelection::new(["reviewer-one"], ["platform"]),
                );
                status.draft = true;
                status.labels = vec![PullRequestLabel {
                    name: "ui".to_owned(),
                    color: "fbca04".to_owned(),
                }];
                status
            }),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![101, 102]]
    );
    assert!(result.stdout.contains(&format!(
        "{}  ",
        osc8_link(
            "https://github.com/example-owner/example-repo",
            "example-owner/example-repo"
        )
    )));
    assert!(result.stdout.contains("(origin/main 3 commits behind)"));
    assert!(result.stdout.contains("PR       Chk  Rev  Title"));
    assert!(result.stdout.contains(&format!(
        "{}  ✓    ✓    ◯ Root change [bug] [help wanted] reviewer-approved",
        stack_status_pull_request_cell(101)
    )));
    assert!(result.stdout.contains(&format!(
        "{}  ◷    ◷    └ ◌ Child change [ui] reviewer-one, team/platform",
        stack_status_pull_request_cell(102)
    )));
    assert!(result.stdout.contains("Legend:\n  Title: ◯ ready, ◌ draft"));
    assert!(result.stdout.contains("labels and reviewers follow title"));
    assert!(result
        .stdout
        .contains("Chk: ✓ passing, ✗ failing, ◷ pending"));
    assert!(result
        .stdout
        .contains("Rev: ✓ approved, ! changes requested, ◷ waiting"));
}

#[test]
fn stack_status_moves_configured_review_gate_failures_to_review_column() {
    // Verifies: repo policy can classify approval-gate checks separately from test health.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    workspace.write_file(
        ".jx.toml",
        r#"
[[repo.rules]]
repo = "example-owner/example-repo"

[[repo.rules.stack_status.review_gate_checks]]
name = "approval gate"
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![stack_status_node(
                120,
                "topic/review-gate",
                "main",
                "Review gate change",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let mut status = stack_status_record(
        120,
        "Review gate change",
        "topic/review-gate",
        "main",
        PullRequestCheckStatus::Failing,
        PullRequestReviewStatus::NotReviewed,
        ReviewerSelection::default(),
    );
    status.checks = vec![
        // GitHub rollups can include stale duplicate contexts; the latest matching name wins.
        PullRequestCheck {
            name: "approval gate".to_owned(),
            status: PullRequestCheckStatus::Failing,
        },
        PullRequestCheck {
            name: "ci/build".to_owned(),
            status: PullRequestCheckStatus::Failing,
        },
        PullRequestCheck {
            name: "ci/build".to_owned(),
            status: PullRequestCheckStatus::Passing,
        },
    ];
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/review-gate".to_owned()],
        pull_request_statuses: BTreeMap::from([(120, status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(result.stdout.contains(&format!(
        "{}  ✓    ◷    ◯ Review gate change",
        stack_status_pull_request_cell(120)
    )));
}

#[test]
fn stack_status_resolves_branch_only_stack_nodes_before_fetching_status() {
    // Verifies: status repairs stack entries that know only the local branch, not the PR number.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/branch-only-status".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: None,
                parent_pull_request: None,
                title: "Example branch-only status".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let mut status = stack_status_record(
        451,
        "Example branch-only status",
        "topic/branch-only-status",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::ReviewRequested,
        ReviewerSelection::new(["example-reviewer"], std::iter::empty::<&str>()),
    );
    status.draft = true;
    let services = FakeServices {
        pull_requests_by_head: BTreeMap::from([(
            "topic/branch-only-status".to_owned(),
            pull_request_choice_record(
                451,
                "Example branch-only status",
                "topic/branch-only-status",
                "main",
                true,
            ),
        )]),
        pull_request_statuses: BTreeMap::from([(451, status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert_eq!(
        services.pull_request_head_calls.borrow().as_slice(),
        ["topic/branch-only-status"]
    );
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![451]]
    );
    assert!(result.stdout.contains(&format!(
        "{}  ✓    ◷    ◌ Example branch-only status example-reviewer",
        stack_status_pull_request_cell(451)
    )));
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes[0].pull_request, Some(451));
}

#[test]
fn stack_status_colorizes_labels_with_github_backgrounds() {
    // Verifies: colored stack status renders labels as GitHub-colored chips.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                stack_status_node(110, "topic/labeled", "main", "Labeled change", false),
                stack_status_node(111, "topic/draft-label", "main", "Draft label", true),
            ],
        },
    )
    .expect("stack metadata writes");
    let mut ready_status = stack_status_record(
        110,
        "Labeled change",
        "topic/labeled",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    ready_status.labels = vec![
        PullRequestLabel {
            name: "bug".to_owned(),
            color: "000000".to_owned(),
        },
        PullRequestLabel {
            name: "docs".to_owned(),
            color: "fbca04".to_owned(),
        },
    ];
    ready_status.requested_reviewers =
        ReviewerSelection::new(["reviewer-pending"], std::iter::empty::<&str>());
    ready_status.approved_reviewers = vec!["reviewer-approved".to_owned()];
    let mut draft_status = stack_status_record(
        111,
        "Draft label",
        "topic/draft-label",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    draft_status.draft = true;
    draft_status.labels = vec![PullRequestLabel {
        name: "ui".to_owned(),
        color: "d73a4a".to_owned(),
    }];
    draft_status.requested_reviewers =
        ReviewerSelection::new(["draft-pending"], std::iter::empty::<&str>());
    draft_status.approved_reviewers = vec!["draft-approved".to_owned()];
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/labeled".to_owned(), "topic/draft-label".to_owned()],
        pull_request_statuses: BTreeMap::from([(110, ready_status), (111, draft_status)]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);
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
        ["jx", "stack", "status"],
        &environment,
        &services,
        &NoProgress,
        prompts,
        OutputMode { color: true },
    )
    .expect("colored stack status succeeds");

    assert!(result.stdout.contains(
        "\x1b[48;2;0;0;0m\x1b[38;2;255;255;255m bug \x1b[0m\x1b[48;2;251;202;4m\x1b[38;2;0;0;0m docs \x1b[0m"
    ));
    assert!(result
        .stdout
        .contains("\x1b[1m\x1b[30mreviewer-pending\x1b[0m, \x1b[32mreviewer-approved\x1b[0m"));
    assert!(result.stdout.contains(
        "\x1b[48;2;246;237;234m\x1b[38;2;190;184;176m ui \x1b[0m\x1b[2m\x1b[38;2;190;184;176m draft-pending, draft-approved"
    ));
    assert!(result
        .stdout
        .contains("\x1b[2m\x1b[38;2;190;184;176mLegend:"));
    assert!(!result.stdout.contains("[ui]"));
    assert!(!result.stdout.contains("\x1b[1m\x1b[30mdraft-pending"));
    assert!(!result.stdout.contains("\x1b[32mdraft-approved"));
}

#[test]
fn stack_status_hides_merged_rows_and_prunes_fully_merged_cache_trees() {
    // Verifies: status output focuses on actionable PRs while cleaning completed cached stacks.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                stack_status_node(201, "merged/done", "main", "Fully merged change", false),
                stack_status_node(202, "mixed/root", "main", "Merged ancestor", false),
                stack_status_node(203, "mixed/child", "mixed/root", "Open child", false),
            ],
        },
    )
    .expect("stack metadata writes");
    let mut fully_merged = stack_status_record(
        201,
        "Fully merged change",
        "merged/done",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    fully_merged.merged = true;
    let mut merged_ancestor = stack_status_record(
        202,
        "Merged ancestor",
        "mixed/root",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    merged_ancestor.merged = true;
    let services = FakeServices {
        pull_request_statuses: BTreeMap::from([
            (201, fully_merged),
            (202, merged_ancestor),
            (
                203,
                stack_status_record(
                    203,
                    "Open child",
                    "mixed/child",
                    "mixed/root",
                    PullRequestCheckStatus::Passing,
                    PullRequestReviewStatus::Approved,
                    ReviewerSelection::default(),
                ),
            ),
        ]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(!result.stdout.contains("Fully merged change"));
    assert!(!result.stdout.contains("Merged ancestor"));
    assert!(result.stdout.contains("Open child"));
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(
        metadata
            .nodes
            .iter()
            .map(|node| (node.branch.as_str(), node.merged))
            .collect::<Vec<_>>(),
        vec![("mixed/root", true), ("mixed/child", false)]
    );
}

#[test]
fn stack_status_removes_closed_rows_from_cache_and_output() {
    // Verifies: closed PRs are treated as stale stack cache entries, not actionable rows.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                stack_status_node(301, "closed/root", "main", "Closed root", false),
                stack_status_node(302, "open/child", "closed/root", "Open child", false),
            ],
        },
    )
    .expect("stack metadata writes");
    let mut closed = stack_status_record(
        301,
        "Closed root",
        "closed/root",
        "main",
        PullRequestCheckStatus::Failing,
        PullRequestReviewStatus::ReviewRequired,
        ReviewerSelection::default(),
    );
    closed.closed = true;
    let services = FakeServices {
        pull_request_statuses: BTreeMap::from([
            (301, closed),
            (
                302,
                stack_status_record(
                    302,
                    "Open child",
                    "open/child",
                    "closed/root",
                    PullRequestCheckStatus::Passing,
                    PullRequestReviewStatus::Approved,
                    ReviewerSelection::default(),
                ),
            ),
        ]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");

    assert!(!result.stdout.contains("Closed root"));
    assert!(result.stdout.contains("Open child"));
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes.len(), 1);
    assert_eq!(metadata.nodes[0].branch, "open/child");
    assert_eq!(metadata.nodes[0].parent_branch, None);
    assert_eq!(metadata.nodes[0].parent_pull_request, None);
}

#[test]
fn stack_status_json_renders_machine_readable_pull_request_health() {
    // Verifies: JSON output exposes stable labels for scripting without terminal formatting.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![stack_status_node(
                103,
                "topic/json",
                "main",
                "JSON change",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/json".to_owned()],
        pull_request_statuses: BTreeMap::from([(103, {
            let mut status = stack_status_record(
                103,
                "JSON change",
                "topic/json",
                "main",
                PullRequestCheckStatus::Failing,
                PullRequestReviewStatus::ChangesRequested,
                ReviewerSelection::default(),
            );
            status.labels = vec![PullRequestLabel {
                name: "bug".to_owned(),
                color: "d73a4a".to_owned(),
            }];
            status
        })]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "status", "--format", "json"],
        &environment,
        &services,
    )
    .expect("stack status json succeeds");
    let value: serde_json::Value = serde_json::from_str(&result.stdout).expect("valid json");

    assert_eq!(value["command"], "stack-status");
    assert_eq!(
        value["repositories"][0]["repository"],
        "example-owner/example-repo"
    );
    assert_eq!(value["repositories"][0]["trunk"]["remote"], "origin");
    assert_eq!(value["repositories"][0]["trunk"]["branch"], "main");
    assert_eq!(value["repositories"][0]["trunk"]["state"], "github-ahead");
    assert_eq!(value["repositories"][0]["trunk"]["githubAheadBy"], 3);
    assert_eq!(value["repositories"][0]["pullRequests"][0]["number"], 103);
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["checkStatus"],
        "failing"
    );
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["reviewStatus"],
        "changes_requested"
    );
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["labels"][0]["name"],
        "bug"
    );
    assert_eq!(
        value["repositories"][0]["pullRequests"][0]["labels"][0]["color"],
        "d73a4a"
    );
}

#[test]
fn stack_status_all_filters_configured_repositories() {
    // Verifies: -a positional patterns match configured repository identities like jx sync -a.
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
    write_stack_metadata(
        &alpha,
        &StackMetadata {
            version: 1,
            nodes: vec![stack_status_node(
                201,
                "topic/alpha",
                "main",
                "Alpha change",
                false,
            )],
        },
    )
    .expect("alpha stack metadata writes");
    write_stack_metadata(
        &beta,
        &StackMetadata {
            version: 1,
            nodes: vec![stack_status_node(
                202,
                "topic/beta",
                "main",
                "Beta change",
                false,
            )],
        },
    )
    .expect("beta stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        pull_request_statuses: BTreeMap::from([(
            201,
            stack_status_record(
                201,
                "Alpha change",
                "topic/alpha",
                "main",
                PullRequestCheckStatus::Passing,
                PullRequestReviewStatus::Approved,
                ReviewerSelection::default(),
            ),
        )]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "status", "-a", "api-*"],
        &environment,
        &services,
    )
    .expect("filtered global stack status succeeds");

    assert!(result.stdout.contains("Stack status: 1 repository checked"));
    assert!(result.stdout.contains("api-alpha"));
    assert!(result.stdout.contains("Alpha change"));
    assert!(!result.stdout.contains("web-beta"));
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![201]]
    );
}

#[test]
fn stack_status_all_skips_repositories_pruned_to_empty_stack_state() {
    // Verifies: -a status cleans fully merged cached stacks and omits them from global output.
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
    let merged_repo = workspace.create_jj_workspace("projects/api-merged");
    let open_repo = workspace.create_jj_workspace("projects/api-open");
    TestWorkspace::write_git_config_at(
        &merged_repo,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api-merged.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &open_repo,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api-open.git
"#,
    );
    write_stack_metadata(
        &merged_repo,
        &StackMetadata {
            version: 1,
            nodes: vec![stack_status_node(
                401,
                "topic/merged",
                "main",
                "Merged change",
                false,
            )],
        },
    )
    .expect("merged stack metadata writes");
    write_stack_metadata(
        &open_repo,
        &StackMetadata {
            version: 1,
            nodes: vec![stack_status_node(
                402,
                "topic/open",
                "main",
                "Open change",
                false,
            )],
        },
    )
    .expect("open stack metadata writes");
    let mut merged = stack_status_record(
        401,
        "Merged change",
        "topic/merged",
        "main",
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    );
    merged.merged = true;
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        pull_request_statuses: BTreeMap::from([
            (401, merged),
            (
                402,
                stack_status_record(
                    402,
                    "Open change",
                    "topic/open",
                    "main",
                    PullRequestCheckStatus::Passing,
                    PullRequestReviewStatus::Approved,
                    ReviewerSelection::default(),
                ),
            ),
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "status", "-a", "api-*"],
        &environment,
        &services,
    )
    .expect("global stack status succeeds");

    assert!(result.stdout.contains(
        "Stack status: 2 repositories checked, 1 repository with stacks, 1 pull request"
    ));
    assert!(!result.stdout.contains("api-merged"));
    assert!(result.stdout.contains("api-open"));
    assert_eq!(
        read_stack_metadata(&merged_repo).expect("merged metadata reads"),
        StackMetadata::default()
    );
}

#[test]
fn stack_status_records_perf_steps() {
    // Verifies: stack status emits timing spans from its first implementation.
    let workspace = TestWorkspace::new();
    let perf_log = workspace.path().join("perf/stack-status.jsonl");
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![stack_status_node(
                301,
                "topic/perf",
                "main",
                "Perf change",
                false,
            )],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("JX_PERF_LOG".to_owned(), perf_log.display().to_string())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/perf".to_owned()],
        pull_request_statuses: BTreeMap::from([(
            301,
            stack_status_record(
                301,
                "Perf change",
                "topic/perf",
                "main",
                PullRequestCheckStatus::Passing,
                PullRequestReviewStatus::Approved,
                ReviewerSelection::default(),
            ),
        )]),
        ..FakeServices::default()
    };

    run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("stack status succeeds");
    let log = fs::read_to_string(perf_log).expect("perf log writes");
    let event = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("perf event json"))
        .find(|event| event["op"] == "stack.status")
        .expect("stack status span is logged");

    assert_eq!(event["op"], "stack.status");
    assert_eq!(event["pr_count"], 1);
    let steps = event["steps"].as_array().expect("steps");
    assert!(steps
        .iter()
        .any(|step| step["name"] == "load_stack_snapshot"));
    assert!(steps
        .iter()
        .any(|step| step["name"] == "fetch_github_status"));
    assert!(steps
        .iter()
        .any(|step| step["name"] == "maintain_stack_metadata"));
    assert!(steps
        .iter()
        .any(|step| step["name"] == "fetch_trunk_status"));
    assert!(steps.iter().any(|step| step["name"] == "render"));
}

fn stack_status_pull_request_cell(number: u64) -> String {
    let target = format!("#{number}");
    format!(
        "{}{}",
        example_pull_request_link(number),
        " ".repeat(7_usize.saturating_sub(target.chars().count()))
    )
}

fn stack_status_node(
    number: u64,
    branch: &str,
    base_branch: &str,
    title: &str,
    draft: bool,
) -> StackMetadataNode {
    StackMetadataNode {
        branch: branch.to_owned(),
        base_branch: base_branch.to_owned(),
        parent_branch: (base_branch != "main").then(|| base_branch.to_owned()),
        pull_request: Some(number),
        parent_pull_request: None,
        title: title.to_owned(),
        url: Some(format!(
            "https://github.com/example-owner/example-repo/pull/{number}"
        )),
        draft,
        merged: false,
    }
}

fn stack_status_record(
    number: u64,
    title: &str,
    branch: &str,
    base_branch: &str,
    check_status: PullRequestCheckStatus,
    review_status: PullRequestReviewStatus,
    requested_reviewers: ReviewerSelection,
) -> PullRequestStatusRecord {
    PullRequestStatusRecord {
        number,
        title: title.to_owned(),
        url: Some(format!(
            "https://github.com/example-owner/example-repo/pull/{number}"
        )),
        head_branch: branch.to_owned(),
        base_branch: base_branch.to_owned(),
        draft: false,
        merged: false,
        closed: false,
        check_status,
        checks: Vec::new(),
        review_status,
        requested_reviewers,
        approved_reviewers: Vec::new(),
        labels: Vec::new(),
        latest_commit_oid: Some(format!("commit-{number}")),
    }
}

#[test]
fn stack_publish_existing_plans_include_projected_stack_context() {
    // Verifies: existing stacked PR updates can write the final stack-context body during publish.
    let mut root = preview_plan();
    root.title = "Root change".to_owned();
    root.body = "Root body".to_owned();
    root.bookmark.branch = "topic/root".to_owned();
    root.head = PullRequestHead::same_repository("example-owner", "topic/root");
    root.existing_pull_request = Some(pull_request_choice_record(
        1,
        "Root change",
        "topic/root",
        "main",
        false,
    ));

    let mut child = preview_plan();
    child.title = "Child change".to_owned();
    child.body = "Child body".to_owned();
    child.bookmark.branch = "topic/child".to_owned();
    child.head = PullRequestHead::same_repository("example-owner", "topic/child");
    child.base = "topic/root".to_owned();
    child.existing_pull_request = Some(pull_request_choice_record(
        2,
        "Child change",
        "topic/child",
        "topic/root",
        false,
    ));

    let mut plans = vec![root, child];
    add_projected_stack_context_to_existing_plans(&mut plans);

    assert!(plans[0].body.contains("<!-- jx-stack:start -->"));
    assert!(plans[0].body.contains("#1 Root change"));
    assert!(plans[0].body.contains("#2 Child change"));
    assert!(plans[1].body.contains("<!-- jx-stack:start -->"));
    assert!(plans[1].body.contains("#1 Root change"));
    assert!(plans[1].body.contains("#2 Child change"));
}

#[test]
fn stack_plan_renders_neighbourhood_tree_without_github_mutations() {
    // Verifies: stack plan shows selected and context commits in tree order without publishing.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut root = workspace_facts();
    root.target_change.short_commit_id = "aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    let mut left = workspace_facts();
    left.target_change.short_commit_id = "bbbbbbbb".to_owned();
    left.target_change.description = "Left change".to_owned();
    let mut right = workspace_facts();
    right.target_change.short_commit_id = "cccccccc".to_owned();
    right.target_change.description = "Right change".to_owned();
    let services = FakeServices {
        stack_plan_facts: Some(StackPlanFacts {
            trunk: root.trunk.clone(),
            nodes: vec![
                crate::jj::StackPlanNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPlanNodeFacts {
                    workspace: left,
                    parent_index: Some(0),
                },
                crate::jj::StackPlanNodeFacts {
                    workspace: right,
                    parent_index: Some(0),
                },
            ],
            selected_indexes: vec![1, 2],
            anchor_index: None,
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "plan", "-r", "left | right"],
        &environment,
        &services,
    )
    .expect("stack plan succeeds");

    assert_eq!(
        services.stack_plan_selections.borrow().as_slice(),
        [StackPlanSelection::ExplicitRevisions {
            revisions: vec!["left | right".to_owned()]
        }]
    );
    assert_eq!(
        result.stdout,
        "Stack plan: 3 commits, 2 selected\nBase: main @ 11112222\nRoot: aaaaaaaa Root change\n\n◯ aaaaaaaa Root change  context\n├─ ◉ bbbbbbbb Left change  selected\n└─ ◉ cccccccc Right change  selected\n\nSelected revisions share one stack root. Publish would create/update PRs for selected rows.\n"
    );
    assert!(services.sync_pull_request_pushes.borrow().is_empty());
}

#[test]
fn stack_publish_without_revision_publishes_inferred_stack() {
    // Verifies: stack publish expands the working-copy stack when no explicit revset is supplied.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let root_pr =
        pull_request_choice_record(42, "Root change", "example-user/00-aaaaaaaa", "main", false);
    let child_pr = pull_request_choice_record(
        43,
        "Child change",
        "example-user/01-bbbbbbbb",
        "example-user/00-aaaaaaaa",
        false,
    );
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        sync_pull_requests: vec![root_pr, child_pr],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "publish"], &environment, &services)
        .expect("stack publishes");

    assert_eq!(
        services.stack_publish_selections.borrow().first(),
        Some(&StackPublishSelection::InferredStack { anchor: None })
    );
    assert_eq!(
        services.sync_pull_request_pushes.borrow()[0]
            .bookmarks
            .iter()
            .map(|bookmark| (
                bookmark.branch.as_str(),
                bookmark.pull_request_base.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("example-user/00-aaaaaaaa", Some("main")),
            ("example-user/01-bbbbbbbb", Some("example-user/00-aaaaaaaa")),
        ]
    );
    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nCreated {}\nStack: refreshed stack context on {}, {}\n",
            example_pull_request_link(42),
            example_pull_request_link(43),
            example_pull_request_link(42),
            example_pull_request_link(43),
        )
    );
}

#[test]
fn stack_publish_ignores_empty_commits_in_selected_stack() {
    // Verifies: empty commits do not block publishing non-empty stack descendants.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut empty_root = workspace_facts();
    empty_root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    empty_root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    empty_root.target_change.description = "Empty root".to_owned();
    empty_root.target_change.is_empty = true;
    empty_root.changed_files = Vec::new();
    empty_root.local_bookmarks_at_target = vec!["example-user/empty-root".to_owned()];
    empty_root.nearest_ancestor_bookmark = None;
    empty_root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = Some("example-user/empty-root".to_owned());
    child.stack_index = 1;
    let child_pr = pull_request_choice_record(
        42,
        "Child change",
        "example-user/01-bbbbbbbb",
        "main",
        false,
    );
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: empty_root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![0, 1],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        }),
        sync_pull_requests: vec![child_pr],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "publish"], &environment, &services)
        .expect("stack publishes non-empty child");

    assert_eq!(services.published_pull_request_count.get(), 1);
    assert_eq!(
        services.sync_pull_request_pushes.borrow()[0]
            .bookmarks
            .iter()
            .map(|bookmark| (
                bookmark.branch.as_str(),
                bookmark.pull_request_base.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![("example-user/01-bbbbbbbb", Some("main"))]
    );
    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nStack: refreshed stack context on {}\n",
            example_pull_request_link(42),
            example_pull_request_link(42),
        )
    );
}

#[test]
fn stack_publish_updates_only_metadata_when_existing_branch_is_up_to_date() {
    // Verifies: unchanged existing PRs can update reviewers, labels, and ready state without PR code updates.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let existing = PullRequestRecord {
        number: 77,
        title: "Existing title".to_owned(),
        body: Some("Existing body".to_owned()),
        head_branch: "example-user/02-zzzzzzzz".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/77".to_owned()),
        draft: true,
        merged: false,
        reviewers: ReviewerSelection::new(["existing-reviewer"], std::iter::empty::<&str>()),
    };
    let services = FakeServices {
        existing_pull_request: Some(existing),
        push: PushOutcome {
            branch: String::new(),
            pushed_refs: 0,
            pushed_commits: Vec::new(),
        },
        bookmark_update: BookmarkUpdate {
            branch: String::new(),
            created: false,
        },
        reviewer_candidates: vec![ReviewerCandidate::new(
            ReviewerTarget::team("example-org/platform", "platform"),
            vec!["matched 1 file".to_owned()],
        )],
        expected_draft: Some(false),
        expected_labels: vec!["needs-review".to_owned()],
        expected_reviewers: Some(ReviewerSelection::new(["existing-reviewer"], ["platform"])),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        [
            "jx",
            "stack",
            "publish",
            "--ready",
            "--label",
            "needs-review",
        ],
        &environment,
        &services,
    )
    .expect("metadata-only stack publish succeeds");

    assert_eq!(services.published_pull_request_count.get(), 0);
    assert_eq!(services.metadata_only_pull_request_count.get(), 1);
    assert_eq!(
        services.push_bookmark_calls.borrow().as_slice(),
        &["example-user/02-zzzzzzzz".to_owned()]
    );
    assert!(services.sync_pull_request_pushes.borrow().is_empty());
    assert_eq!(
        result.stdout,
        format!("Updated {}\n", example_pull_request_link(77))
    );
}

#[test]
fn stack_publish_readiness_selectors_can_create_mixed_ready_draft_stack() {
    // Verifies: readiness revsets assign final ready/draft state within the published stack.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.target_change.commit_id = "11111111aaaaaaaa".to_owned();
    root.target_change.description = "Root change".to_owned();
    root.nearest_ancestor_bookmark = None;
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.commit_id = "22222222bbbbbbbb".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = None;
    child.stack_index = 1;
    let facts = StackPublishFacts {
        nodes: vec![
            crate::jj::StackPublishNodeFacts {
                workspace: root,
                parent_index: None,
            },
            crate::jj::StackPublishNodeFacts {
                workspace: child,
                parent_index: Some(0),
            },
        ],
        publish_indexes: vec![0, 1],
        anchor_index: Some(1),
        metrics: StackPublishMetrics::default(),
    };
    let mut ready_root_facts = facts.clone();
    ready_root_facts.publish_indexes = vec![0];
    ready_root_facts.anchor_index = None;
    let services = FakeServices {
        stack_publish_facts: Some(facts),
        stack_publish_facts_by_revision: BTreeMap::from([("root".to_owned(), ready_root_facts)]),
        expected_drafts: Some(vec![false, true]),
        ..FakeServices::default()
    };

    run_with_args_and_services(
        ["jx", "stack", "publish", "--draft", "--ready=root"],
        &environment,
        &services,
    )
    .expect("stack publishes with mixed readiness");

    assert_eq!(services.published_pull_request_count.get(), 2);
    assert_eq!(
        services.stack_publish_selections.borrow().as_slice(),
        &[
            StackPublishSelection::InferredStack { anchor: None },
            StackPublishSelection::ExplicitRevisions {
                revisions: vec!["root".to_owned()],
            },
        ]
    );
}

#[test]
fn stack_publish_rejects_overlapping_readiness_selectors() {
    // Verifies: contradictory ready/draft revsets fail before any publishing mutation.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut selected = workspace_facts();
    selected.target_change.change_id = "aaaaaaaa11111111".to_owned();
    let facts = StackPublishFacts {
        nodes: vec![crate::jj::StackPublishNodeFacts {
            workspace: selected,
            parent_index: None,
        }],
        publish_indexes: vec![0],
        anchor_index: Some(0),
        metrics: StackPublishMetrics::default(),
    };
    let services = FakeServices {
        stack_publish_facts: Some(facts.clone()),
        stack_publish_facts_by_revision: BTreeMap::from([("selected".to_owned(), facts)]),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(
        [
            "jx",
            "stack",
            "publish",
            "--ready=selected",
            "--draft=selected",
        ],
        &environment,
        &services,
    )
    .expect_err("overlapping readiness is rejected");

    assert!(error
        .to_string()
        .contains("--ready and --draft selectors overlap"));
    assert_eq!(services.published_pull_request_count.get(), 0);
}

#[test]
fn stack_publish_writes_perf_trace_for_publish_and_stack_update() {
    // Verifies: stack publish emits structured timings for the command and stack update phases.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let log_path = workspace.path().join("jx-perf.log");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
            ("JX_PERF_LOG".to_owned(), log_path.display().to_string()),
        ],
    );
    let services = FakeServices::default();

    run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("stack publishes");

    let events = fs::read_to_string(&log_path).expect("perf log writes");
    let events = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("perf line is json"))
        .collect::<Vec<_>>();
    let publish = events
        .iter()
        .find(|event| event["op"] == "stack.publish")
        .expect("stack publish span is logged");
    assert_eq!(publish["repo"], "example-owner/example-repo");
    assert_eq!(publish["explicit_revisions"], true);
    assert!(publish["duration_us"].as_u64().is_some());
    let step_names = publish["steps"]
        .as_array()
        .expect("publish steps are logged")
        .iter()
        .filter_map(|step| step["name"].as_str())
        .collect::<Vec<_>>();
    assert!(step_names.contains(&"load_publish_stack"));
    assert!(step_names.contains(&"stack_publish_facts.resolve_trunk"));
    assert!(step_names.contains(&"stack_publish_facts.workspace_facts"));
    assert!(step_names.contains(&"plan_pull_requests"));
    assert!(step_names.contains(&"pull_request_plan"));
    assert!(step_names.contains(&"publish_pull_request"));
    assert!(step_names.contains(&"update_stack"));
    let update = events
        .iter()
        .find(|event| event["op"] == "stack.update_after_publish")
        .expect("stack update span is logged");
    let update_step_names = update["steps"]
        .as_array()
        .expect("update steps are logged")
        .iter()
        .filter_map(|step| step["name"].as_str())
        .collect::<Vec<_>>();
    assert!(update_step_names.contains(&"load_local_stack_branches"));
    assert!(update_step_names.contains(&"local_stack_branches.resolve_trunk"));
    assert!(update_step_names.contains(&"local_stack_branches.linear_stack_path"));
    assert!(update_step_names
        .contains(&"stack_metadata.apply_existing_local.apply_local_stack_branches"));
    assert!(
        update_step_names.contains(&"stack_metadata.apply_seeded_local.apply_local_stack_branches")
    );
    assert!(update_step_names
        .contains(&"component_metadata.apply_local_stack_metadata.apply_local_stack_branches"));
    assert!(!update_step_names
        .iter()
        .any(|name| name.ends_with(".local_stack_branches")));
    assert!(events
        .iter()
        .any(|event| event["op"] == "stack.sync_pull_requests"));
}

#[test]
fn stack_publish_revision_publishes_explicit_subset_only() {
    // Verifies: -r selects the publish set instead of expanding to the whole stack.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut root = workspace_facts();
    root.target_change.change_id = "aaaaaaaa11111111".to_owned();
    root.stack_index = 0;
    let mut child = workspace_facts();
    child.target_change.change_id = "bbbbbbbb22222222".to_owned();
    child.target_change.description = "Child change".to_owned();
    child.nearest_ancestor_bookmark = Some("topic/root".to_owned());
    child.stack_index = 1;
    let services = FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes: vec![
                crate::jj::StackPublishNodeFacts {
                    workspace: root,
                    parent_index: None,
                },
                crate::jj::StackPublishNodeFacts {
                    workspace: child,
                    parent_index: Some(0),
                },
            ],
            publish_indexes: vec![1],
            anchor_index: None,
            metrics: StackPublishMetrics::default(),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("selected change publishes");

    assert_eq!(
        services.stack_publish_selections.borrow().first(),
        Some(&StackPublishSelection::ExplicitRevisions {
            revisions: vec!["@".to_owned()]
        })
    );
    assert_eq!(
        services.sync_pull_request_pushes.borrow()[0].bookmarks[0]
            .pull_request_base
            .as_deref(),
        Some("topic/root")
    );
    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn stack_subcommand_help_explains_effects() {
    // Verifies: Stack subcommand help names data sources and non-mutating GitHub behavior.
    let show_help = help_output(["jx", "stack", "show", "--help"]);
    assert!(show_help.contains("Show stored pull request stack state"));
    assert!(show_help.contains("without contacting GitHub"));
    assert!(show_help.contains("default when no stack subcommand"));

    let refresh_help = help_output(["jx", "stack", "refresh", "--help"]);
    assert!(refresh_help.contains("Rebuild repo-local stack state"));
    assert!(refresh_help.contains("local PR bookmark heads"));
    assert!(refresh_help.contains("writes"));
    assert!(refresh_help.contains(".jx/stack.toml"));
    assert!(refresh_help.contains("syncs affected PR bases/descriptions"));
    assert!(refresh_help.contains("push branches"));
    assert!(refresh_help.contains("create, close, or delete pull requests"));
}

#[test]
fn stack_interactive_opens_selected_cached_pull_request() {
    // Verifies: Stack selection opens cached PR metadata without querying GitHub.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "example-user/old".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Older change".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "example-user/chosen".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(11),
                    parent_pull_request: None,
                    title: "Chosen change".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/11".to_owned()),
                    draft: true,
                    merged: false,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = RecordingPullRequestSelector::new(0);

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack open succeeds");

    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/example-repo/pull/11\n"
    );
    assert_eq!(
        selector.labels.borrow().as_slice(),
        &[vec![
            "\x1b[2m\x1b[38;2;190;184;176m◌ #11     Chosen change\x1b[0m".to_owned(),
            "◯ #10     Older change".to_owned(),
        ]]
    );
    assert_eq!(
        services.opened_urls.borrow().as_slice(),
        ["https://github.com/example-owner/example-repo/pull/11"]
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [None]
    );
    assert_eq!(services.pull_request_bookmark_calls.get(), 0);
    assert!(services
        .authored_open_pull_request_head_calls
        .borrow()
        .is_empty());
    assert!(services.pull_request_head_calls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_interactive_can_be_cancelled() {
    // Verifies: Quitting the stack selector stops without opening a browser.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "example-user/change".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Change".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = CancellingPullRequestSelector;

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
    assert!(services.opened_urls.borrow().is_empty());
}

#[test]
fn stack_interactive_shows_full_cached_stack_with_draft_rows() {
    // Verifies: Stack selection keeps draft PRs visible even when another branch is current.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "topic/root".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Root".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                    draft: false,
                    merged: true,
                },
                StackMetadataNode {
                    branch: "topic/child".to_owned(),
                    base_branch: "topic/root".to_owned(),
                    parent_branch: Some("topic/root".to_owned()),
                    pull_request: Some(11),
                    parent_pull_request: Some(10),
                    title: "Child".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/11".to_owned()),
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "topic/draft".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(12),
                    parent_pull_request: None,
                    title: "Draft".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/12".to_owned()),
                    draft: true,
                    merged: false,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        ..Default::default()
    };
    let selector = RecordingPullRequestSelector::new(2);

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack open succeeds");

    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/example-repo/pull/11\n"
    );
    assert_eq!(
        selector.labels.borrow().as_slice(),
        &[vec![
            "\x1b[2m\x1b[38;2;190;184;176m◌ #12     Draft\x1b[0m".to_owned(),
            "✓ #10     Root".to_owned(),
            "└─ ◉ #11     Child".to_owned(),
        ]]
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [None]
    );
    assert_eq!(services.pull_request_bookmark_calls.get(), 0);
    assert!(services
        .authored_open_pull_request_head_calls
        .borrow()
        .is_empty());
    assert!(services.pull_request_head_calls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_show_colored_rows_match_interactive_selector_labels() {
    // Verifies: Non-interactive stack output and interactive choices share row rendering.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                StackMetadataNode {
                    branch: "topic/ready".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(10),
                    parent_pull_request: None,
                    title: "Ready".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
                    draft: false,
                    merged: false,
                },
                StackMetadataNode {
                    branch: "topic/draft".to_owned(),
                    base_branch: "main".to_owned(),
                    parent_branch: None,
                    pull_request: Some(11),
                    parent_pull_request: None,
                    title: "Draft".to_owned(),
                    url: Some("https://github.com/example-owner/example-repo/pull/11".to_owned()),
                    draft: true,
                    merged: false,
                },
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
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

    let stack = run_with_args_and_progress(
        ["jx", "stack"],
        &environment,
        &FakeServices::default(),
        &NoProgress,
        prompts,
        OutputMode { color: true },
    )
    .expect("stack show succeeds");
    let selector = RecordingPullRequestSelector::new(0);
    run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i", "--print"],
        &environment,
        &FakeServices::default(),
        &selector,
    )
    .expect("interactive stack open succeeds");

    let stack_rows = stack.stdout.lines().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(selector.labels.borrow().as_slice(), &[stack_rows]);
}

#[test]
fn stack_interactive_prints_selected_pull_request_url() {
    // Verifies: --print suppresses browser launch after cached stack selection.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "example-user/selected".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(12),
                parent_pull_request: None,
                title: "Selected change".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = FixedPullRequestSelector { selected: 0 };

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i", "--print"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack open succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo/pull/12\n"
    );
    assert!(services.opened_urls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_interactive_opens_historical_cached_pull_requests() {
    // Verifies: Cached stack opening trusts stored PR identity instead of live authored filters.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "example-user/reused".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(40568),
                parent_pull_request: None,
                title: "Merged or unowned historical PR".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/40568".to_owned()),
                draft: false,
                merged: true,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = FixedPullRequestSelector { selected: 0 };

    let result = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect("interactive stack open succeeds");

    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/example-repo/pull/40568\n"
    );
    assert_eq!(
        services.opened_urls.borrow().as_slice(),
        ["https://github.com/example-owner/example-repo/pull/40568"]
    );
    assert_eq!(services.pull_request_bookmark_calls.get(), 0);
    assert!(services
        .authored_open_pull_request_head_calls
        .borrow()
        .is_empty());
    assert!(services.pull_request_head_calls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_interactive_reports_missing_stack_state() {
    // Verifies: Interactive stack opening is cache-only and reports empty metadata directly.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices::default();
    let selector = FixedPullRequestSelector { selected: 0 };

    let error = run_with_args_and_pull_request_selector(
        ["jx", "stack", "-i"],
        &environment,
        &services,
        &selector,
    )
    .expect_err("missing stack state is reported");

    assert!(matches!(
        error,
        CommandError::Workflow(WorkflowError::MissingLocalBookmarkPullRequests { repository })
            if repository == "example-owner/example-repo"
    ));
    assert!(services.open_pull_request_selectors.borrow().is_empty());
    assert_eq!(services.pull_request_bookmark_calls.get(), 0);
    assert!(services.pull_request_head_calls.borrow().is_empty());
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn stack_refresh_persists_hierarchy_and_ignore_rules() {
    // Verifies: stack refresh records PR hierarchy in repo-local ignored metadata.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec![
            "topic/child".to_owned(),
            "topic/root".to_owned(),
            "topic/draft".to_owned(),
        ],
        authored_open_pull_requests_by_head: BTreeMap::from([
            (
                "topic/root".to_owned(),
                pull_request_choice_record(10, "Root", "topic/root", "main", false),
            ),
            (
                "topic/child".to_owned(),
                pull_request_choice_record(11, "Child", "topic/child", "topic/root", false),
            ),
            (
                "topic/draft".to_owned(),
                pull_request_choice_record(12, "Draft", "topic/draft", "topic/root", true),
            ),
        ]),
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
        ["jx", "stack", "refresh"],
        &environment,
        &services,
        &progress,
        prompts,
        OutputMode::plain(),
    )
    .expect("stack refresh succeeds");

    assert_eq!(progress.messages(), ["Refreshing pull request stack…"]);
    assert!(progress.finished.get());
    assert_eq!(
        result.stdout,
        "◯ #10     Root\n├─ ◯ #11     Child\n└─ ◌ #12     Draft\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join(".jx/.gitignore")).expect("read gitignore"),
        "/.gitignore\n/workspace.toml\n/stack.toml\n"
    );
    let stack_file =
        fs::read_to_string(workspace.path().join(".jx/stack.toml")).expect("read stack state");
    assert!(stack_file.contains("pull_request = 10"));
    assert!(stack_file.contains("parent_branch = \"topic/root\""));
    let sync_pushes = services.sync_pull_request_pushes.borrow();
    assert_eq!(sync_pushes.len(), 1);
    assert_eq!(
        sync_pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["topic/root", "topic/child", "topic/draft"]
    );
    assert_eq!(
        sync_pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.pull_request_base.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("main"), Some("topic/root"), Some("topic/root")]
    );
}

#[test]
fn stack_without_subcommand_shows_state() {
    // Verifies: bare `jx stack` uses the safe read-only stack view.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let result =
        run_with_args_and_services(["jx", "stack"], &environment, &FakeServices::default())
            .expect("stack show succeeds");

    assert_eq!(result.stdout, "◯ #10     Root\n");
}

#[test]
fn stack_show_reads_primary_checkout_state_from_managed_workspace() {
    // Verifies: stack state is repo-local even when the command runs from a managed workspace.
    let workspace = TestWorkspace::new_under("projects/.work/jx/current");
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
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let primary = workspace.home.join("projects/jx");
    write_stack_metadata(
        &primary,
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let result = run_with_args_and_services(
        ["jx", "stack", "show"],
        &environment,
        &FakeServices::default(),
    )
    .expect("stack show succeeds");

    assert_eq!(result.stdout, "◯ #10     Root\n");
    assert!(!workspace.path().join(".jx/stack.toml").exists());
}

#[test]
fn stack_tracking_retains_missing_stored_ancestors() {
    // Verifies: disappeared parents remain in stack state while children are still tracked.
    let existing = StackMetadata {
        version: 1,
        nodes: vec![
            StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: true,
            },
            StackMetadataNode {
                branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                parent_branch: Some("topic/root".to_owned()),
                pull_request: Some(11),
                parent_pull_request: Some(10),
                title: "Old child".to_owned(),
                url: None,
                draft: false,
                merged: false,
            },
        ],
    };
    let child = pull_request_choice_record(11, "Child", "topic/child", "main", false);

    let metadata = stack_metadata_from_pull_requests(&[child], &existing);

    assert_eq!(
        stack_metadata_rows(&metadata.nodes),
        vec!["✓ #10     Root", "└─ ◯ #11     Child"]
    );
}

#[test]
fn stack_refresh_updates_missing_stored_ancestor_by_pull_request_number() {
    // Verifies: stack refresh updates disappeared parent metadata from its durable PR number.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Stale root".to_owned(),
                url: None,
                draft: false,
                merged: false,
            }],
        },
    )
    .expect("stack metadata writes");
    let root = PullRequestRecord {
        number: 10,
        title: "Merged root".to_owned(),
        body: None,
        head_branch: "deleted/root".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/10".to_owned()),
        draft: false,
        merged: true,
        reviewers: ReviewerSelection::default(),
    };
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec!["topic/child".to_owned()],
        authored_open_pull_requests_by_head: BTreeMap::from([(
            "topic/child".to_owned(),
            pull_request_choice_record(11, "Child", "topic/child", "topic/root", false),
        )]),
        pull_requests_by_number: BTreeMap::from([(10, root)]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "stack", "refresh"], &environment, &services)
        .expect("stack refresh succeeds");

    assert_eq!(result.stdout, "✓ #10     Merged root\n└─ ◯ #11     Child\n");
    assert_eq!(
        services.pull_request_number_calls.borrow().as_slice(),
        [10, 10]
    );
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes[0].branch, "topic/root");
    assert_eq!(metadata.nodes[0].title, "Merged root");
    assert!(metadata.nodes[0].merged);
}

#[test]
fn stack_onto_moves_current_stack_and_syncs_old_and_new_components() {
    // Verifies: Stack moves sync every branch affected by the old and new stack graph.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                stack_node("topic/root", "main", None, 10, "Root"),
                stack_node("topic/child", "topic/root", Some("topic/root"), 11, "Child"),
                stack_node("topic/new-root", "main", None, 12, "New root"),
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        pull_request_bookmarks: vec![
            "topic/root".to_owned(),
            "topic/child".to_owned(),
            "topic/new-root".to_owned(),
        ],
        open_pull_request_candidates: vec!["topic/child".to_owned()],
        local_stack_branches: std::cell::RefCell::new(vec![
            vec![
                local_stack_branch("topic/root", "main", None),
                local_stack_branch("topic/child", "topic/root", Some("topic/root")),
                local_stack_branch("topic/new-root", "main", None),
            ],
            vec![
                local_stack_branch("topic/root", "main", None),
                local_stack_branch("topic/child", "topic/new-root", Some("topic/new-root")),
                local_stack_branch("topic/new-root", "main", None),
            ],
        ]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "--onto", "new-root"],
        &environment,
        &services,
    )
    .expect("stack move succeeds");

    assert_eq!(
        services.stack_move_targets.borrow().as_slice(),
        &[StackMoveTarget::Onto("new-root".to_owned())]
    );
    assert_eq!(
        services.push_syncable_revision_requests.borrow().as_slice(),
        &[
            Some("topic/root".to_owned()),
            Some("topic/child".to_owned()),
            Some("topic/new-root".to_owned()),
        ]
    );
    let synced_metadata = services.sync_pull_request_metadata.borrow();
    let child = synced_metadata
        .last()
        .expect("sync receives metadata")
        .nodes
        .iter()
        .find(|node| node.branch == "topic/child")
        .expect("child metadata exists");
    assert_eq!(child.base_branch, "topic/new-root");
    assert_eq!(child.parent_branch.as_deref(), Some("topic/new-root"));
    assert!(result
        .stdout
        .starts_with("Moved current stack from a1b2c3d4 onto new-root\nSynced:"));
}

#[test]
fn stack_move_no_sync_updates_local_metadata_without_github_mutations() {
    // Verifies: --no-sync keeps the stack move local while still repairing cached stack state.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            nodes: vec![
                stack_node("topic/root", "main", None, 10, "Root"),
                stack_node("topic/child", "topic/root", Some("topic/root"), 11, "Child"),
            ],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        local_stack_branches: std::cell::RefCell::new(vec![vec![
            local_stack_branch("topic/root", "main", None),
            local_stack_branch("topic/child", "main", None),
        ]]),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "--trunk", "--no-sync"],
        &environment,
        &services,
    )
    .expect("local stack move succeeds");

    assert_eq!(
        services.stack_move_targets.borrow().as_slice(),
        &[StackMoveTarget::Trunk]
    );
    assert!(services.fetch_origin_roots.borrow().is_empty());
    assert!(services.push_syncable_revision_requests.borrow().is_empty());
    assert!(services.sync_pull_request_metadata.borrow().is_empty());
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    let child = metadata
        .nodes
        .iter()
        .find(|node| node.branch == "topic/child")
        .expect("child metadata exists");
    assert_eq!(child.base_branch, "main");
    assert_eq!(child.parent_branch, None);
    assert_eq!(
        result.stdout,
        "Moved current stack from a1b2c3d4 onto trunk\nSync skipped (--no-sync)\n"
    );
}

fn stack_node(
    branch: &str,
    base_branch: &str,
    parent_branch: Option<&str>,
    pull_request: u64,
    title: &str,
) -> StackMetadataNode {
    StackMetadataNode {
        branch: branch.to_owned(),
        base_branch: base_branch.to_owned(),
        parent_branch: parent_branch.map(str::to_owned),
        pull_request: Some(pull_request),
        parent_pull_request: None,
        title: title.to_owned(),
        url: None,
        draft: false,
        merged: false,
    }
}

fn local_stack_branch(
    branch: &str,
    base_branch: &str,
    parent_branch: Option<&str>,
) -> LocalStackBranch {
    LocalStackBranch {
        branch: branch.to_owned(),
        base_branch: base_branch.to_owned(),
        parent_branch: parent_branch.map(str::to_owned),
        title: branch.to_owned(),
    }
}

fn help_output<const N: usize>(args: [&str; N]) -> String {
    let error = cli()
        .try_get_matches_from(args)
        .expect_err("help exits before command execution");
    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
    error.to_string()
}
