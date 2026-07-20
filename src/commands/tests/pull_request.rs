use super::*;

#[test]
fn pull_request_accepts_short_task_id_flag_and_renders_published_pr() {
    // Verifies: Pull request accepts short task ID flag and renders the published PR URL.
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
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@", "-t", "ABC-123"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_fixes_flag_records_work_id_and_supplies_task_context() {
    // Verifies: a single fixing work ID doubles as task context when -t is omitted.
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
        expected_task_id: Some(Some("ABC-123".to_owned())),
        ..FakeServices::default()
    };

    run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@", "-F", "ABC-123"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes[0].work_ids, ["ABC-123".to_owned()]);
    assert_eq!(metadata.nodes[0].fixes_work_ids, ["ABC-123".to_owned()]);
}

#[test]
fn pull_request_bare_fixes_uses_workspace_task_id() {
    // Verifies: -F without a value marks the already attached workspace task as fixed.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_workspace_metadata(
        &workspace.path(),
        &WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
            project: None,
            parent: None,
        },
    )
    .expect("workspace metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        expected_task_id: Some(Some("ABC-123".to_owned())),
        ..FakeServices::default()
    };

    run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@", "-F"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(metadata.nodes[0].work_ids, ["ABC-123".to_owned()]);
    assert_eq!(metadata.nodes[0].fixes_work_ids, ["ABC-123".to_owned()]);
}

#[test]
fn pull_request_records_published_pr_in_stack_state() {
    // Verifies: Publishing a PR upserts durable stack state even before a full stack exists.
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
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
    let sync_pushes = services.sync_pull_request_pushes.borrow();
    assert_eq!(sync_pushes.len(), 1);
    assert_eq!(
        sync_pushes[0].bookmarks[0].branch,
        "example-user/02-zzzzzzzz"
    );
    assert_eq!(
        sync_pushes[0].bookmarks[0].pull_request_base.as_deref(),
        Some("example-user/01-ancestor")
    );
    assert_eq!(
        read_stack_metadata(&workspace.path()).expect("stack metadata reads"),
        StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![StackMetadataNode {
                branch: "example-user/02-zzzzzzzz".to_owned(),
                base_branch: "example-user/01-ancestor".to_owned(),
                parent_branch: None,
                pull_request: Some(42),
                parent_pull_request: None,
                title: "example change".to_owned(),
                url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
                draft: false,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            }],
        }
    );
}

#[test]
fn pull_request_refreshes_stack_context_for_published_stack_component() {
    // Verifies: Publishing a stacked PR refreshes generated stack context for every known PR in the component.
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
            work_item_handler_runs: Vec::new(),
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
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut facts = workspace_facts();
    facts.nearest_ancestor_bookmark = Some("topic/root".to_owned());
    let root = pull_request_choice_record(10, "Root", "topic/root", "main", false);
    let child = PullRequestRecord {
        number: 42,
        title: "example change".to_owned(),
        body: None,
        head_branch: "example-user/02-zzzzzzzz".to_owned(),
        base_branch: "topic/root".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
        draft: false,
        merged: false,
        reviewers: ReviewerSelection::default(),
    };
    let services = FakeServices {
        workspace: facts,
        pull_requests_by_head: BTreeMap::from([("topic/root".to_owned(), root.clone())]),
        sync_pull_requests: vec![root.clone(), child.clone()],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nStack: refreshed stack context on {}, {}\n",
            example_pull_request_link(42),
            example_pull_request_link(10),
            example_pull_request_link(42)
        )
    );
    let sync_pushes = services.sync_pull_request_pushes.borrow();
    assert_eq!(sync_pushes.len(), 1);
    assert_eq!(
        sync_pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["topic/root", "example-user/02-zzzzzzzz"]
    );
    assert_eq!(
        sync_pushes[0]
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.pull_request_base.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("main"), Some("topic/root")]
    );
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(
        stack_metadata_rows(&metadata.nodes),
        vec!["◯ #10     Root", "└ ◯ #42     example change"]
    );
    assert_eq!(
        metadata.nodes[1].parent_branch.as_deref(),
        Some("topic/root")
    );
    assert_eq!(metadata.nodes[1].parent_pull_request, Some(10));
}

