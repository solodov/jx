use super::*;

#[test]
fn status_discovers_authored_pr_without_local_bookmark_or_metadata() {
    // Verifies: GitHub-created PRs appear immediately and repeated status calls do not duplicate them.
    let workspace = discovery_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        authored_open_pull_requests: vec![pull_request_choice_record(
            297480,
            "Revert change",
            "revert/change",
            "main",
            false,
        )],
        pull_request_statuses: BTreeMap::from([(
            297480,
            discovery_status(297480, "Revert change", "revert/change", "main"),
        )]),
        ..FakeServices::default()
    };
    assert!(!workspace.path().join(".jx/stack.toml").exists());

    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("remote-only PR is discovered");
    assert!(result.stdout.contains("#297480"));
    assert!(result.stdout.contains("Revert change"));
    let metadata = read_stack_metadata(&workspace.path()).expect("metadata reads");
    assert_eq!(metadata.nodes.len(), 1);
    assert_eq!(metadata.nodes[0].pull_request, Some(297480));
    assert_eq!(metadata.nodes[0].branch, "revert/change");

    let result = run_with_args_and_services(
        ["jx", "stack", "status", "--format", "json"],
        &environment,
        &services,
    )
    .expect("subsequent JSON status succeeds");
    let value: serde_json::Value = serde_json::from_str(&result.stdout).expect("valid JSON");
    assert_eq!(
        value["repositories"][0]["pullRequests"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(value["repositories"][0]["trunk"]["remote"], "origin");
    assert_eq!(read_stack_metadata(&workspace.path()).unwrap(), metadata);
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![297480], vec![297480]]
    );
    assert_eq!(
        services
            .authored_open_pull_request_calls
            .borrow()
            .as_slice(),
        ["example-user", "example-user"]
    );
    assert_status_did_not_publish(&services);
}

#[test]
fn status_discovery_preserves_cached_ancestry_and_unrelated_prs() {
    // Verifies: remote discovery adds a child without overwriting locally planned bases or work-item intent.
    let workspace = discovery_workspace();
    let mut child = stack_status_node(201, "topic/child", "topic/root", "Local child", false);
    child.parent_branch = Some("topic/root".to_owned());
    child.parent_pull_request = Some(200);
    child.work_ids = vec!["TASK-123".to_owned()];
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            nodes: vec![
                stack_status_node(200, "topic/root", "main", "Cached root", false),
                child,
            ],
            ..StackMetadata::default()
        },
    )
    .expect("metadata writes");
    let services = FakeServices {
        authored_open_pull_requests: vec![
            pull_request_choice_record(201, "Existing child", "topic/child", "main", false),
            pull_request_choice_record(202, "Remote child", "remote/child", "topic/child", true),
        ],
        pull_request_statuses: BTreeMap::from([
            (
                200,
                discovery_status(200, "Cached root", "topic/root", "main"),
            ),
            (
                201,
                discovery_status(201, "Existing child", "topic/child", "main"),
            ),
            (202, {
                let mut status =
                    discovery_status(202, "Remote child", "remote/child", "topic/child");
                status.draft = true;
                status
            }),
        ]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let result = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect("status merges discovery");
    assert!(result.stdout.contains("Cached root"));
    assert!(result.stdout.contains("Remote child"));
    let metadata = read_stack_metadata(&workspace.path()).unwrap();
    assert_eq!(metadata.nodes.len(), 3);
    let child = metadata
        .nodes
        .iter()
        .find(|node| node.pull_request == Some(201))
        .unwrap();
    assert_eq!(child.base_branch, "topic/root");
    assert_eq!(child.parent_pull_request, Some(200));
    assert_eq!(child.work_ids, ["TASK-123"]);
    let remote = metadata
        .nodes
        .iter()
        .find(|node| node.pull_request == Some(202))
        .unwrap();
    assert_eq!(remote.parent_pull_request, Some(201));
    assert!(remote.draft);
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![200, 201, 202]]
    );
    assert_status_did_not_publish(&services);
}

#[test]
fn status_reuses_authored_discovery_for_branch_only_nodes() {
    // Verifies: an authored search hit avoids a redundant per-head lookup and hydrates the cached node once.
    let workspace = discovery_workspace();
    let mut node = stack_status_node(201, "topic/child", "main", "Branch only", false);
    node.pull_request = None;
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            nodes: vec![node],
            ..StackMetadata::default()
        },
    )
    .unwrap();
    let services = FakeServices {
        authored_open_pull_requests: vec![pull_request_choice_record(
            201,
            "Child",
            "topic/child",
            "main",
            false,
        )],
        pull_request_statuses: BTreeMap::from([(
            201,
            discovery_status(201, "Child", "topic/child", "main"),
        )]),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    run_with_args_and_services(["jx", "stack", "status"], &environment, &services).unwrap();
    assert!(services.pull_request_head_calls.borrow().is_empty());
    let metadata = read_stack_metadata(&workspace.path()).unwrap();
    assert_eq!(metadata.nodes.len(), 1);
    assert_eq!(metadata.nodes[0].pull_request, Some(201));
    assert_eq!(
        services.pull_request_status_calls.borrow().as_slice(),
        &[vec![201]]
    );
}

