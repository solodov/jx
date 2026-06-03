use super::*;

#[test]
fn push_reuses_existing_bookmark_on_current_change() {
    // Verifies: Push uses the current change and existing bookmark by default.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let mut workspace_facts = workspace_facts();
    workspace_facts.local_bookmarks = vec!["example-user/current".to_owned()];
    workspace_facts.local_bookmarks_at_target = workspace_facts.local_bookmarks.clone();
    let services = FakeServices {
        workspace: workspace_facts,
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "push"], &environment, &services).expect("push succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Pushed: {} -> a1b2c3d4\n",
            example_bookmark_link("example-user/current")
        )
    );
}

#[test]
fn push_revision_creates_generated_bookmark_after_confirmation() {
    // Verifies: Push can target a selected revision and create the displayed bookmark.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result =
        run_with_args_and_services(["jx", "push", "-r", "deadbeef"], &environment, &services)
            .expect("push succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Pushed: {} -> deadbeef (created bookmark)\n",
            example_bookmark_link("push-zzzzzzzz")
        )
    );
}

#[test]
fn push_can_be_cancelled_before_creating_generated_bookmark() {
    // Verifies: Declining generated bookmark creation stops before push mutation.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();
    let confirmer = FixedPushConfirmer { confirmed: false };

    let result =
        run_with_args_and_push_confirmer(["jx", "push"], &environment, &services, &confirmer)
            .expect("push cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
}

#[test]
fn yes_flag_confirms_push_bookmark_creation() {
    // Verifies: Batch confirmation mode proceeds through push confirmation prompts.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();
    let confirmer = FixedPushConfirmer { confirmed: false };

    let result = run_with_args_and_push_confirmer(
        ["jx", "push", "--yes"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("yes flag confirms push");

    assert_eq!(
        result.stdout,
        format!(
            "Pushed: {} -> a1b2c3d4 (created bookmark)\n",
            example_bookmark_link("push-zzzzzzzz")
        )
    );
}

#[test]
fn push_tracked_pushes_tracked_bookmarks_and_deletions() {
    // Verifies: Tracked push reports both moved and deleted tracked bookmarks.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "push", "--tracked"], &environment, &services)
        .expect("tracked push succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Pushed tracked bookmarks:\n  {}: 11112222 -> a1b2c3d4\n  {}: deleted from 99990000\n",
            example_bookmark_link("example-user/current"),
            example_bookmark_link("example-user/old")
        )
    );
}
