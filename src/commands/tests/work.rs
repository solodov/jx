use super::*;

#[test]
fn work_add_uses_hidden_layout_workspace_path() {
    // Verifies: Add creates jj workspaces under the configured hidden `.work` layout.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: Some("main".to_owned()),
            shared_paths: Vec::new(),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "add", "fix", "-r", "main"],
        &environment,
        &services,
    )
    .expect("workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!("Added workspace: {}\n", expected_destination.display())
    );
}

#[test]
fn work_add_from_managed_workspace_invokes_jj_from_primary_checkout() {
    // Verifies: Workspace add planning runs jj from the primary checkout, not the current managed workspace.
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
    let primary = workspace.home.join("projects/jx");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let services = FakeServices {
        expected_workspace_add_current_dir: Some(primary),
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: Vec::new(),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "work", "add", "fix"], &environment, &services)
        .expect("workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!("Added workspace: {}\n", expected_destination.display())
    );
}

#[test]
fn work_add_plan_splits_shared_path_sources_from_primary_checkout() {
    // Verifies: Existing shared paths are linked from the primary checkout and missing paths are skipped.
    let workspace = TestWorkspace::new_under("projects/.work/jx/current");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = [".pi", "nested/state", "missing/state"]
"#,
    );
    workspace.write_home_file("projects/jx/.pi/config.toml", "pi state");
    workspace.write_home_file("projects/jx/nested/state", "nested state");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let context = LocalRepositoryContext::discover(&environment).expect("context discovers");
    let request = WorkAddRequest {
        name: "fix".to_owned(),
        revision: Some("main".to_owned()),
        task_id: Some("ABC-123".to_owned()),
        shell_cd_target: false,
    };
    let primary = workspace.home.join("projects/jx");
    let destination = workspace.home.join("projects/.work/jx/ABC-123-fix");

    let plan = plan_work_add(&request, &context, &environment).expect("plan builds");

    assert_eq!(
        plan.identity,
        RepositoryIdentity {
            source: "github".to_owned(),
            host: "github.com".to_owned(),
            owner: "example-owner".to_owned(),
            repo: "jx".to_owned(),
        }
    );
    assert_eq!(plan.primary_checkout_root, primary);
    assert_eq!(plan.destination, destination);
    assert_eq!(plan.workspace_name, "ABC-123-fix");
    assert_eq!(plan.revision.as_deref(), Some("main"));
    assert_eq!(plan.task_id.as_deref(), Some("ABC-123"));
    assert_eq!(
        plan.shared_paths.effective_paths,
        vec![".pi", "nested/state", "missing/state"]
    );
    assert_eq!(
        plan.shared_paths.link_candidates,
        vec![
            SharedWorkspacePathCandidate {
                relative_path: ".pi".to_owned(),
                source: primary.join(".pi"),
                destination: destination.join(".pi"),
            },
            SharedWorkspacePathCandidate {
                relative_path: "nested/state".to_owned(),
                source: primary.join("nested/state"),
                destination: destination.join("nested/state"),
            },
        ]
    );
    assert_eq!(
        plan.shared_paths.missing_sources,
        vec![MissingSharedWorkspacePath {
            relative_path: "missing/state".to_owned(),
            source: primary.join("missing/state"),
        }]
    );
    assert_eq!(
        plan.workspace_options().shared_paths,
        vec![".pi".to_owned(), "nested/state".to_owned()]
    );
}

#[test]
fn work_add_shell_cd_target_prints_new_workspace_path() {
    // Verifies: Shell integration can enter a newly added managed workspace after creation.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: Vec::new(),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "add", "--shell-cd-target", "fix"],
        &environment,
        &services,
    )
    .expect("workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Added workspace: {}\n{}{}\n",
            expected_destination.display(),
            SHELL_CD_TARGET_PREFIX,
            expected_destination.display()
        )
    );
}

#[test]
fn work_add_creation_failure_stops_before_metadata_setup() {
    // Verifies: If jj workspace creation fails, post-create setup is not attempted.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/ABC-123-fix");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "ABC-123-fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: Vec::new(),
        }),
        workspace_add_error: Some("simulated creation failure".to_owned()),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(
        ["jx", "work", "add", "fix", "--task-id", "ABC-123"],
        &environment,
        &services,
    )
    .expect_err("workspace add fails");

    assert!(matches!(
        error,
        CommandError::Jj(JjError::WorkspaceAddFailed { status }) if status == "simulated creation failure"
    ));
    assert!(!expected_destination.join(".jx/workspace.toml").exists());
}

#[test]
fn work_add_post_create_setup_failure_reports_without_rollback() {
    // Verifies: Setup failures after jj creation surface an error and leave the created workspace in place.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/ABC-123-fix");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "ABC-123-fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: Vec::new(),
        }),
        workspace_add_metadata_blocker: Some(expected_destination.clone()),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(
        ["jx", "work", "add", "fix", "--task-id", "ABC-123"],
        &environment,
        &services,
    )
    .expect_err("post-create setup fails");

    assert!(matches!(
        error,
        CommandError::WorkAddSetup { ref workspace, ref destination, ref message }
            if workspace == "ABC-123-fix"
                && destination == &expected_destination
                && message.contains("Could not write workspace metadata")
    ));
    assert!(expected_destination.is_dir());
    assert!(expected_destination.join(".jx").is_file());
}

