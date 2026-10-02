use super::*;

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
