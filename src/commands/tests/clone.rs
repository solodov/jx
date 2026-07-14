use super::*;

#[test]
fn clone_uses_owner_rule_and_default_source_shorthand() {
    // Verifies: Clone resolves owner/repo shorthands through layout rules before invoking jj.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[layout]
default_root = "~/src"

[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.path().join("work/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "clone", "example-owner/example-repo"],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/work/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            )
        )
    );
}

#[test]
fn clone_infers_owner_from_current_layout_prefix() {
    // Verifies: Repo-only clone shorthands use the cwd when it supplies the configured slug prefix.
    let workspace = TestWorkspace::new_uninitialized_under("projects");
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
    let expected_destination = workspace.home.join("projects/example-tool");
    let services = FakeServices {
        expected_clone: Some((
            "git@github.com:example-owner/example-tool.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result =
        run_with_args_and_services(["jx", "clone", "example-tool"], &environment, &services)
            .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/projects/example-tool\n",
            osc8_link(
                "https://github.com/example-owner/example-tool",
                "git@github.com:example-owner/example-tool.git"
            )
        )
    );
}

#[test]
fn clone_uses_default_layout_for_unmatched_github_repos() {
    // Verifies: Unmatched repositories stay globally discoverable under the default root.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace
        .path()
        .join("src/github.com/example-owner/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "clone", "example-owner/example-repo"],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/src/github.com/example-owner/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            )
        )
    );
}

#[test]
fn clone_accepts_host_owner_repo_form() {
    // Verifies: Explicit host input still uses the matching source and layout rules.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.path().join("work/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "git@github.com:example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "clone", "github.com/example-owner/example-repo"],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/work/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "git@github.com:example-owner/example-repo.git"
            )
        )
    );
}

#[test]
fn clone_uses_configured_clone_url_format() {
    // Verifies: Source config owns generated clone URL shape for shorthand inputs.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[layout.sources]]
name = "github"
provider = "github"
host = "github.com"
clone_url = "https"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace
        .path()
        .join("src/github.com/example-owner/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "https://github.com/example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        ["jx", "clone", "example-owner/example-repo"],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/src/github.com/example-owner/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "https://github.com/example-owner/example-repo.git"
            )
        )
    );
}

#[test]
fn clone_preserves_explicit_url_but_uses_layout_destination() {
    // Verifies: Explicit URLs decide clone transport while normalized identity decides placement.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let expected_destination = workspace.path().join("work/example-repo");
    let services = FakeServices {
        expected_clone: Some((
            "https://github.com/example-owner/example-repo.git".to_owned(),
            expected_destination.clone(),
        )),
        ..FakeServices::default()
    };

    let result = run_with_args_and_services(
        [
            "jx",
            "clone",
            "https://github.com/example-owner/example-repo.git",
        ],
        &environment,
        &services,
    )
    .expect("clone succeeds");

    assert_eq!(
        result.stdout,
        format!(
            "Cloned {} to ~/work/example-repo\n",
            osc8_link(
                "https://github.com/example-owner/example-repo",
                "https://github.com/example-owner/example-repo.git"
            )
        )
    );
}