#[test]
fn work_add_links_shared_paths_and_creates_nested_parents() {
    // Verifies: Existing shared-path candidates are symlinked into the created workspace.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = [".pi", "nested/state", ".local-link", "missing/state"]
"#,
    );
    workspace.write_file(".pi/settings.toml", "pi state");
    workspace.write_file("nested/state", "nested state");
    workspace.write_file("real-target", "real state");
    std::os::unix::fs::symlink(
        workspace.path().join("real-target"),
        workspace.path().join(".local-link"),
    )
    .expect("create source symlink");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: vec![
                ".pi".to_owned(),
                "nested/state".to_owned(),
                ".local-link".to_owned(),
            ],
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "work", "add", "fix"], &environment, &services)
        .expect("workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!("Added workspace: {}\n", expected_destination.display())
    );
    assert_eq!(
        fs::read_link(expected_destination.join(".pi")).expect(".pi symlink"),
        workspace.path().join(".pi")
    );
    assert_eq!(
        fs::read_link(expected_destination.join("nested/state")).expect("nested symlink"),
        workspace.path().join("nested/state")
    );
    assert_eq!(
        fs::read_link(expected_destination.join(".local-link")).expect("source symlink linked"),
        workspace.path().join(".local-link")
    );
    assert!(expected_destination.join("nested").is_dir());
    assert!(!expected_destination.join("missing/state").exists());
}

#[test]
fn work_add_shared_path_setup_reports_blocked_nested_parent() {
    // Verifies: Nested shared-path parent creation reports filesystem blockers clearly.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = ["nested/state"]
"#,
    );
    workspace.write_file("nested/state", "nested state");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let parent_blocker = expected_destination.join("nested");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: vec!["nested/state".to_owned()],
        }),
        workspace_add_existing_shared_path: Some(parent_blocker.clone()),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "work", "add", "fix"], &environment, &services)
        .expect_err("shared path setup fails");

    assert!(matches!(
        error,
        CommandError::WorkAddSetup { ref workspace, ref destination, ref message }
            if workspace == "fix"
                && destination == &expected_destination
                && message.contains("create parent directories")
                && message.contains("nested/state")
    ));
    assert_eq!(
        fs::read_to_string(&parent_blocker).expect("parent blocker remains"),
        "existing content"
    );
}

#[test]
fn work_add_shared_path_setup_failure_keeps_workspace_and_existing_content() {
    // Verifies: Shared-path setup fails conservatively without rollback or overwrite.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = [".pi"]
"#,
    );
    workspace.write_file(".pi/settings.toml", "pi state");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let existing_shared_path = expected_destination.join(".pi");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: vec![".pi".to_owned()],
        }),
        workspace_add_existing_shared_path: Some(existing_shared_path.clone()),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(["jx", "work", "add", "fix"], &environment, &services)
        .expect_err("shared path setup fails");

    assert!(matches!(
        error,
        CommandError::WorkAddSetup { ref workspace, ref destination, ref message }
            if workspace == "fix"
                && destination == &expected_destination
                && message.contains("destination already exists")
    ));
    assert!(expected_destination.is_dir());
    assert_eq!(
        fs::read_to_string(&existing_shared_path).expect("existing content remains"),
        "existing content"
    );
}

#[test]
fn work_add_shell_cd_target_setup_failure_returns_error_without_cd_target() {
    // Verifies: Shell integration receives no hidden cd target when post-create setup fails.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[repo]
workspace_shared_paths = [".pi"]
"#,
    );
    workspace.write_file(".pi/settings.toml", "pi state");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/fix");
    let existing_shared_path = expected_destination.join(".pi");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: vec![".pi".to_owned()],
        }),
        workspace_add_existing_shared_path: Some(existing_shared_path),
        ..FakeServices::default()
    };

    let error = run_with_args_and_services(
        ["jx", "work", "add", "--shell-cd-target", "fix"],
        &environment,
        &services,
    )
    .expect_err("shared path setup fails");

    assert!(matches!(
        error,
        CommandError::WorkAddSetup { ref workspace, ref destination, .. }
            if workspace == "fix" && destination == &expected_destination
    ));
    assert!(!error.to_string().contains(SHELL_CD_TARGET_PREFIX));
}

#[test]
fn work_add_task_id_prefixes_workspace_name_and_writes_metadata() {
    // Verifies: Task workspaces use task-visible names while metadata remains the source of truth.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.home.join("projects/.work/jx/ABC-123-fix");
    let services = FakeServices {
        expected_workspace_add: Some(WorkspaceAddOptions {
            name: "ABC-123-fix".to_owned(),
            destination: expected_destination.clone(),
            revision: None,
            shared_paths: Vec::new(),
        }),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "add", "fix", "--task-id", "ABC-123"],
        &environment,
        &services,
    )
    .expect("task workspace add succeeds");

    assert_eq!(
        result.stdout,
        format!("Added workspace: {}\n", expected_destination.display())
    );
    assert_eq!(
        read_workspace_metadata(&expected_destination).expect("metadata reads"),
        WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
        }
    );
    assert_eq!(
        fs::read_to_string(expected_destination.join(".jx/.gitignore")).expect("gitignore"),
        "/.gitignore\n/workspace.toml\n/stack.toml\n"
    );
}