#[test]
fn status_all_discovers_uncached_repositories_and_keeps_failures_isolated() {
    // Verifies: global discovery respects repo filters, omits empty repos, and isolates search failures.
    let workspace = discovery_workspace();
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
    let mut roots = BTreeMap::new();
    for name in ["api-alpha", "web-beta", "empty"] {
        let root = workspace.create_jj_workspace(&format!("projects/{name}"));
        TestWorkspace::write_git_config_at(
            &root,
            &format!(
                "[remote \"origin\"]\n    url = https://github.com/example-owner/{name}.git\n"
            ),
        );
        roots.insert(name, root);
    }
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let mut services = FakeServices {
        authored_open_pull_requests_by_repository: BTreeMap::from([
            (
                "example-owner/api-alpha".to_owned(),
                vec![pull_request_choice_record(
                    201,
                    "Alpha PR",
                    "remote/alpha",
                    "main",
                    false,
                )],
            ),
            (
                "example-owner/web-beta".to_owned(),
                vec![pull_request_choice_record(
                    202,
                    "Beta PR",
                    "remote/beta",
                    "main",
                    false,
                )],
            ),
        ]),
        pull_request_statuses: BTreeMap::from([
            (
                201,
                discovery_status(201, "Alpha PR", "remote/alpha", "main"),
            ),
            (202, discovery_status(202, "Beta PR", "remote/beta", "main")),
        ]),
        ..FakeServices::default()
    };
    let filtered = run_with_args_and_services(
        ["jx", "stack", "status", "-a", "api-*"],
        &environment,
        &services,
    )
    .unwrap();
    assert!(filtered.stdout.contains("Alpha PR"));
    assert!(!filtered.stdout.contains("Beta PR"));
    assert!(!roots["web-beta"].join(".jx/stack.toml").exists());

    let all = run_with_args_and_services(["jx", "stack", "status", "-a"], &environment, &services)
        .unwrap();
    assert!(all.stdout.contains("Alpha PR"));
    assert!(all.stdout.contains("Beta PR"));
    assert!(!all.stdout.contains("~/projects/empty"));
    assert!(!roots["empty"].join(".jx/stack.toml").exists());
    assert_eq!(
        read_stack_metadata(&roots["web-beta"]).unwrap().nodes[0].pull_request,
        Some(202)
    );

    let cached = fs::read(roots["api-alpha"].join(".jx/stack.toml")).unwrap();
    services
        .authored_open_pull_request_errors
        .insert("example-owner/api-alpha".to_owned());
    let partial =
        run_with_args_and_services(["jx", "stack", "status", "-a"], &environment, &services)
            .unwrap();
    assert!(partial.stdout.contains("discovery unavailable"));
    assert!(partial.stdout.contains("Beta PR"));
    assert_eq!(
        fs::read(roots["api-alpha"].join(".jx/stack.toml")).unwrap(),
        cached
    );
    assert_status_did_not_publish(&services);
}

#[test]
fn status_discovery_failure_does_not_rewrite_the_cache() {
    // Verifies: a failed authored search is reported, not treated as an empty result that prunes state.
    let workspace = discovery_workspace();
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            nodes: vec![stack_status_node(
                200,
                "topic/root",
                "main",
                "Cached root",
                false,
            )],
            ..StackMetadata::default()
        },
    )
    .unwrap();
    let cached = fs::read(workspace.path().join(".jx/stack.toml")).unwrap();
    let services = FakeServices {
        authored_open_pull_request_errors: BTreeSet::from(
            ["example-owner/example-repo".to_owned()],
        ),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let error = run_with_args_and_services(["jx", "stack", "status"], &environment, &services)
        .expect_err("discovery failure is reported");
    assert!(error.to_string().contains("discovery unavailable"));
    assert_eq!(
        fs::read(workspace.path().join(".jx/stack.toml")).unwrap(),
        cached
    );
    assert!(services.pull_request_status_calls.borrow().is_empty());
    assert_status_did_not_publish(&services);
}

#[test]
fn status_help_explains_automatic_discovery() {
    let help = help_output(["jx", "stack", "status", "--help"]);
    assert!(help.contains("without local bookmarks"));
    assert!(help.contains("even without existing stack metadata"));
    assert!(help.contains("does not sync PR bases or descriptions"));
}

fn discovery_workspace() -> TestWorkspace {
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        "[remote \"origin\"]\n    url = https://github.com/example-owner/example-repo.git\n",
    );
    workspace
}

fn discovery_status(number: u64, title: &str, branch: &str, base: &str) -> PullRequestStatusRecord {
    stack_status_record(
        number,
        title,
        branch,
        base,
        PullRequestCheckStatus::Passing,
        PullRequestReviewStatus::Approved,
        ReviewerSelection::default(),
    )
}

fn assert_status_did_not_publish(services: &FakeServices) {
    assert!(services.sync_pull_request_pushes.borrow().is_empty());
    assert!(services.push_bookmark_calls.borrow().is_empty());
    assert_eq!(services.published_pull_request_count.get(), 0);
}
