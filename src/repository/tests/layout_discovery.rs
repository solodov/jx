use super::*;

#[test]
fn discovery_paths_bind_fixed_layout_components() {
    // Verifies: a home-root rule becomes a direct path, while only unknown names are enumerated.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"

[[layout.rules]]
source = "github"
owner = "example"
repo = "org"
root = "~"
path = "org"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let config = WorkflowConfig::discover_global(&environment).expect("config loads");

    let paths = config
        .layout
        .discovery_paths(&environment)
        .expect("paths resolve");

    assert_eq!(
        paths,
        vec![
            LayoutDiscoveryPath {
                root: workspace.path().to_path_buf(),
                components: vec![Some("org".to_owned())],
            },
            LayoutDiscoveryPath {
                root: workspace.path().join("projects"),
                components: vec![None],
            },
            LayoutDiscoveryPath {
                root: workspace.path().join("src"),
                components: vec![Some("github.com".to_owned()), None, None],
            },
        ]
    );
}

#[test]
fn discovery_paths_bind_source_host_owner_and_repo_without_globbing_literals() {
    // Verifies: rules narrow placeholders and punctuation in literal paths remains literal.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
repo = "org"
root = "~"
path = "repos[1]/{source}/{host}/{owner}/{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let config = WorkflowConfig::discover_global(&environment).expect("config loads");

    let paths = config
        .layout
        .discovery_paths(&environment)
        .expect("paths resolve");

    assert_eq!(
        paths[0].components,
        ["repos[1]", "github", "github.com", "example", "org"].map(|value| Some(value.to_owned()))
    );
}

#[test]
fn discovery_paths_deduplicate_equivalent_rules() {
    // Verifies: overlapping equivalent rules do not repeat the same filesystem walk.
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
root = "~/projects"
path = "{repo}"

[[layout.rules]]
source = "github"
owner = "other"
root = "~/projects"
path = "{repo}"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let config = WorkflowConfig::discover_global(&environment).expect("config loads");

    let paths = config
        .layout
        .discovery_paths(&environment)
        .expect("paths resolve");

    assert_eq!(paths.len(), 2);
}