#[test]
fn work_add_rejects_invalid_workspace_name() {
    // Verifies: Workspace names are validated before they become filesystem paths.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error =
        run_with_args_and_services(["jx", "work", "add", "bad/name"], &environment, &services)
            .expect_err("invalid workspace names are rejected");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::InvalidWorkspaceName { .. })
    ));
}

#[test]
fn work_list_marks_current_workspace_and_aligns_paths() {
    // Verifies: List renders jj workspaces as concise name/path rows with `@` on current.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "default".to_owned(),
                root: PathBuf::from("/Users/example/projects/jx"),
                is_current: true,
            },
            WorkspaceEntry {
                name: "fix".to_owned(),
                root: PathBuf::from("/Users/example/projects/.work/jx/fix"),
                is_current: false,
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "work", "list"], &environment, &services)
        .expect("workspace list succeeds");

    assert_eq!(
        result.stdout,
        "default@  /Users/example/projects/jx\nfix       /Users/example/projects/.work/jx/fix\n"
    );
}

#[test]
fn work_complete_lists_global_repositories_and_workspaces() {
    // Verifies: Completion scans configured layout roots and orders each repo before workspaces.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let project_root = workspace.home.join("projects/project");
    create_jj_workspace_marker(&project_root);
    create_jj_workspace_marker(&workspace.home.join("projects/.work/project/fix"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/project/review"));
    create_jj_workspace_marker(&workspace.home.join("projects/other"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--prefix", "p"],
        &environment,
        &services,
    )
    .expect("work completion succeeds");

    assert_eq!(result.stdout, "project\nproject@fix\nproject@review\n");
}

#[test]
fn work_complete_picker_format_includes_keys_and_paths() {
    // Verifies: fzf-backed shell completion can display a key with its resolved target path.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let project_root = workspace.home.join("projects/project");
    let fix_root = workspace.home.join("projects/.work/project/fix");
    create_jj_workspace_marker(&project_root);
    create_jj_workspace_marker(&fix_root);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        [
            "jx", "work", "complete", "--format", "picker", "--prefix", "p",
        ],
        &environment,
        &services,
    )
    .expect("work picker completion succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "project\t{}\nproject@fix\t{}\n",
            project_root.display(),
            fix_root.display()
        )
    );
}

#[test]
fn work_complete_workspaces_lists_only_deletable_workspace_names() {
    // Verifies: Delete completion offers managed workspaces but omits the primary default checkout.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let mut workspaces = project_workspaces(&workspace);
    workspaces.push(WorkspaceEntry {
        name: "review".to_owned(),
        root: workspace.home.join("projects/.work/jx/review"),
        is_current: false,
    });
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces,
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--workspaces", "--prefix", ""],
        &environment,
        &services,
    )
    .expect("workspace completion succeeds");

    assert_eq!(result.stdout, "fix\nreview\n");
}

#[test]
fn work_complete_can_list_only_primary_repositories() {
    // Verifies: Project-argument shell completion omits secondary workspaces.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    create_jj_workspace_marker(&workspace.home.join("projects/project"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/project/fix"));
    create_jj_workspace_marker(&workspace.home.join("projects/other"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--repositories", "--prefix", "p"],
        &environment,
        &services,
    )
    .expect("repository completion succeeds");

    assert_eq!(result.stdout, "project\n");
}

#[test]
fn work_root_resolves_global_workspace_key() {
    // Verifies: Shell integration can resolve a global repo@workspace key to a path-only result.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let expected_root = workspace.home.join("projects/.work/project/fix");
    create_jj_workspace_marker(&workspace.home.join("projects/project"));
    create_jj_workspace_marker(&expected_root);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "root", "project@fix"],
        &environment,
        &services,
    )
    .expect("work root succeeds");

    assert_eq!(result.stdout, format!("{}\n", expected_root.display()));
}

#[test]
fn work_root_navigation_prefers_current_workspace_names_over_global_keys() {
    // Verifies: `u name` resolves same-repository workspaces before unrelated global keys.
    let workspace = TestWorkspace::new_under("projects/.work/project/current");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let sibling_root = workspace.home.join("projects/.work/project/other");
    let unrelated_root = workspace.home.join("projects/other");
    create_jj_workspace_marker(&sibling_root);
    create_jj_workspace_marker(&unrelated_root);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "current".to_owned(),
                root: workspace.path(),
                is_current: true,
            },
            WorkspaceEntry {
                name: "other".to_owned(),
                root: sibling_root.clone(),
                is_current: false,
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "other"],
        &environment,
        &services,
    )
    .expect("navigation root succeeds");

    assert_eq!(result.stdout, format!("{}\n", sibling_root.display()));
}

#[test]
fn work_root_navigation_resolves_unique_location_fragment() {
    // Verifies: `u fragment` can resolve one layout location without requiring a prefix match.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let expected_root = workspace.home.join("projects/termflow");
    create_jj_workspace_marker(&expected_root);
    create_jj_workspace_marker(&workspace.home.join("projects/other"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "flow"],
        &environment,
        &services,
    )
    .expect("navigation root succeeds");

    assert_eq!(result.stdout, format!("{}\n", expected_root.display()));
}