#[test]
fn pull_request_preserves_stored_parent_when_base_pr_is_missing() {
    // Verifies: A published child keeps known parent metadata even when GitHub cannot refresh that parent by branch.
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
            work_item_handler_runs: Vec::new(),
            nodes: vec![StackMetadataNode {
                branch: "topic/root".to_owned(),
                base_branch: "main".to_owned(),
                parent_branch: None,
                pull_request: Some(10),
                parent_pull_request: None,
                title: "Root".to_owned(),
                url: None,
                draft: false,
                merged: true,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            }],
        },
    )
    .expect("stack metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut facts = workspace_facts();
    facts.nearest_ancestor_bookmark = Some("topic/root".to_owned());
    let services = FakeServices {
        workspace: facts,
        sync_pull_requests: vec![PullRequestRecord {
            number: 42,
            title: "example change".to_owned(),
            body: None,
            head_branch: "example-user/02-zzzzzzzz".to_owned(),
            base_branch: "topic/root".to_owned(),
            html_url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nStack: refreshed stack context on {}\n",
            example_pull_request_link(42),
            example_pull_request_link(42)
        )
    );
    assert_eq!(
        services.pull_request_head_calls.borrow().as_slice(),
        ["topic/root"]
    );
    let metadata = read_stack_metadata(&workspace.path()).expect("stack metadata reads");
    assert_eq!(
        stack_metadata_rows(&metadata.nodes),
        vec!["✓ #10     Root", "└ ◯ #42     example change"]
    );
    assert_eq!(
        metadata.nodes[1].parent_branch.as_deref(),
        Some("topic/root")
    );
    assert_eq!(metadata.nodes[1].parent_pull_request, Some(10));
}

#[test]
fn pull_request_prepare_event_updates_commit_title_before_planning() {
    // Verifies: Prepare handlers rewrite the selected commit before PR metadata is planned.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.event_handlers]]
id = "prepend-task"
on = "pull_request.prepare"
when = "has:task"
run = "prepend_task_id"
"#,
    );
    write_workspace_metadata(
        &workspace.path(),
        &WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
            project: None,
            parent: None,
        },
    )
    .expect("metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut fake_workspace = workspace_facts();
    fake_workspace.target_change.description = "Example title\n\nDetailed body".to_owned();
    let services = FakeServices {
        workspace: fake_workspace,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        services.description_rewrites.borrow().as_slice(),
        &[(
            "a1b2c3d4e5f6".to_owned(),
            "ABC-123: Example title\n\nDetailed body".to_owned()
        )]
    );
    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn stack_pub_task_id_flag_updates_commit_title_before_planning() {
    // Verifies: `stack pub -t` supplies task context to prepare handlers for explicit revsets.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.event_handlers]]
id = "prepend-task"
on = "pull_request.prepare"
when = "has:task"
run = "prepend_task_id"
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let mut fake_workspace = workspace_facts();
    fake_workspace.target_change.description = "Example title\n\nDetailed body".to_owned();
    let services = FakeServices {
        workspace: fake_workspace,
        expected_task_id: Some(Some("FOO-1234".to_owned())),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "pub", "-r", "@", "-t", "FOO-1234"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        services.description_rewrites.borrow().as_slice(),
        &[(
            "a1b2c3d4e5f6".to_owned(),
            "FOO-1234: Example title\n\nDetailed body".to_owned()
        )]
    );
    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_opens_created_pr_when_event_handler_requests_it() {
    // Verifies: PR publishing runs command-side browser effects after the PR is created.
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
        pull_request_event_effects: vec![
            domain::PullRequestEventEffect {
                event: crate::repository::RepoEvent::PullRequestCreated,
                handler_id: Some("label-created".to_owned()),
                kind: PullRequestEventEffectKind::AddLabels {
                    labels: vec!["queued".to_owned()],
                },
            },
            domain::PullRequestEventEffect {
                event: crate::repository::RepoEvent::PullRequestCreated,
                handler_id: Some("open-created".to_owned()),
                kind: PullRequestEventEffectKind::OpenPullRequest {
                    url: "https://github.com/example-owner/example-repo/pull/42".to_owned(),
                },
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!(
            "Created {}\nEvent[label-created]: added label queued\nEvent[open-created]: opened {}\n",
            example_pull_request_link(42),
            example_pull_request_link(42)
        )
    );
    assert_eq!(
        services.opened_urls.borrow().as_slice(),
        &["https://github.com/example-owner/example-repo/pull/42".to_owned()]
    );
}

#[test]
fn pull_request_hides_noop_event_effects_by_default() {
    // Verifies: Default output stays quiet when handlers matched but did not change state.
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
        pull_request_event_effects: vec![
            domain::PullRequestEventEffect {
                event: crate::repository::RepoEvent::PullRequestCreated,
                handler_id: Some("labels-present".to_owned()),
                kind: PullRequestEventEffectKind::LabelsAlreadyPresent {
                    labels: vec!["queued".to_owned()],
                },
            },
            domain::PullRequestEventEffect {
                event: crate::repository::RepoEvent::PullRequestPrepare,
                handler_id: Some("title-present".to_owned()),
                kind: PullRequestEventEffectKind::TitleAlready {
                    title: "ABC-123: Example title".to_owned(),
                },
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_no_event_handlers_suppresses_event_effects() {
    // Verifies: Operators can disable configured PR automation for a single publish.
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
        pull_request_event_effects: vec![domain::PullRequestEventEffect {
            event: crate::repository::RepoEvent::PullRequestCreated,
            handler_id: Some("open-created".to_owned()),
            kind: PullRequestEventEffectKind::OpenPullRequest {
                url: "https://github.com/example-owner/example-repo/pull/42".to_owned(),
            },
        }],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@", "--no-event-handlers"],
        &environment,
        &services,
    )
    .expect("pull request publishes without event handlers");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
    assert!(services.opened_urls.borrow().is_empty());
}

#[test]
fn pull_request_infers_task_id_from_workspace_metadata() {
    // Verifies: Workspace metadata supplies the default task ID for PR planning.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_workspace_metadata(
        &workspace.path(),
        &WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
            project: None,
            parent: None,
        },
    )
    .expect("metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        expected_task_id: Some(Some("ABC-123".to_owned())),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_no_task_id_ignores_workspace_metadata() {
    // Verifies: Operators can opt out of workspace task metadata for ticketless PRs.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_workspace_metadata(
        &workspace.path(),
        &WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
            project: None,
            parent: None,
        },
    )
    .expect("metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        expected_task_id: Some(None),
        ..FakeServices::default()
    };

    run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@", "--no-task-id"],
        &environment,
        &services,
    )
    .expect("pull request publishes without task id");
}

#[test]
fn pull_request_task_id_flag_overrides_workspace_metadata() {
    // Verifies: Explicit PR task IDs win over workspace-local defaults.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    write_workspace_metadata(
        &workspace.path(),
        &WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
            project: None,
            parent: None,
        },
    )
    .expect("metadata writes");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        expected_task_id: Some(Some("XYZ-9".to_owned())),
        ..FakeServices::default()
    };

    run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@", "--task-id", "XYZ-9"],
        &environment,
        &services,
    )
    .expect("pull request publishes");
}

