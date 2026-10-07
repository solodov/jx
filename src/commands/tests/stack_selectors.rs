use super::*;

#[test]
fn publish_help_explains_current_change_default_and_full_stack_opt_in() {
    for command in ["publish", "pub"] {
        let help = cli()
            .try_get_matches_from(["jx", "stack", command, "--help"])
            .expect_err("help prints instead of running publish")
            .to_string();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(help.contains("Publish the working-copy change by default"));
        assert!(help.contains("Use --apply-to-stack to publish the full linear stack"));
    }
}

#[test]
fn default_publish_matches_explicit_current_selection_with_unpublished_stack_context() {
    // Verifies: accepting existing reviewers publishes only the current PR, not its unpublished relatives.
    let mut expected_output = None;
    for selector in [None, Some("@"), Some("bbbbbbbb"), Some("42")] {
        let workspace = TestWorkspace::new();
        let branch = "example-user/01-bbbbbbbb";
        selector_context(&workspace, vec![selector_node(42, branch)]);
        let environment = RuntimeEnvironment::new(workspace.path(), []);
        let nodes = ["aaaaaaaa", "bbbbbbbb", "cccccccc"]
            .into_iter()
            .enumerate()
            .map(|(index, change_id)| {
                let mut workspace = workspace_facts();
                workspace.target_change.change_id = change_id.to_owned();
                workspace.target_change.commit_id = format!("commit-{index}");
                workspace.target_change.description = format!("Change {index}");
                workspace.nearest_ancestor_bookmark = None;
                workspace.stack_index = index;
                crate::jj::StackPublishNodeFacts {
                    workspace,
                    parent_index: index.checked_sub(1),
                }
            })
            .collect::<Vec<_>>();
        let inferred = StackPublishFacts {
            nodes,
            publish_indexes: vec![0, 1, 2],
            anchor_index: Some(1),
            metrics: StackPublishMetrics::default(),
        };
        let mut selected = inferred.clone();
        selected.nodes.truncate(2);
        selected.publish_indexes = vec![1];
        selected.anchor_index = None;
        let mut pull_request = pull_request_choice_record(42, "Change 1", branch, "main", false);
        pull_request.reviewers = ReviewerSelection::new(["alice", "bob"], ["platform"]);
        let services = FakeServices {
            stack_publish_facts: Some(inferred),
            stack_publish_facts_by_revision: ["@", "bbbbbbbb", "commit-1"]
                .into_iter()
                .map(|revision| (revision.to_owned(), selected.clone()))
                .collect(),
            local_bookmark_commit_ids: BTreeMap::from([(branch.to_owned(), "commit-1".to_owned())]),
            pull_requests_by_head: BTreeMap::from([(branch.to_owned(), pull_request.clone())]),
            pull_request_action: PullRequestAction::Updated,
            ..FakeServices::default()
        };
        let confirmer = SequencePullRequestConfirmer::new([true]);
        let mut args = vec!["jx", "stack", "pub"];
        if let Some(selector) = selector {
            args.extend(["-r", selector]);
        }

        let result = run_with_args_and_prompts(
            args,
            &environment,
            &services,
            &CheckedReviewerSelector,
            &confirmer,
        )
        .expect("current PR publishes without publishing its relatives");

        assert_eq!(result.exit_code, 0);
        assert!(result
            .stdout
            .contains(&format!("Updated {}", example_pull_request_link(42))));
        if let Some(expected) = &expected_output {
            assert_eq!(&result.stdout, expected);
        } else {
            expected_output = Some(result.stdout);
        }
        assert_eq!(confirmer.titles.borrow().as_slice(), ["Change 1"]);
        assert_eq!(services.push_bookmark_calls.borrow().as_slice(), [branch]);
        assert_eq!(services.published_pull_request_count.get(), 1);
        let plans = services.published_plans.borrow();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].target_commit_id, "commit-1");
        assert_eq!(plans[0].base, "main");
        assert_eq!(plans[0].reviewers, pull_request.reviewers);
        assert_eq!(plans[0].existing_pull_request.as_ref().unwrap().number, 42);
        let revision = match selector {
            None => "@",
            Some("42") => "commit-1",
            Some(revision) => revision,
        };
        assert_eq!(
            services.stack_publish_selections.borrow().first(),
            Some(&StackPublishSelection::ExplicitRevisions {
                revisions: vec![revision.to_owned()],
            }),
        );
    }
}