#[test]
fn work_root_navigation_resolves_fragment_subpaths() {
    // Verifies: Slash-separated `u` fragments can pick a repo and nested directory by partial names.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let termflow = workspace.home.join("projects/termflow");
    create_jj_workspace_marker(&termflow);
    fs::create_dir_all(termflow.join("bin")).expect("create bin directory");
    fs::create_dir_all(termflow.join("hammerspoon")).expect("create hammerspoon directory");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let bin = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "flow/bin"],
        &environment,
        &services,
    )
    .expect("bin navigation root succeeds");
    let hammerspoon = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "term/spoon"],
        &environment,
        &services,
    )
    .expect("hammerspoon navigation root succeeds");

    assert_eq!(bin.stdout, format!("{}\n", termflow.join("bin").display()));
    assert_eq!(
        hammerspoon.stdout,
        format!("{}\n", termflow.join("hammerspoon").display())
    );
}

#[test]
fn work_root_navigation_accepts_explicit_paths() {
    // Verifies: `u` treats absolute and dot-relative paths as filesystem navigation before fuzzy matching.
    let workspace = TestWorkspace::new_uninitialized_under("projects/current");
    let relative_target = workspace.home.join("projects/foo");
    let absolute_target = workspace.home.join("absolute-target");
    fs::create_dir_all(&relative_target).expect("create relative target");
    fs::create_dir_all(&absolute_target).expect("create absolute target");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let relative = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "../foo"],
        &environment,
        &services,
    )
    .expect("relative path navigation succeeds");
    let absolute_arg = absolute_target.to_string_lossy().into_owned();
    let absolute = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", absolute_arg.as_str()],
        &environment,
        &services,
    )
    .expect("absolute path navigation succeeds");

    assert_eq!(
        relative.stdout,
        format!("{}\n", fs::canonicalize(relative_target).unwrap().display())
    );
    assert_eq!(
        absolute.stdout,
        format!("{}\n", fs::canonicalize(absolute_target).unwrap().display())
    );
}

#[test]
fn work_root_navigation_rejects_ambiguous_fragments() {
    // Verifies: Fuzzy navigation only succeeds when the fragment selects one best target.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    create_jj_workspace_marker(&workspace.home.join("projects/termflow"));
    create_jj_workspace_marker(&workspace.home.join("projects/terminal"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let error = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "term"],
        &environment,
        &services,
    )
    .expect_err("ambiguous navigation roots are rejected");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::WorkLocationAmbiguous { .. })
    ));
}

#[test]
fn work_complete_navigation_orders_current_repo_before_global_locations() {
    // Verifies: navigation completion presents current-repo layout workspace aliases and trunk aliases first.
    let workspace = TestWorkspace::new_under("projects/.work/project/current");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let primary_root = workspace.home.join("projects/project");
    let sibling_root = workspace.home.join("projects/.work/project/fix");
    let unrelated_root = workspace.home.join("projects/other");
    create_jj_workspace_marker(&primary_root);
    create_jj_workspace_marker(&sibling_root);
    create_jj_workspace_marker(&unrelated_root);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: vec![WorkspaceEntry {
            name: "current".to_owned(),
            root: workspace.path(),
            is_current: true,
        }],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--navigation", "--prefix", ""],
        &environment,
        &services,
    )
    .expect("navigation completion succeeds");

    assert_eq!(
        result.stdout,
        "current\nfix\ndefault\ntrunk\nroot\nproject\nproject@current\nproject@fix\nother\n"
    );

    let default = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "default"],
        &environment,
        &services,
    )
    .expect("default navigation root succeeds");
    assert_eq!(default.stdout, format!("{}\n", primary_root.display()));
}

#[test]
fn work_navigation_uses_configured_repository_slugs() {
    // Verifies: Shell navigation can require organization slugs for configured repository groups.
    let workspace = TestWorkspace::new_uninitialized_under("outside");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "Faire"
root = "~/faire"
path = "{repo}"

[shell]
slug_repositories = ["Faire/*"]
"#,
    );
    let primary = workspace.home.join("faire/backend");
    let fix = workspace.home.join("faire/.work/backend/FD-123-fix");
    create_jj_workspace_marker(&primary);
    create_jj_workspace_marker(&fix);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let completion = run_with_args_and_services(
        [
            "jx",
            "work",
            "complete",
            "--navigation",
            "--prefix",
            "Faire",
        ],
        &environment,
        &services,
    )
    .expect("navigation completion succeeds");
    let root = run_with_args_and_services(
        [
            "jx",
            "work",
            "root",
            "--navigation",
            "Faire/backend@FD-123-fix",
        ],
        &environment,
        &services,
    )
    .expect("slugged navigation root succeeds");
    let raw = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "backend"],
        &environment,
        &services,
    )
    .expect_err("raw repo name is not a slugged navigation key");

    assert_eq!(
        completion.stdout,
        "Faire/backend\nFaire/backend@FD-123-fix\n"
    );
    assert_eq!(root.stdout, format!("{}\n", fix.display()));
    assert!(matches!(
        raw,
        CommandError::Repository(RepositoryError::WorkLocationNotFound { .. })
    ));
}