#[test]
fn pull_request_accepts_repeated_label_flags() {
    // Verifies: PR publishing applies each operator-supplied label once.
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
        expected_labels: vec!["bug".to_owned(), "help wanted".to_owned()],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        [
            "jx",
            "stack",
            "publish",
            "-r",
            "@",
            "--label",
            "bug",
            "-l",
            "help wanted",
            "-l",
            "bug",
        ],
        &environment,
        &services,
    )
    .expect("pull request publishes with labels");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_renders_updated_pr_url() {
    // Verifies: Pull request update success uses the same one-line URL format as creation.
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
        pull_request_action: PullRequestAction::Updated,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("pull request updates");

    assert_eq!(
        result.stdout,
        format!("Updated {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_falls_back_to_number_when_github_omits_url() {
    // Verifies: Pull request output remains concise when GitHub omits a PR URL.
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
        pull_request_url: None,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
    )
    .expect("pull request publishes without html url");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn stack_publish_revision_flag_plans_that_commit() {
    // Verifies: Stack publish accepts an explicit revision and plans the selected commit.
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
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "deadbeef", "-t", "ABC-123"],
        &environment,
        &services,
    )
    .expect("stack publish publishes selected commit");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_accepts_repeated_reviewer_flags() {
    // Verifies: Explicit reviewer flags become the requested reviewers and skip prompting.
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
        expected_reviewers: Some(ReviewerSelection {
            users: vec!["example-reviewer".to_owned()],
            teams: vec!["frontend".to_owned()],
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_reviewer_selector(
        [
            "jx",
            "stack",
            "publish",
            "-r",
            "@",
            "-R",
            "example-reviewer",
            "--reviewer",
            "ExampleOrg/frontend",
        ],
        &environment,
        &services,
        &CheckedReviewerSelector,
    )
    .expect("pull request publishes with explicit reviewers");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_respects_draft_flag() {
    // Verifies: The draft flag is carried into PR creation planning.
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
        expected_draft: Some(true),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "stack", "publish", "-r", "@", "--draft"],
        &environment,
        &services,
    )
    .expect("draft pull request publishes");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_reviewer_selection_can_be_cancelled() {
    // Verifies: Quitting reviewer selection cancels PR publishing before final confirmation.
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
    let services = FakeServices::default();

    let result = run_with_args_and_prompts(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
        &CancellingReviewerSelector,
        &AlwaysConfirmPullRequest,
    )
    .expect("pull request reviewer cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
}

#[test]
fn pull_request_can_be_cancelled_after_planning() {
    // Verifies: Declining the final confirmation stops before bookmark, push, or PR mutation.
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
    let services = FakeServices::default();
    let confirmer = FixedPullRequestConfirmer { confirmed: false };

    let result = run_with_args_and_prompts(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
        &SelectAllReviewers,
        &confirmer,
    )
    .expect("pull request cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
}

#[test]
fn yes_flag_confirms_pull_request_publish() {
    // Verifies: Batch confirmation mode proceeds through PR confirmations even without prompt input.
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
    let services = FakeServices::default();
    let confirmer = FixedPullRequestConfirmer { confirmed: false };

    let result = run_with_args_and_prompts(
        ["jx", "stack", "publish", "-r", "@", "--yes"],
        &environment,
        &services,
        &SelectAllReviewers,
        &confirmer,
    )
    .expect("yes flag confirms pull request publishing");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_rejects_invalid_cli_reviewer() {
    // Verifies: Reviewer flags use the same user/team shape as config reviewers.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices::default();

    let error = run_with_args_and_services(
        [
            "jx",
            "stack",
            "publish",
            "-r",
            "@",
            "--reviewer",
            "bad/reviewer/name",
        ],
        &environment,
        &services,
    )
    .expect_err("invalid reviewer is rejected during parsing");

    assert!(matches!(error, CommandError::Usage(_)));
}

#[test]
fn stack_publish_preselects_existing_and_cli_reviewers() {
    // Verifies: existing PR reviewers and CLI reviewers are selected by default for stack publish.
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
    let mut existing = existing_pull_request(false);
    existing.reviewers = ReviewerSelection::new(["existing-reviewer"], ["platform"]);
    let services = FakeServices {
        existing_pull_request: Some(existing),
        expected_reviewers: Some(ReviewerSelection::new(
            ["cli-reviewer", "existing-reviewer"],
            ["platform"],
        )),
        pull_request_action: PullRequestAction::Updated,
        ..FakeServices::default()
    };

    let result = run_with_args_and_reviewer_selector(
        [
            "jx",
            "stack",
            "publish",
            "-r",
            "@",
            "--reviewer",
            "cli-reviewer",
        ],
        &environment,
        &services,
        &CheckedReviewerSelector,
    )
    .expect("pull request publishes with existing reviewers selected");

    assert_eq!(
        result.stdout,
        format!("Updated {}\n", example_pull_request_link(42))
    );
}

#[test]
fn stack_publish_preselects_existing_review_activity() {
    // Verifies: prior reviewer activity stays checked so updating a PR can request another look.
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
        existing_pull_request: Some(existing_pull_request(false)),
        reviewer_candidates: vec![
            ReviewerCandidate::new(
                ReviewerTarget::user("approved-reviewer"),
                vec!["already approved".to_owned()],
            ),
            ReviewerCandidate::new(
                ReviewerTarget::user("commented-reviewer"),
                vec!["commented".to_owned()],
            ),
            ReviewerCandidate::new(
                ReviewerTarget::user("addressed-reviewer"),
                vec!["comments addressed".to_owned()],
            ),
            ReviewerCandidate::new(
                ReviewerTarget::user("suggested-reviewer"),
                vec!["suggested by GitHub".to_owned()],
            ),
        ],
        expected_reviewers: Some(ReviewerSelection::new(
            [
                "addressed-reviewer",
                "approved-reviewer",
                "commented-reviewer",
            ],
            Vec::<String>::new(),
        )),
        pull_request_action: PullRequestAction::Updated,
        ..FakeServices::default()
    };

    let result = run_with_args_and_reviewer_selector(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
        &CheckedReviewerSelector,
    )
    .expect("pull request publishes with active reviewer selected");

    assert_eq!(
        result.stdout,
        format!("Updated {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_uses_interactively_selected_reviewers() {
    // Verifies: PR publishing syncs only the configured reviewers selected by the operator.
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
    let selected = ReviewerSelection::new(["second-reviewer"], std::iter::empty::<&str>());
    let services = FakeServices {
        reviewer_candidates: vec![
            ReviewerCandidate::new(
                ReviewerTarget::user("example-reviewer"),
                vec!["global".to_owned()],
            ),
            ReviewerCandidate::new(
                ReviewerTarget::user("second-reviewer"),
                vec!["src/** matched 1 file".to_owned()],
            ),
        ],
        expected_reviewers: Some(selected.clone()),
        ..FakeServices::default()
    };
    let selector = FixedReviewerSelector { selected };

    let result = run_with_args_and_reviewer_selector(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
        &selector,
    )
    .expect("pull request publishes");

    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}

#[test]
fn pull_request_previews_next_to_final_confirmation() {
    // Verifies: PR previews stay adjacent to the confirmation prompt they describe.
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
    let selected = ReviewerSelection::new(["second-reviewer"], std::iter::empty::<&str>());
    let services = FakeServices {
        reviewer_candidates: vec![ReviewerCandidate::new(
            ReviewerTarget::user("second-reviewer"),
            vec!["src/** matched 1 file".to_owned()],
        )],
        expected_reviewers: Some(selected.clone()),
        ..FakeServices::default()
    };
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let previewer = RecordingPullRequestPreviewer {
        events: events.clone(),
    };
    let reviewer_selector = RecordingReviewerSelector {
        events: events.clone(),
        selected,
    };
    let prompts = PromptHandlers {
        pull_request_previewer: &previewer,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &reviewer_selector,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };

    let result = run_with_args_and_progress(
        ["jx", "stack", "publish", "-r", "@"],
        &environment,
        &services,
        &NoProgress,
        prompts,
        OutputMode::plain(),
    )
    .expect("pull request publishes");

    assert_eq!(events.borrow().as_slice(), &["reviewers", "preview"]);
    assert_eq!(
        result.stdout,
        format!("Created {}\n", example_pull_request_link(42))
    );
}
