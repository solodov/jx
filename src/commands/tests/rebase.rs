use super::*;

#[test]
fn rebase_on_trunk_rebases_current_by_default() {
    // Verifies: Rebase-on-trunk defaults to rebasing the working-copy change.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        expected_rebase_sources: Some(Vec::new()),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "rebase-on-trunk"], &environment, &services)
        .expect("rebase-on-trunk succeeds");

    assert_eq!(
            result.stdout,
            "Rebased: a1b2c3d4 onto origin/main (\x1b]8;;https://github.com/example-owner/example-repo/tree/main\x1b\\ssh://git@github.com/example-owner/example-repo.git\x1b]8;;\x1b\\), rebased 2 commits\n"
        );
}

#[test]
fn rt_alias_accepts_source_flag() {
    // Verifies: The rt alias can rebase a specific source revision and descendants.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        expected_rebase_sources: Some(vec!["deadbeef".to_owned()]),
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "rt", "-s", "deadbeef"], &environment, &services)
            .expect("rt succeeds");

    assert!(result
        .stdout
        .starts_with("Rebased: a1b2c3d4 onto origin/main "));
}

#[test]
fn rt_alias_accepts_repeated_source_flags() {
    // Verifies: The rt alias forwards repeated source revisions as one rebase operation.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        expected_rebase_sources: Some(vec!["aaaabbbb".to_owned(), "ccccdddd".to_owned()]),
        rebase_on_trunk: RebaseOnTrunkOutcome {
            source_short_commit_ids: vec!["aaaabbbb".to_owned(), "ccccdddd".to_owned()],
            ..FakeServices::default().rebase_on_trunk
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "rt", "-s", "aaaabbbb", "--source", "ccccdddd"],
        &environment,
        &services,
    )
    .expect("rt succeeds");

    assert!(result
        .stdout
        .starts_with("Rebased: 2 sources onto origin/main "));
}

#[test]
fn rebase_on_trunk_renders_up_to_date_when_no_commits_move() {
    // Verifies: Rebase-on-trunk says up to date when the source already sits on trunk.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices {
        rebase_on_trunk: RebaseOnTrunkOutcome {
            rebased_commits: 0,
            skipped_commits: 1,
            current_updated: false,
            ..FakeServices::default().rebase_on_trunk
        },
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "rebase-on-trunk"], &environment, &services)
        .expect("rebase-on-trunk succeeds");

    assert!(result.stdout.ends_with(", up to date\n"));
}