#[test]
fn work_complete_navigation_records_perf_steps() {
    // Verifies: Navigation completion reports where candidate gathering spends time.
    let workspace = TestWorkspace::new_under("projects/.work/sample/current");
    let log_path = workspace.home.join("work-complete-perf.jsonl");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    create_jj_workspace_marker(&workspace.home.join("projects/sample"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/sample/current"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/sample/review"));
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            ("HOME".to_owned(), workspace.home.display().to_string()),
            ("JX_PERF_LOG".to_owned(), log_path.display().to_string()),
        ],
    );
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "current".to_owned(),
                root: workspace.home.join("projects/.work/sample/current"),
                is_current: true,
            },
            WorkspaceEntry {
                name: "review".to_owned(),
                root: workspace.home.join("projects/.work/sample/review"),
                is_current: false,
            },
        ],
        ..FakeServices::default()
    };

    run_with_args_and_services(
        [
            "jx",
            "work",
            "complete",
            "--navigation",
            "--format",
            "picker",
            "--prefix",
            "sample@",
        ],
        &environment,
        &services,
    )
    .expect("navigation completion succeeds");
    let events = work_perf_events(&log_path);
    let event = events
        .iter()
        .find(|event| event["op"] == "work.complete")
        .expect("work.complete span is recorded");

    assert_eq!(event["mode"], "navigation");
    assert_eq!(event["format"], "picker");
    assert_eq!(event["candidate_count"], 2);
    assert_eq!(event["current_workspace_count"], 1);
    assert_eq!(event["global_location_count"], 3);
    assert_eq!(
        work_perf_step_names(event),
        [
            "discover_global_config",
            "load_current_workspaces",
            "discover_global_work_locations",
            "compose_navigation_locations",
            "filter_candidates",
            "render",
        ]
    );
}

#[test]
fn work_root_navigation_fast_path_records_perf_steps() {
    // Verifies: exact current-repo workspace aliases avoid scanning global layout roots.
    let workspace = TestWorkspace::new_under("projects/.work/sample/current");
    let log_path = workspace.home.join("work-root-fast-perf.jsonl");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    create_jj_workspace_marker(&workspace.home.join("projects/.work/sample/current"));
    fs::create_dir_all(workspace.home.join("projects/.work/sample/review"))
        .expect("create managed workspace directory");
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            ("HOME".to_owned(), workspace.home.display().to_string()),
            ("JX_PERF_LOG".to_owned(), log_path.display().to_string()),
        ],
    );
    let services = FakeServices {
        workspaces: vec![WorkspaceEntry {
            name: "current".to_owned(),
            root: workspace.home.join("projects/.work/sample/current"),
            is_current: true,
        }],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "review"],
        &environment,
        &services,
    )
    .expect("navigation root succeeds");
    let events = work_perf_events(&log_path);
    let event = events
        .iter()
        .find(|event| event["op"] == "work.root")
        .expect("work.root span is recorded");

    assert_eq!(
        result.stdout,
        format!(
            "{}\n",
            workspace
                .home
                .join("projects/.work/sample/review")
                .display()
        )
    );
    assert_eq!(event["navigation"], true);
    assert_eq!(event["local_resolution"], true);
    assert_eq!(event["current_workspace_count"], 1);
    assert_eq!(event.get("global_location_count"), None);
    assert_eq!(
        work_perf_step_names(event),
        [
            "discover_global_config",
            "load_current_workspaces",
            "resolve_local_navigation_target",
            "render",
        ]
    );
}

#[test]
fn work_root_navigation_global_fallback_records_perf_steps() {
    // Verifies: non-local navigation still reports the expensive global discovery phases.
    let workspace = TestWorkspace::new_under("projects/.work/sample/current");
    let log_path = workspace.home.join("work-root-global-perf.jsonl");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    create_jj_workspace_marker(&workspace.home.join("projects/sample"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/sample/current"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/sample/review"));
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            ("HOME".to_owned(), workspace.home.display().to_string()),
            ("JX_PERF_LOG".to_owned(), log_path.display().to_string()),
        ],
    );
    let services = FakeServices {
        workspaces: vec![WorkspaceEntry {
            name: "current".to_owned(),
            root: workspace.home.join("projects/.work/sample/current"),
            is_current: true,
        }],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "root", "--navigation", "sample@review"],
        &environment,
        &services,
    )
    .expect("navigation root succeeds");
    let events = work_perf_events(&log_path);
    let event = events
        .iter()
        .find(|event| event["op"] == "work.root")
        .expect("work.root span is recorded");

    assert_eq!(
        result.stdout,
        format!(
            "{}\n",
            workspace
                .home
                .join("projects/.work/sample/review")
                .display()
        )
    );
    assert_eq!(event["navigation"], true);
    assert_eq!(event["local_resolution"], false);
    assert_eq!(event["current_workspace_count"], 1);
    assert_eq!(event["global_location_count"], 3);
    assert_eq!(event["location_count"], 8);
    assert_eq!(
        work_perf_step_names(event),
        [
            "discover_global_config",
            "load_current_workspaces",
            "resolve_local_navigation_target",
            "discover_global_work_locations",
            "compose_navigation_locations",
            "resolve_navigation_target",
            "render",
        ]
    );
}

fn work_perf_events(path: &Path) -> Vec<serde_json::Value> {
    read_jsonl_events(path, "perf event is json")
}

fn read_jsonl_events(path: &Path, expectation: &str) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .expect("jsonl log is written")
        .lines()
        .map(|line| serde_json::from_str(line).expect(expectation))
        .collect()
}

fn work_perf_step_names(event: &serde_json::Value) -> Vec<&str> {
    event["steps"]
        .as_array()
        .expect("steps are recorded")
        .iter()
        .map(|step| step["name"].as_str().expect("step has name"))
        .collect()
}