#[test]
fn cached_pr_numbers_select_current_local_commits_and_preserve_other_revisions() {
    let workspace = TestWorkspace::new();
    let context = selector_context(&workspace, vec![selector_node(305743, "topic/current")]);
    let services = FakeServices {
        local_bookmark_commit_ids: BTreeMap::from([(
            "topic/current".to_owned(),
            "feedfacecafebeef".to_owned(),
        )]),
        ..FakeServices::default()
    };

    let resolved = resolve_stack_publish_revisions(
        &context,
        &services,
        &["305743", "@", "topic/other", "305744", "305743::"].map(str::to_owned),
    )
    .unwrap();

    assert_eq!(
        resolved,
        ["feedfacecafebeef", "@", "topic/other", "305744", "305743::"],
    );
    assert_eq!(
        services.local_bookmark_commit_requests.borrow().as_slice(),
        ["topic/current"],
    );
    assert!(services.pull_request_number_calls.borrow().is_empty());
}

#[test]
fn unknown_numbers_and_numeric_commit_prefixes_remain_jj_selectors() {
    let workspace = TestWorkspace::new();
    let context = selector_context(&workspace, Vec::new());
    let services = FakeServices::default();
    let revisions =
        ["123456", "012345", "18446744073709551616", "deadbeef", "@"].map(str::to_owned);

    assert_eq!(
        resolve_stack_publish_revisions(&context, &services, &revisions).unwrap(),
        revisions,
    );
    assert!(services.local_bookmark_commit_requests.borrow().is_empty());
}

#[test]
fn cached_pr_without_exact_local_bookmark_does_not_fall_back_to_a_commit_or_fragment() {
    let workspace = TestWorkspace::new();
    let context = selector_context(&workspace, vec![selector_node(123456, "topic/current")]);
    let services = FakeServices {
        local_bookmark_commit_ids: BTreeMap::from([(
            "other/topic/current".to_owned(),
            "feedfacecafebeef".to_owned(),
        )]),
        ..FakeServices::default()
    };

    let error =
        resolve_stack_publish_revisions(&context, &services, &["123456".to_owned()]).unwrap_err();

    assert!(matches!(
        error,
        CommandError::Jj(JjError::MissingLocalBookmark { branch }) if branch == "topic/current"
    ));
}

#[test]
fn cached_pr_with_multiple_bookmarks_requires_an_explicit_selection() {
    let workspace = TestWorkspace::new();
    let context = selector_context(
        &workspace,
        vec![
            selector_node(42, "topic/one"),
            selector_node(42, "topic/two"),
        ],
    );
    let services = FakeServices::default();

    let error =
        resolve_stack_publish_revisions(&context, &services, &["42".to_owned()]).unwrap_err();

    assert!(error
        .to_string()
        .contains("PR 42 is associated with multiple local bookmarks"));
    assert!(services.local_bookmark_commit_requests.borrow().is_empty());
}

#[test]
fn publish_pr_aliases_work_for_exact_selection_stack_anchors_and_readiness() {
    for apply_to_stack in [false, true] {
        let workspace = TestWorkspace::new();
        selector_context(&workspace, vec![selector_node(42, "topic/current")]);
        let environment = RuntimeEnvironment::new(
            workspace.path(),
            [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
        );
        let commit_id = workspace_facts().target_change.commit_id;
        let services = FakeServices {
            local_bookmark_commit_ids: BTreeMap::from([(
                "topic/current".to_owned(),
                commit_id.clone(),
            )]),
            expected_draft: Some(false),
            ..FakeServices::default()
        };
        let mut args = vec!["jx", "stack", "pub", "-r", "42", "--ready=42"];
        if apply_to_stack {
            args.push("--apply-to-stack");
        }

        let result = run_with_args_and_services(args, &environment, &services).unwrap();

        assert_eq!(result.exit_code, 0);
        let expected = if apply_to_stack {
            StackPublishSelection::InferredStack {
                anchor: Some(commit_id.clone()),
            }
        } else {
            StackPublishSelection::ExplicitRevisions {
                revisions: vec![commit_id.clone()],
            }
        };
        assert_eq!(
            services.stack_publish_selections.borrow().first(),
            Some(&expected)
        );
        assert!(services.stack_publish_selections.borrow().contains(
            &StackPublishSelection::ExplicitRevisions {
                revisions: vec![commit_id],
            },
        ));
        assert!(!services.published_plans.borrow().is_empty());
    }
}

fn selector_context(workspace: &TestWorkspace, nodes: Vec<StackMetadataNode>) -> RepositoryContext {
    workspace.write_git_config(
        "[remote \"origin\"]\n    url = https://github.com/example-owner/example-repo.git\n",
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            nodes,
            ..StackMetadata::default()
        },
    )
    .unwrap();
    RepositoryContext::discover(&RuntimeEnvironment::new(workspace.path(), [])).unwrap()
}

fn selector_node(number: u64, branch: &str) -> StackMetadataNode {
    StackMetadataNode {
        branch: branch.to_owned(),
        base_branch: "main".to_owned(),
        parent_branch: None,
        pull_request: Some(number),
        parent_pull_request: None,
        title: "Current change".to_owned(),
        url: None,
        draft: false,
        merged: false,
        work_ids: Vec::new(),
        fixes_work_ids: Vec::new(),
    }
}
