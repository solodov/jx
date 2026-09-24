use super::*;

#[test]
fn publish_and_pub_help_explain_ready_only_selection_and_the_draft_exception() {
    for command in ["publish", "pub"] {
        let help = cli()
            .try_get_matches_from(["jx", "stack", command, "--help"])
            .expect_err("help prints instead of running publish")
            .to_string();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(help.contains("Reviewer selection applies only to ready PRs"));
        assert!(help.contains("after readiness overrides"));
        assert!(help.contains("Drafts retain their reviewers"));
        assert!(help.contains("an all-draft selection skips the picker"));
        assert!(help.contains("jx stack pub -r REVISION -R alice"));
        assert!(help.contains("select it explicitly without --apply-to-stack"));
    }
}

#[test]
fn mixed_stack_selection_and_flags_only_apply_to_ready_prs() {
    for with_anchor in [false, true] {
        let workspace = reviewer_workspace();
        let environment = RuntimeEnvironment::new(workspace.path(), []);
        let services = reviewer_stack_services(&[false, true, false]);
        let mut args = vec!["jx", "stack", "pub", "-A", "-R", "alice"];
        if with_anchor {
            args.extend(["-r", "@"]);
        }
        run_with_args_and_reviewer_selector(
            args,
            &environment,
            &services,
            &CheckedReviewerSelector,
        )
        .expect("mixed stack publishes");

        let plans = services.published_plans.borrow();
        assert_eq!(plans.len(), 3);
        let ready_reviewers =
            ReviewerSelection::new(["alice", "existing-0", "existing-2"], ["team-0", "team-2"]);
        assert_eq!(plans[0].reviewers, ready_reviewers);
        assert_eq!(plans[2].reviewers, ready_reviewers);
        assert!(plans[1].reviewers.is_empty());
        assert_eq!(
            plans[1].effective_reviewers(),
            &ReviewerSelection::new(["existing-1"], ["team-1"]),
        );
    }
}

#[test]
fn all_draft_stack_skips_picker_even_with_stack_wide_reviewer_flags() {
    let workspace = reviewer_workspace();
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = reviewer_stack_services(&[true, true]);
    run_with_args_and_reviewer_selector(
        ["jx", "stack", "pub", "-A", "-r", "@", "-R", "alice"],
        &environment,
        &services,
        &CancellingReviewerSelector,
    )
    .expect("all-draft stack publishes without invoking the picker");

    let plans = services.published_plans.borrow();
    assert_eq!(plans.len(), 2);
    assert!(plans.iter().all(|plan| plan.reviewers.is_empty()));
    assert!(plans
        .iter()
        .all(|plan| !plan.effective_reviewers().is_empty()));
}

#[test]
fn draft_reviewer_opt_in_requires_an_explicit_single_revision_without_apply_to_stack() {
    for (flags, explicit) in [
        (vec![], false),
        (vec!["-R", "alice"], false),
        (vec!["-r", "@"], false),
        (vec!["-A", "-r", "@", "-R", "alice"], false),
        (
            vec!["-r", "@", "-R", "alice", "-R", "ExampleOrg/frontend"],
            true,
        ),
    ] {
        let workspace = reviewer_workspace();
        let environment = RuntimeEnvironment::new(workspace.path(), []);
        let services = reviewer_stack_services(&[true]);
        let mut args = vec!["jx", "stack", "pub"];
        args.extend(flags);
        run_with_args_and_reviewer_selector(
            args,
            &environment,
            &services,
            &CancellingReviewerSelector,
        )
        .expect("draft publishes without invoking the picker");

        let plans = services.published_plans.borrow();
        assert_eq!(plans.len(), 1);
        if explicit {
            assert_eq!(
                plans[0].reviewers,
                ReviewerSelection::new(["alice", "existing-0"], ["frontend", "team-0"]),
            );
        } else {
            assert!(plans[0].reviewers.is_empty());
        }
        assert!(plans[0]
            .effective_reviewers()
            .users
            .contains(&"existing-0".to_owned()));
        assert!(plans[0]
            .effective_reviewers()
            .teams
            .contains(&"team-0".to_owned()));
    }
}