#[test]
fn work_trunk_resolves_primary_checkout_from_current_workspace() {
    // Verifies: A managed workspace can resolve the trunk checkout for quick navigation back.
    let workspace = TestWorkspace::new_under("projects/.work/tool/fix");
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
    let trunk = workspace.home.join("projects/tool");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "work", "trunk"], &environment, &services)
        .expect("work trunk succeeds");

    assert_eq!(result.stdout, format!("{}\n", trunk.display()));
}

#[test]
fn work_trunk_shell_cd_target_prints_only_trunk_checkout_target() {
    // Verifies: Shell integration can jump to the trunk checkout without printing an extra path row.
    let workspace = TestWorkspace::new_under("projects/.work/tool/fix");
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
    let trunk = workspace.home.join("projects/tool");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "trunk", "--shell-cd-target"],
        &environment,
        &services,
    )
    .expect("work trunk shell target succeeds");

    assert_eq!(
        result.stdout,
        format!("{}{}\n", SHELL_CD_TARGET_PREFIX, trunk.display())
    );
}

#[test]
fn work_complete_qualifies_colliding_repository_names() {
    // Verifies: Global keys stay deterministic when two configured repos share a basename.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[layout.default]
path = "{owner}/{repo}"

[[layout.rules]]
source = "github"
root = "~/projects"
path = "{owner}/{repo}"
owner = "alpha"

[[layout.rules]]
source = "github"
root = "~/projects"
path = "{owner}/{repo}"
owner = "beta"
"#,
    );
    create_jj_workspace_marker(&workspace.home.join("projects/alpha/tool"));
    create_jj_workspace_marker(&workspace.home.join("projects/.work/alpha/tool/fix"));
    create_jj_workspace_marker(&workspace.home.join("projects/beta/tool"));
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "complete", "--prefix", ""],
        &environment,
        &services,
    )
    .expect("work completion succeeds");

    assert_eq!(result.stdout, "alpha/tool\nalpha/tool@fix\nbeta/tool\n");
}

#[test]
fn work_list_all_renders_global_keys_and_paths() {
    // Verifies: The global work list exposes the same keys that shell completion resolves.
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"
"#,
    );
    let project_root = workspace.home.join("projects/project");
    let workspace_root = workspace.home.join("projects/.work/project/fix");
    create_jj_workspace_marker(&project_root);
    create_jj_workspace_marker(&workspace_root);
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "work", "list", "--all", "--prefix", "project"],
        &environment,
        &services,
    )
    .expect("global work list succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "project      {}\nproject@fix  {}\n",
            project_root.display(),
            workspace_root.display()
        )
    );
}

