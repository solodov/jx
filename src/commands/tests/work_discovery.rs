use super::*;

#[test]
fn work_discovery_does_not_walk_unrelated_home_or_repository_contents() {
    // Verifies: unrelated trees cannot increase discovery work for a literal home-root rule.
    let workspace = TestWorkspace::new_uninitialized_under("");
    workspace.write_home_file(
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
    for path in [
        "org",
        ".work/org/task",
        "projects/api",
        "projects/not-a-repo",
    ] {
        fs::create_dir_all(workspace.home.join(path)).expect("create layout directory");
    }
    for path in ["org", ".work/org/task", "projects/api"] {
        create_jj_workspace_marker(&workspace.home.join(path));
    }
    let log_path = workspace.home.join("discovery-perf.jsonl");
    let environment = discovery_environment(&workspace, &log_path);
    let config = WorkflowConfig::discover_global(&environment).expect("config loads");
    let before = global_work_locations(&config, &environment).expect("locations resolve");
    let before_event = last_discovery_event(&log_path);

    for index in 0..100 {
        for prefix in ["Library/cache", "org/build", "projects/not-a-repo/build"] {
            create_jj_workspace_marker(&workspace.home.join(format!("{prefix}/{index}/nested")));
        }
    }
    let after = global_work_locations(&config, &environment).expect("locations resolve");
    let after_event = last_discovery_event(&log_path);

    assert_eq!(before, after);
    assert_eq!(location_keys(&after), ["api", "org", "org@task"]);
    assert_eq!(
        discovery_counts(&before_event),
        discovery_counts(&after_event)
    );
    let home_scan = root_scan(&after_event, &workspace.home);
    assert_eq!(home_scan["pattern_count"], 2);
    assert_eq!(home_scan["directory_count"], 6);
    assert_eq!(home_scan["read_dir_count"], 1); // Only ~/.work/org is enumerated.
    assert_eq!(home_scan["workspace_root_count"], 2);
}

#[test]
fn work_discovery_preserves_overrides_collisions_and_workspace_only_repositories() {
    // Verifies: narrow traversal still discovers workspaces without primaries and rejects old placements.
    let workspace = TestWorkspace::new_uninitialized_under("");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[layout]
workspace_dir = ".tasks"

[[layout.rules]]
source = "github"
owner = "alpha"
root = "~/projects"
path = "{repo}"

[[layout.rules]]
source = "github"
owner = "alpha"
repo = "org"
root = "~"
path = "org"
"#,
    );
    for path in [
        "org",
        "projects/org",
        "src/github.com/alpha/org",
        "src/github.com/beta/org",
        "projects/.tasks/api/review",
        "projects/.tasks/api/review/nested",
        "projects/.tasks/org/stale",
        ".tasks/org/current",
    ] {
        create_jj_workspace_marker(&workspace.home.join(path));
    }
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let config = WorkflowConfig::discover_global(&environment).expect("config loads");

    let locations = global_work_locations(&config, &environment).expect("locations resolve");
    let repositories =
        global_work_repositories(&config, &environment).expect("repositories resolve");

    assert_eq!(
        location_keys(&locations),
        ["alpha/org", "alpha/org@current", "api@review", "beta/org"]
    );
    assert_eq!(locations[0].root, workspace.home.join("org"));
    assert_eq!(
        repositories
            .iter()
            .map(|repo| repo.key.as_str())
            .collect::<Vec<_>>(),
        ["alpha/org", "beta/org"]
    );
}

#[test]
fn work_discovery_primary_scope_does_not_enumerate_managed_workspaces() {
    // Verifies: cross-repository commands keep managed workspace trees out of their discovery path.
    let workspace = TestWorkspace::new_uninitialized_under("");
    workspace.write_home_file(
        ".config/jx/config.toml",
        r#"
[[layout.rules]]
source = "github"
owner = "example"
repo = "org"
root = "~"
path = "org"
"#,
    );
    create_jj_workspace_marker(&workspace.home.join("org"));
    let log_path = workspace.home.join("discovery-perf.jsonl");
    let environment = discovery_environment(&workspace, &log_path);
    let config = WorkflowConfig::discover_global(&environment).expect("config loads");
    global_work_repositories(&config, &environment).expect("repositories resolve");
    let before = last_discovery_event(&log_path);
    create_jj_workspace_marker(&workspace.home.join(".work/org/task"));

    let repositories =
        global_work_repositories(&config, &environment).expect("repositories resolve");
    let after = last_discovery_event(&log_path);

    assert_eq!(repositories.len(), 1);
    assert_eq!(after["scope"], "primary");
    assert_eq!(discovery_counts(&before), discovery_counts(&after));
    assert_eq!(root_scan(&after, &workspace.home)["read_dir_count"], 0);
}

#[cfg(unix)]
#[test]
fn work_discovery_does_not_follow_literal_or_placeholder_directory_symlinks() {
    // Verifies: both traversal branches preserve the old walker's child-symlink policy.
    let workspace = TestWorkspace::new_uninitialized_under("");
    workspace.write_home_file(
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
    let target = workspace.home.join("elsewhere/target");
    create_jj_workspace_marker(&target);
    fs::create_dir_all(workspace.home.join("projects")).expect("create projects");
    std::os::unix::fs::symlink(&target, workspace.home.join("org")).expect("link literal path");
    std::os::unix::fs::symlink(&target, workspace.home.join("projects/api"))
        .expect("link placeholder path");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let config = WorkflowConfig::discover_global(&environment).expect("config loads");

    assert!(global_work_locations(&config, &environment)
        .expect("locations resolve")
        .is_empty());
}

fn discovery_environment(workspace: &TestWorkspace, log_path: &Path) -> RuntimeEnvironment {
    RuntimeEnvironment::new(
        workspace.path(),
        [
            ("HOME".to_owned(), workspace.home.display().to_string()),
            ("JX_PERF_LOG".to_owned(), log_path.display().to_string()),
        ],
    )
}

fn last_discovery_event(log_path: &Path) -> serde_json::Value {
    fs::read_to_string(log_path)
        .expect("perf log exists")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("perf event is json"))
        .rfind(|event| event["op"] == "work.discover_locations")
        .expect("discovery span exists")
}

fn root_scan<'a>(event: &'a serde_json::Value, root: &Path) -> &'a serde_json::Value {
    event["steps"]
        .as_array()
        .expect("steps recorded")
        .iter()
        .find(|step| step["layout_root"] == root.display().to_string())
        .expect("root scan recorded")
}

fn discovery_counts(event: &serde_json::Value) -> Vec<(u64, u64, u64)> {
    event["steps"]
        .as_array()
        .expect("steps recorded")
        .iter()
        .filter(|step| step["name"] == "scan_layout_root")
        .map(|step| {
            (
                step["directory_count"].as_u64().expect("directory count"),
                step["read_dir_count"].as_u64().expect("read_dir count"),
                step["entry_count"].as_u64().expect("entry count"),
            )
        })
        .collect()
}

fn location_keys(locations: &[WorkLocation]) -> Vec<&str> {
    locations
        .iter()
        .map(|location| location.key.as_str())
        .collect()
}