#[test]
fn new_drafts_ignore_candidates_unless_explicitly_targeted_with_reviewers() {
    for explicit in [false, true] {
        let workspace = reviewer_workspace();
        let environment = RuntimeEnvironment::new(workspace.path(), []);
        let services = FakeServices {
            reviewer_candidates: vec![ReviewerCandidate::new(
                ReviewerTarget::user("automatic-reviewer"),
                vec!["matched 1 file".to_owned()],
            )],
            ..FakeServices::default()
        };
        let mut args = vec!["jx", "stack", "pub", "-r", "@", "--draft"];
        if explicit {
            args.extend(["-R", "alice"]);
        }
        run_with_args_and_reviewer_selector(
            args,
            &environment,
            &services,
            &CancellingReviewerSelector,
        )
        .expect("new draft publishes without the picker");

        let plans = services.published_plans.borrow();
        assert_eq!(plans.len(), 1);
        assert!(plans[0].draft);
        let expected = if explicit {
            ReviewerSelection::new(["alice"], Vec::<String>::new())
        } else {
            ReviewerSelection::default()
        };
        assert_eq!(plans[0].reviewers, expected);
    }
}

#[test]
fn final_readiness_controls_reviewer_selection() {
    for (initial_draft, readiness_flag, expected_draft) in
        [(true, "--ready", false), (false, "--draft", true)]
    {
        let workspace = reviewer_workspace();
        let environment = RuntimeEnvironment::new(workspace.path(), []);
        let services = reviewer_stack_services(&[initial_draft]);
        run_with_args_and_reviewer_selector(
            ["jx", "stack", "pub", "-A", "-R", "alice", readiness_flag],
            &environment,
            &services,
            &CheckedReviewerSelector,
        )
        .expect("readiness override publishes");

        let plans = services.published_plans.borrow();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].draft, expected_draft);
        assert_eq!(plans[0].reviewers.is_empty(), expected_draft);
        if !expected_draft {
            assert!(plans[0].reviewers.users.contains(&"alice".to_owned()));
        }
    }
}

#[test]
fn metadata_only_draft_updates_use_the_same_opt_in_rules() {
    for explicit in [false, true] {
        let workspace = reviewer_workspace();
        let environment = RuntimeEnvironment::new(workspace.path(), []);
        let mut services = reviewer_stack_services(&[true]);
        services.push.pushed_refs = 0;
        services.push.pushed_commits.clear();
        let mut args = vec!["jx", "stack", "pub", "-r", "@"];
        if explicit {
            args.extend(["-R", "alice"]);
        }
        run_with_args_and_reviewer_selector(
            args,
            &environment,
            &services,
            &CancellingReviewerSelector,
        )
        .expect("draft metadata updates without the picker");

        assert!(services.published_plans.borrow().is_empty());
        let plans = services.metadata_only_plans.borrow();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].reviewers.is_empty(), !explicit);
        assert!(plans[0]
            .effective_reviewers()
            .users
            .contains(&"existing-0".to_owned()));
    }
}

fn reviewer_workspace() -> TestWorkspace {
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        "[remote \"origin\"]\n    url = ssh://git@github.com/example-owner/example-repo.git\n",
    );
    workspace
}

fn reviewer_stack_services(drafts: &[bool]) -> FakeServices {
    let mut nodes = Vec::new();
    let mut pull_requests_by_head = BTreeMap::new();
    let mut parent_branch = "main".to_owned();
    for (index, draft) in drafts.iter().enumerate() {
        let change_id = format!("{:08}", index + 1);
        let branch = format!("example-user/{index:02}-{change_id}");
        let mut workspace = workspace_facts();
        workspace.target_change.change_id = change_id;
        workspace.target_change.commit_id = format!("commit-{index}");
        workspace.target_change.description = format!("Change {index}");
        workspace.nearest_ancestor_bookmark = None;
        workspace.stack_index = index;
        let mut pull_request = pull_request_choice_record(
            42 + index as u64,
            &workspace.target_change.description,
            &branch,
            &parent_branch,
            *draft,
        );
        pull_request.reviewers =
            ReviewerSelection::new([format!("existing-{index}")], [format!("team-{index}")]);
        pull_requests_by_head.insert(branch.clone(), pull_request);
        nodes.push(crate::jj::StackPublishNodeFacts {
            workspace,
            parent_index: index.checked_sub(1),
        });
        parent_branch = branch;
    }
    FakeServices {
        stack_publish_facts: Some(StackPublishFacts {
            nodes,
            publish_indexes: (0..drafts.len()).collect(),
            anchor_index: drafts.len().checked_sub(1),
            metrics: StackPublishMetrics::default(),
        }),
        pull_requests_by_head,
        pull_request_action: PullRequestAction::Updated,
        ..FakeServices::default()
    }
}