#[test]
fn work_delete_can_be_cancelled() {
    // Verifies: Declining deletion stops before jj forget or filesystem deletion.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: project_workspaces(&workspace),
        ..FakeServices::default()
    };
    let confirmer = FixedWorkspaceRemoveConfirmer { confirmed: false };

    let result = run_with_args_and_workspace_remove_confirmer(
        ["jx", "work", "delete", "fix"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("workspace deletion cancellation succeeds");

    assert_eq!(result.stdout, "cancelled\n");
    assert!(services.workspace_removes.borrow().is_empty());
}

#[test]
fn work_delete_forgets_and_deletes_managed_workspace_after_confirmation() {
    // Verifies: Deletion delegates one confirmed forget/delete operation for managed workspaces.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: project_workspaces(&workspace),
        ..FakeServices::default()
    };
    let confirmer = FixedWorkspaceRemoveConfirmer { confirmed: true };

    let result = run_with_args_and_workspace_remove_confirmer(
        ["jx", "work", "delete", "fix"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("workspace deletion succeeds");

    assert_eq!(result.stdout, "Deleted workspace: fix\n");
    assert_eq!(
        services.workspace_removes.borrow().as_slice(),
        [WorkspaceRemoveOptions {
            name: "fix".to_owned(),
            root: workspace.home.join("projects/.work/jx/fix"),
            cleanup_root: workspace.home.join("projects/.work"),
        }]
    );
}

#[test]
fn work_delete_runs_configured_hooks_from_target_workspace_before_removal() {
    // Verifies: Delete hooks run from the workspace being deleted, even when invoked elsewhere.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[[repo.rules]]
repo = "example-owner/*"

[[repo.rules.hooks]]
id = "bazel-shutdown"
on = "workspace.delete.before"
command = ["bazel", "shutdown"]

[[repo.rules.hooks]]
id = "bazel-expunge"
on = "workspace.delete.before"
command = ["bazel", "clean", "--expunge"]
"#,
    );
    let target = workspace.home.join("projects/.work/jx/fix");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: project_workspaces(&workspace),
        ..FakeServices::default()
    };
    let confirmer = FixedWorkspaceRemoveConfirmer { confirmed: true };

    let result = run_with_args_and_workspace_remove_confirmer(
        ["jx", "work", "delete", "fix"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("workspace deletion succeeds");

    assert_eq!(
        result.stdout,
        "Deleted workspace: fix\nEvent[bazel-shutdown]: ran `bazel shutdown`\nEvent[bazel-expunge]: ran `bazel clean --expunge`\n"
    );
    let hook_calls = services.hook_command_calls.borrow();
    assert_eq!(hook_calls.len(), 2);
    assert_eq!(hook_calls[0].0, target);
    assert_eq!(hook_calls[0].1.id, "bazel-shutdown");
    assert_eq!(
        hook_calls[0].1.command,
        vec!["bazel".to_owned(), "shutdown".to_owned()]
    );
    assert_eq!(
        hook_calls[1].0,
        workspace.home.join("projects/.work/jx/fix")
    );
    assert_eq!(hook_calls[1].1.id, "bazel-expunge");
    assert_eq!(
        hook_calls[1].1.command,
        vec![
            "bazel".to_owned(),
            "clean".to_owned(),
            "--expunge".to_owned()
        ]
    );
    assert_eq!(
        services.workspace_delete_events.borrow().as_slice(),
        ["hook:bazel-shutdown", "hook:bazel-expunge", "remove:fix"]
    );
    let log = read_jsonl_events(
        &workspace.home.join(".local/state/jx/jx-hooks.log"),
        "hook log event is json",
    );
    assert_eq!(log.len(), 4);
    assert_eq!(log[0]["status"], "start");
    assert_eq!(log[0]["hook"], "bazel-shutdown");
    assert_eq!(log[0]["event"], "workspace.delete.before");
    assert_eq!(log[0]["repo"], "example-owner/jx");
    assert_eq!(log[0]["workspace"], "fix");
    assert_eq!(
        log[0]["cwd"],
        workspace
            .home
            .join("projects/.work/jx/fix")
            .display()
            .to_string()
    );
    assert_eq!(log[1]["status"], "success");
    assert_eq!(log[2]["hook"], "bazel-expunge");
    assert_eq!(log[3]["status"], "success");
}

#[test]
fn work_delete_shell_cd_output_keeps_hook_events_subdued() {
    // Verifies: Shell integration still renders hook event lines with the same subdued style as stack publish.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[[repo.rules]]
repo = "example-owner/*"

[[repo.rules.hooks]]
id = "cleanup"
on = "workspace.delete.before"
command = ["./cleanup"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: project_workspaces(&workspace),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "delete", "fix", "--shell-cd-target"],
        &environment,
        &services,
    )
    .expect("workspace deletion succeeds");

    assert_eq!(
        result.stdout,
        "Deleted workspace: fix\n\u{1b}[2m\u{1b}[38;5;244mEvent[cleanup]: ran `./cleanup`\u{1b}[0m\n"
    );
}

#[test]
fn work_delete_runs_configured_hooks_from_current_workspace_before_safe_removal() {
    // Verifies: Current-workspace deletion runs hooks before moving the process to the safe operation directory.
    let workspace = TestWorkspace::new_under("projects/.work/tool/fix");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[[repo.rules]]
repo = "example-owner/*"

[[repo.rules.hooks]]
id = "cleanup"
on = "workspace.delete.before"
command = ["./cleanup"]
"#,
    );
    let primary = workspace.home.join("projects/tool");
    let managed = workspace.home.join("projects/.work/tool/fix");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "default".to_owned(),
                root: primary.clone(),
                is_current: false,
            },
            WorkspaceEntry {
                name: "fix".to_owned(),
                root: managed.clone(),
                is_current: true,
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(["jx", "work", "delete"], &environment, &services)
        .expect("current workspace deletion succeeds");

    assert_eq!(
        result.stdout,
        "Deleted workspace: fix\nEvent[cleanup]: ran `./cleanup`\n"
    );
    assert_eq!(services.hook_command_calls.borrow()[0].0, managed);
    assert_eq!(
        services.workspace_remove_current_dirs.borrow().as_slice(),
        [primary]
    );
    assert_eq!(
        services.workspace_delete_events.borrow().as_slice(),
        ["hook:cleanup", "remove:fix"]
    );
}

#[test]
fn work_delete_aborts_without_removing_when_configured_hook_fails() {
    // Verifies: Hook failures keep the workspace intact so cleanup problems can be fixed and retried.
    let workspace = TestWorkspace::new_under("projects/jx");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"

[[repo.rules]]
repo = "example-owner/*"

[[repo.rules.hooks]]
id = "cleanup"
on = "workspace.delete.before"
command = ["./cleanup"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: project_workspaces(&workspace),
        hook_command_outputs: std::cell::RefCell::new(vec![HookCommandOutput::failure(
            "exit code 7",
            "cleanup failed\n",
        )]),
        ..FakeServices::default()
    };
    let confirmer = FixedWorkspaceRemoveConfirmer { confirmed: true };

    let error = run_with_args_and_workspace_remove_confirmer(
        ["jx", "work", "delete", "fix"],
        &environment,
        &services,
        &confirmer,
    )
    .expect_err("failing hook aborts deletion");

    assert!(matches!(error, CommandError::Hook { .. }));
    assert!(error.to_string().contains("cleanup failed"));
    assert_eq!(
        services.workspace_delete_events.borrow().as_slice(),
        ["hook:cleanup"]
    );
    assert!(services.workspace_removes.borrow().is_empty());
    let log = read_jsonl_events(
        &workspace.home.join(".local/state/jx/jx-hooks.log"),
        "hook log event is json",
    );
    assert_eq!(log.len(), 2);
    assert_eq!(log[0]["status"], "start");
    assert_eq!(log[1]["status"], "error");
    assert_eq!(log[1]["message"], "exit code 7");
    assert_eq!(log[1]["output"], "cleanup failed");
}

#[test]
fn work_delete_resolves_unique_workspace_name_fragment() {
    // Verifies: Delete accepts the same exact/prefix/contains workspace fragments as navigation.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "default".to_owned(),
                root: workspace.home.join("projects/jx"),
                is_current: true,
            },
            WorkspaceEntry {
                name: "food".to_owned(),
                root: workspace.home.join("projects/.work/jx/food"),
                is_current: false,
            },
        ],
        ..FakeServices::default()
    };
    let confirmer = FixedWorkspaceRemoveConfirmer { confirmed: true };

    let result = run_with_args_and_workspace_remove_confirmer(
        ["jx", "work", "delete", "foo"],
        &environment,
        &services,
        &confirmer,
    )
    .expect("workspace deletion succeeds");

    assert_eq!(result.stdout, "Deleted workspace: food\n");
    assert_eq!(
        services.workspace_removes.borrow().as_slice(),
        [WorkspaceRemoveOptions {
            name: "food".to_owned(),
            root: workspace.home.join("projects/.work/jx/food"),
            cleanup_root: workspace.home.join("projects/.work"),
        }]
    );
}

#[test]
fn work_delete_rejects_ambiguous_workspace_name_fragments() {
    // Verifies: Fragment deletion fails before prompting when multiple names share the best match.
    let workspace = TestWorkspace::new_under("projects/jx");
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
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "default".to_owned(),
                root: workspace.home.join("projects/jx"),
                is_current: true,
            },
            WorkspaceEntry {
                name: "food".to_owned(),
                root: workspace.home.join("projects/.work/jx/food"),
                is_current: false,
            },
            WorkspaceEntry {
                name: "fool".to_owned(),
                root: workspace.home.join("projects/.work/jx/fool"),
                is_current: false,
            },
        ],
        ..FakeServices::default()
    };

    let error =
        run_with_args_and_services(["jx", "work", "delete", "foo"], &environment, &services)
            .expect_err("ambiguous workspace fragment is rejected");

    assert!(matches!(
        error,
        CommandError::Repository(RepositoryError::WorkspaceNameAmbiguous { name, matches })
            if name == "foo" && matches == vec!["food".to_owned(), "fool".to_owned()]
    ));
    assert!(services.workspace_removes.borrow().is_empty());
}

#[test]
fn work_delete_current_managed_workspace_returns_shell_cd_target() {
    // Verifies: Deleting the active managed workspace runs from the trunk checkout and tells shell integration where to land.
    let workspace = TestWorkspace::new_under("projects/.work/tool/fix");
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
    let primary = workspace.home.join("projects/tool");
    let managed = workspace.home.join("projects/.work/tool/fix");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        workspaces: vec![
            WorkspaceEntry {
                name: "default".to_owned(),
                root: primary.clone(),
                is_current: false,
            },
            WorkspaceEntry {
                name: "fix".to_owned(),
                root: managed.clone(),
                is_current: true,
            },
        ],
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "work", "delete", "--shell-cd-target"],
        &environment,
        &services,
    )
    .expect("current managed workspace deletion succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Deleted workspace: fix\n{}{}\n",
            SHELL_CD_TARGET_PREFIX,
            primary.display()
        )
    );
    assert_eq!(
        services.workspace_remove_current_dirs.borrow().as_slice(),
        [primary]
    );
    assert_eq!(
        services.workspace_removes.borrow().as_slice(),
        [WorkspaceRemoveOptions {
            name: "fix".to_owned(),
            root: managed,
            cleanup_root: workspace.home.join("projects/.work"),
        }]
    );
}

#[test]
fn work_delete_refuses_primary_and_unmanaged_paths() {
    // Verifies: Delete only targets workspaces inside the managed `.work` layout.
    enum RootKind {
        Primary,
        Unmanaged,
    }

    enum ExpectedError {
        Primary,
        Unmanaged,
    }

    let cases = [
        ("default", RootKind::Primary, false, ExpectedError::Primary),
        ("fix", RootKind::Unmanaged, false, ExpectedError::Unmanaged),
    ];

    for (name, root, is_current, expected_error) in cases {
        let workspace = TestWorkspace::new_under("projects/jx");
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
        let target = WorkspaceEntry {
            name: name.to_owned(),
            root: match root {
                RootKind::Primary => workspace.home.join("projects/jx"),
                RootKind::Unmanaged => PathBuf::from("/tmp/jx/fix"),
            },
            is_current,
        };
        let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
        let services = FakeServices {
            workspaces: vec![target.clone()],
            ..FakeServices::default()
        };

        let error = run_with_args_and_services(
            ["jx", "work", "delete", target.name.as_str()],
            &environment,
            &services,
        )
        .expect_err("unsafe workspace deletion is rejected");

        match expected_error {
            ExpectedError::Primary => assert!(matches!(
                error,
                CommandError::Repository(RepositoryError::RefuseRemovePrimaryWorkspace { .. })
            )),
            ExpectedError::Unmanaged => assert!(matches!(
                error,
                CommandError::Repository(RepositoryError::RefuseRemoveUnmanagedWorkspace { .. })
            )),
        }
        assert!(services.workspace_removes.borrow().is_empty());
    }
}
