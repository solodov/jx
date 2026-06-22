use super::*;
use jj_lib::{
    config::StackedConfig,
    git,
    ref_name::RemoteName,
    repo::StoreFactories,
    settings::UserSettings,
    workspace::{default_working_copy_factories, Workspace},
};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn discovers_fixed_origin_github_context() {
    // Verifies: Repository discovery loads fixed-origin GitHub context.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
reviewers = []
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("JX_GITHUB_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(context.workspace_root, workspace.path());
    assert_eq!(context.repository_root, workspace.path());
    assert_eq!(context.origin.name, ORIGIN_REMOTE_NAME);
    assert_eq!(context.origin.github.owner, "example-owner");
    assert_eq!(context.origin.github.name, "example-repo");
    assert_eq!(context.github_remotes.len(), 1);
    assert_eq!(context.github_remotes[0].name, "origin");
    assert_eq!(
        context.github_remotes[0].github.https_url(),
        "https://github.com/example-owner/example-repo"
    );
    assert_eq!(
        context.token_source,
        TokenSource::Environment("JX_GITHUB_TOKEN")
    );
    assert_eq!(
        context.config.paths,
        vec![workspace.path().join(".jx/config.toml")]
    );
    assert!(context.config.repo.base.reviewers.is_empty());
}

#[test]
fn discovers_all_configured_github_remotes() {
    // Verifies: Status context can report every configured GitHub remote.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
[remote "upstream"]
    url = https://github.com/upstream-owner/example-repo.git
[remote "backup"]
    url = https://example.invalid/upstream-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context
            .github_remotes
            .iter()
            .map(|remote| (remote.name.as_str(), remote.github.slug()))
            .collect::<Vec<_>>(),
        vec![
            ("origin", "example-owner/example-repo".to_owned()),
            ("upstream", "upstream-owner/example-repo".to_owned()),
        ]
    );
}

#[test]
fn workspace_metadata_missing_file_returns_default() {
    // Verifies: Workspace metadata is optional for existing workspaces.
    let workspace = TestWorkspace::new();

    let metadata = read_workspace_metadata(&workspace.path()).expect("metadata reads");

    assert_eq!(metadata, WorkspaceMetadata::default());
}

#[test]
fn workspace_metadata_write_creates_ignored_state_file() {
    // Verifies: Workspace metadata stays local without ignoring future repo config.
    let workspace = TestWorkspace::new();

    write_workspace_metadata(
        &workspace.path(),
        &WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
        },
    )
    .expect("metadata writes");

    assert_eq!(
        fs::read_to_string(workspace.path().join(".jx/.gitignore")).expect("gitignore"),
        "/.gitignore\n/workspace.toml\n/stack.toml\n"
    );
    assert_eq!(
        read_workspace_metadata(&workspace.path()).expect("metadata reads"),
        WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
        }
    );
}

#[test]
fn github_user_name_cache_uses_global_cache_map_shape() {
    // Verifies: display-name cache is compact TOML keyed by login instead of array entries.
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [(
            "XDG_CACHE_HOME".to_owned(),
            workspace.path().join("cache").display().to_string(),
        )],
    );
    let now = chrono::DateTime::parse_from_rfc3339("2026-06-05T12:00:00Z")
        .expect("timestamp parses")
        .with_timezone(&chrono::Utc);
    let mut cache = GitHubUserNameCache::default();
    cache.upsert("human-reviewer", Some("Human Reviewer"), now);

    write_github_user_name_cache(&environment, &cache).expect("cache writes");

    let contents = fs::read_to_string(
        workspace
            .path()
            .join("cache")
            .join("jx")
            .join("github-users.toml"),
    )
    .expect("cache file reads");
    assert!(contents.contains("[users."));
    assert!(!contents.contains("[[users]]"));
    assert!(contents.contains("human-reviewer"));
    assert!(contents.contains("name = \"Human Reviewer\""));
    assert_eq!(
        read_github_user_name_cache(&environment)
            .expect("cache reads")
            .fresh_name("human-reviewer", now),
        Some(Some("Human Reviewer".to_owned()))
    );
}

#[test]
fn github_user_name_cache_expires_after_180_days() {
    // Verifies: cached names stay long-lived but eventually refresh from GitHub.
    let cached_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T12:00:00Z")
        .expect("timestamp parses")
        .with_timezone(&chrono::Utc);
    let fresh_at = cached_at + chrono::Duration::days(GITHUB_USER_NAME_CACHE_TTL_DAYS);
    let expired_at = fresh_at + chrono::Duration::seconds(1);
    let mut cache = GitHubUserNameCache::default();
    cache.upsert("human-reviewer", Some("Human Reviewer"), cached_at);

    assert_eq!(
        cache.fresh_name("human-reviewer", fresh_at),
        Some(Some("Human Reviewer".to_owned()))
    );
    assert_eq!(cache.fresh_name("human-reviewer", expired_at), None);
    assert_eq!(
        cache.cached_name("human-reviewer"),
        Some(Some("Human Reviewer".to_owned()))
    );
}

#[test]
fn stack_metadata_missing_file_returns_default() {
    // Verifies: Stack state is optional until a repository explicitly tracks a stack.
    let workspace = TestWorkspace::new();

    let metadata = read_stack_metadata(&workspace.path()).expect("metadata reads");

    assert_eq!(metadata, StackMetadata::default());
}

#[test]
fn stack_metadata_write_creates_ignored_state_file() {
    // Verifies: Stack metadata and its colocated ignore file both stay local to the checkout.
    let workspace = TestWorkspace::new();

    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![StackMetadataNode {
                branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                parent_branch: Some("topic/root".to_owned()),
                pull_request: Some(12),
                parent_pull_request: Some(11),
                title: "Child".to_owned(),
                url: None,
                draft: true,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            }],
        },
    )
    .expect("metadata writes");

    assert_eq!(
        fs::read_to_string(workspace.path().join(".jx/.gitignore")).expect("gitignore"),
        "/.gitignore\n/workspace.toml\n/stack.toml\n"
    );
    assert_eq!(
        read_stack_metadata(&workspace.path()).expect("metadata reads"),
        StackMetadata {
            version: 1,
            work_item_handler_runs: Vec::new(),
            nodes: vec![StackMetadataNode {
                branch: "topic/child".to_owned(),
                base_branch: "topic/root".to_owned(),
                parent_branch: Some("topic/root".to_owned()),
                pull_request: Some(12),
                parent_pull_request: Some(11),
                title: "Child".to_owned(),
                url: None,
                draft: true,
                merged: false,
                work_ids: Vec::new(),
                fixes_work_ids: Vec::new(),
            }],
        }
    );
}

#[test]
fn stack_metadata_write_preserves_handler_ledger_without_nodes() {
    // Verifies: completed-stack pruning does not erase the side-effect ledger needed to avoid duplicate handlers.
    let workspace = TestWorkspace::new();
    let metadata = StackMetadata {
        version: 1,
        work_item_handler_runs: vec![StackMetadataWorkItemHandlerRun {
            handler: "resolve-work".to_owned(),
            work_id: "ABC-123".to_owned(),
            pull_request: 101,
        }],
        nodes: Vec::new(),
    };

    write_stack_metadata(&workspace.path(), &metadata).expect("metadata writes");

    assert_eq!(
        read_stack_metadata(&workspace.path()).expect("metadata reads"),
        metadata
    );
}

#[test]
fn discovers_workspace_from_nested_directory() {
    // Verifies: Discovers workspace from nested directory.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = git@github.com:example-owner/example-repo.git
"#,
    );
    let nested = workspace.create_dir("src/nested");
    let environment = RuntimeEnvironment::new(nested, []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(context.workspace_root, workspace.path());
    assert_eq!(context.token_source, TokenSource::Missing);
    assert_eq!(context.config.summary(), "defaults");
    assert!(context.config.repo.base.reviewers.is_empty());
}

#[test]
fn loads_default_reviewers_from_project_config() {
    // Verifies: Loads default reviewers from project config.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
reviewers = ["example-reviewer", "second-reviewer"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(context.config.summary(), "present");
    assert_eq!(
        context.config.reviewer_summary(&context.origin.github),
        "2 configured reviewers"
    );
    assert_eq!(
        context.config.repo.base.reviewers,
        reviewer_users(["example-reviewer", "second-reviewer"])
    );
}

#[test]
fn loads_diff_tools_from_project_config() {
    // Verifies: Diff config records default external and pipe tools for jx diff rendering.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[diff]
default_tool = "difft"

[diff.tools.difft]
mode = "external"
command = "difft"
args = ["--color=always", "--display=side-by-side"]

[diff.tools.delta]
mode = "pipe"
producer_args = ["-w", "--git"]
command = "delta"
args = ["--features", "jj-diff"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(context.config.diff.default_tool.as_deref(), Some("difft"));
    assert_eq!(context.config.diff.tools.len(), 2);
    assert_eq!(
        context.config.diff.tools.get("difft"),
        Some(&DiffToolConfig::External(ExternalDiffToolConfig {
            command: "difft".to_owned(),
            args: vec![
                "--color=always".to_owned(),
                "--display=side-by-side".to_owned()
            ],
        }))
    );
    assert_eq!(
        context.config.diff.tools.get("delta"),
        Some(&DiffToolConfig::Pipe(PipeDiffToolConfig {
            producer_args: vec!["-w".to_owned(), "--git".to_owned()],
            command: "delta".to_owned(),
            args: vec!["--features".to_owned(), "jj-diff".to_owned()],
        }))
    );
}

#[test]
fn project_diff_config_overrides_global_default_and_tool() {
    // Verifies: Diff scalars and same-name tools follow the normal later-file override rules.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/00-base.toml",
        r#"
[diff]
default_tool = "delta"

[diff.tools.difft]
mode = "external"
command = "difft"
args = ["--display=inline"]

[diff.tools.delta]
mode = "pipe"
producer_args = ["--git"]
command = "delta"
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[diff]
default_tool = "difft"

[diff.tools.difft]
mode = "external"
command = "difft"
args = ["--display=side-by-side"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(context.config.diff.default_tool.as_deref(), Some("difft"));
    assert_eq!(context.config.diff.tools.len(), 2);
    assert_eq!(
        context.config.diff.tools.get("difft"),
        Some(&DiffToolConfig::External(ExternalDiffToolConfig {
            command: "difft".to_owned(),
            args: vec!["--display=side-by-side".to_owned()],
        }))
    );
}

#[test]
fn layout_resolves_project_and_managed_workspace_destinations() {
    // Verifies: The same layout identity places primary checkouts visibly and workspaces under `.work`.
    let environment =
        RuntimeEnvironment::new("/tmp", [("HOME".to_owned(), "/Users/example".to_owned())]);
    let identity = RepositoryIdentity {
        source: "github".to_owned(),
        host: "github.com".to_owned(),
        owner: "example-owner".to_owned(),
        repo: "example-repo".to_owned(),
    };
    let config = LayoutConfig::default();

    assert_eq!(
        config
            .project_destination(&identity, &environment)
            .expect("project path"),
        PathBuf::from("/Users/example/src/github.com/example-owner/example-repo")
    );
    assert_eq!(
        config
            .workspace_destination(&identity, "fix", &environment)
            .expect("workspace path"),
        PathBuf::from("/Users/example/src/.work/github.com/example-owner/example-repo/fix")
    );
    assert_eq!(
        config
            .identity_for_workspace_root(
                Path::new("/Users/example/src/.work/github.com/example-owner/example-repo/fix"),
                &environment,
            )
            .expect("managed workspace identity"),
        identity
    );
}

#[test]
fn layout_rejects_invalid_workspace_names() {
    // Verifies: Workspace names stay single safe path segments inside the hidden layout.
    let environment =
        RuntimeEnvironment::new("/tmp", [("HOME".to_owned(), "/Users/example".to_owned())]);
    let identity = RepositoryIdentity {
        source: "github".to_owned(),
        host: "github.com".to_owned(),
        owner: "example-owner".to_owned(),
        repo: "example-repo".to_owned(),
    };

    let error = LayoutConfig::default()
        .workspace_destination(&identity, "bad/name", &environment)
        .expect_err("workspace names are validated");

    assert!(matches!(
        error,
        RepositoryError::InvalidWorkspaceName { .. }
    ));
}

#[test]
fn loads_global_config_from_home_config_path() {
    // Verifies: Loads global config from home config path.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[repo]
reviewers = ["example-reviewer"]

[auth.keychain]
service = "jx-example"
account = "example-user"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context.config.paths,
        vec![workspace.path().join(".config/jx/config.toml")]
    );
    assert_eq!(
        context.config.repo.base.reviewers,
        reviewer_users(["example-reviewer"])
    );
    assert_eq!(
        context.token_source,
        TokenSource::Keychain(KeychainConfig {
            service: "jx-example".to_owned(),
            account: "example-user".to_owned(),
        })
    );
}

#[test]
fn loads_global_config_files_in_lexical_order() {
    // Verifies: Global config composes top-level ~/.config/jx/*.toml files in lexical order.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/00-base.toml",
        r#"
[repo]
reviewers = ["global-reviewer"]
"#,
    );
    workspace.write_file(
        ".config/jx/20-work.toml",
        r#"
[repo]
reviewers = ["work-reviewer"]
"#,
    );
    workspace.write_file(
        ".config/jx/10-auth.toml",
        r#"
[repo]
reviewers = ["team-reviewer", "global-reviewer"]

[auth.keychain]
service = "work-jx"
account = "work-user"
"#,
    );
    workspace.write_file(".config/jx/README.md", "ignored");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context.config.paths,
        vec![
            workspace.path().join(".config/jx/00-base.toml"),
            workspace.path().join(".config/jx/10-auth.toml"),
            workspace.path().join(".config/jx/20-work.toml"),
        ]
    );
    assert_eq!(
        context.config.repo.base.reviewers,
        reviewer_users(["global-reviewer", "team-reviewer", "work-reviewer"])
    );
    assert_eq!(
        context.token_source,
        TokenSource::Keychain(KeychainConfig {
            service: "work-jx".to_owned(),
            account: "work-user".to_owned(),
        })
    );
}

#[test]
fn path_reviewers_add_candidates_for_matching_repo_and_files() {
    // Verifies: Path reviewer config contributes candidates with pattern reasons for matching files.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/00-base.toml",
        r#"
[repo]
reviewers = ["global-reviewer"]

[[repo.rules]]
repo = "example-owner/*"
advance_trunk = true

[[repo.rules.path_reviewers]]
paths = ["foo/bar/**", "bar/bux/*.py"]
reviewers = ["foo-reviewer", "global-reviewer"]

[[repo.rules.path_reviewers]]
paths = ["docs/**"]
reviewers = ["docs-reviewer"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");
    assert!(context
        .config
        .repo
        .advance_trunk_enabled_for(&context.origin.github));
    let candidates = context.config.repo.reviewer_candidates_for(
        &context.origin.github,
        &[
            "foo/bar/baz.rs".to_owned(),
            "bar/bux/boo.py".to_owned(),
            "docs/readme.md".to_owned(),
        ],
    );

    assert_eq!(
        candidates,
        vec![
            reviewer_user_candidate(
                "foo-reviewer",
                ["foo/bar/** matched 1 file", "bar/bux/*.py matched 1 file"],
            ),
            reviewer_user_candidate(
                "global-reviewer",
                ["foo/bar/** matched 1 file", "bar/bux/*.py matched 1 file"],
            ),
            reviewer_user_candidate("docs-reviewer", ["docs/** matched 1 file"]),
        ]
    );
}

#[test]
fn reviewer_completion_uses_repo_level_matching_rules() {
    // Verifies: Reviewer completion offers base, wildcard, and exact repo reviewers without path-rule noise.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/00-base.toml",
        r#"
[repo]
reviewers = ["base-reviewer", "ExampleOrg/platform"]

[[repo.path_reviewers]]
paths = ["docs/**"]
reviewers = ["docs-owner"]

[[repo.rules]]
repo = "example-owner/*"
reviewers = ["area-reviewer", "base-reviewer"]

[[repo.rules]]
repo = "example-owner/example-repo"
reviewers = ["repo-reviewer", "ExampleOrg/platform"]

[[repo.rules]]
repo = "other-owner/*"
reviewers = ["external-reviewer"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context
            .config
            .repo
            .reviewer_completion_for(&context.origin.github),
        vec![
            ReviewerTarget::user("base-reviewer"),
            ReviewerTarget::team("ExampleOrg/platform", "platform"),
            ReviewerTarget::user("area-reviewer"),
            ReviewerTarget::user("repo-reviewer"),
        ]
    );
}

#[test]
fn stack_status_review_gate_checks_compose_for_matching_repo() {
    // Verifies: Stack status check classification can be scoped by repository policy.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo.stack_status]
ignored_checks = ["^global-noise-check$"]
ignored_labels = ["global-noise"]
ignored_labels_when_merged = ["global-merge-noise"]
ignored_reviewers = ["global-bot"]
review_gate_checks = ["global approval"]
review_wait_threshold = "8h"

[[repo.stack_status.title_rewrites]]
pattern = "^\\[([A-Z]+-[0-9]+)\\] (.+)$"
replace = "$1: $2"

[[repo.rules]]
repo = "example-owner/*"

[repo.rules.stack_status]
ignored_checks = ["^repo-noise-check.*"]
ignored_labels = ["repo-noise*"]
ignored_labels_when_merged = ["repo-merge-noise*"]
ignored_reviewers = ["repo-bot*"]
review_gate_checks = ["repo approval*"]
review_wait_threshold = "4h"

[[repo.rules.stack_status.title_rewrites]]
pattern = "^Draft: (.+)$"
replace = "$1"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");
    let stack_status = context.config.repo.stack_status_for(&context.origin.github);

    assert_eq!(
        stack_status.review_gate_checks,
        vec![
            ReviewGateCheckConfig {
                name: "global approval".to_owned(),
            },
            ReviewGateCheckConfig {
                name: "repo approval*".to_owned(),
            },
        ]
    );
    assert!(stack_status.review_gate_checks[1].matches("repo approval required"));
    assert!(stack_status.ignores_check("global-noise-check"));
    assert!(stack_status.ignores_check("repo-noise-check-required"));
    assert!(stack_status.ignores_label("global-noise"));
    assert!(stack_status.ignores_label("repo-noise-label"));
    assert!(stack_status.ignores_label_when_merged("global-merge-noise"));
    assert!(stack_status.ignores_label_when_merged("repo-merge-noise-label"));
    assert!(stack_status.ignores_reviewer("global-bot"));
    assert!(stack_status.ignores_reviewer("repo-bot-helper"));
    assert_eq!(
        stack_status.review_wait_threshold_seconds,
        Some(4 * 60 * 60)
    );
    assert_eq!(
        stack_status.rewrite_title("[TASK-123] Update endpoint"),
        "TASK-123: Update endpoint"
    );
    assert_eq!(
        stack_status.rewrite_title("Draft: Update endpoint"),
        "Update endpoint"
    );
}

#[test]
fn legacy_reviewer_rules_alias_still_loads_path_reviewers() {
    // Verifies: Existing config files keep working while new docs use path_reviewers.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.reviewer_rules]]
paths = ["src/**"]
reviewers = ["example-reviewer"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");
    let candidates = context
        .config
        .repo
        .reviewer_candidates_for(&context.origin.github, &["src/main.rs".to_owned()]);

    assert_eq!(
        candidates,
        vec![reviewer_user_candidate(
            "example-reviewer",
            ["src/** matched 1 file"]
        )]
    );
}

#[test]
fn repo_event_handlers_compose_and_override_for_matching_repo() {
    // Verifies: Event handlers compose by repo rule and can be disabled by handler id.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.event_handlers]]
id = "prepare-title"
on = "pull_request.prepare"
when = "has:task"
run = "prepend_task_id"

[[repo.event_handlers]]
id = "label-drafts"
on = "pull_request.created"
when = "is:draft -label:bar"
run = "add_labels"
labels = ["bar"]

[[repo.event_handlers]]
id = "open-unreviewed"
on = "pull_request.created"
when = "-has:reviewers -is:draft"
run = "open_pull_request"

[[repo.rules]]
repo = "example-owner/*"

[[repo.rules.event_handlers]]
id = "open-unreviewed"
enabled = false

[[repo.rules.event_handlers]]
id = "label-unreviewed"
on = "pull_request.created"
when = "-has:reviewers -label:buz"
run = "add_labels"
labels = ["buz", "buz"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");
    let handlers = context
        .config
        .repo
        .event_handlers_for(&context.origin.github);

    assert_eq!(handlers.len(), 3);
    assert_eq!(handlers[0].id.as_deref(), Some("prepare-title"));
    assert_eq!(handlers[0].on, RepoEvent::PullRequestPrepare);
    assert_eq!(handlers[0].when.terms.len(), 1);
    assert!(matches!(
        handlers[0].run,
        RepoEventHandlerRun::PrependTaskId
    ));
    assert_eq!(handlers[1].id.as_deref(), Some("label-drafts"));
    assert_eq!(handlers[1].on, RepoEvent::PullRequestCreated);
    assert_eq!(handlers[1].when.terms.len(), 2);
    assert!(matches!(
        &handlers[1].run,
        RepoEventHandlerRun::AddLabels { labels } if labels == &vec!["bar".to_owned()]
    ));
    assert_eq!(handlers[2].id.as_deref(), Some("label-unreviewed"));
    assert!(matches!(
        &handlers[2].run,
        RepoEventHandlerRun::AddLabels { labels } if labels == &vec!["buz".to_owned()]
    ));
}

#[test]
fn repo_checks_compose_override_and_match_changed_files() {
    // Verifies: Check commands compose by repo rule, replace by id, and run only for matching paths.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[[repo.checks]]
id = "global-docs"
before = ["pull_request"]
paths = ["docs/**"]
command = ["./check-docs"]

[[repo.rules]]
repo = "example-owner/*"

[[repo.rules.checks]]
id = "api-contract"
before = ["pull_request", "sync", "sync"]
paths = ["api/**"]
command = ["./scripts/check-api-contract"]
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.rules]]
repo = "example-owner/example-repo"

[[repo.rules.checks]]
id = "source-check"
before = ["push"]
paths = ["src/**"]
command = ["./scripts/check-source"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");
    let pull_request_checks = context.config.repo.checks_for(
        &context.origin.github,
        RepoCheckTrigger::PullRequest,
        &["docs/readme.md".to_owned(), "src/main.rs".to_owned()],
    );
    let push_checks = context.config.repo.checks_for(
        &context.origin.github,
        RepoCheckTrigger::Push,
        &["src/main.rs".to_owned()],
    );

    assert_eq!(pull_request_checks.len(), 1);
    assert_eq!(pull_request_checks[0].id, "global-docs");
    assert_eq!(push_checks.len(), 1);
    assert_eq!(push_checks[0].id, "source-check");
    assert_eq!(
        push_checks[0].command,
        vec!["./scripts/check-source".to_owned()]
    );
}

#[test]
fn repo_checks_reject_invalid_shape() {
    // Verifies: Check config requires explicit triggers, path globs, and command argv.
    let cases = [
        ("[[repo.checks]]\nid = \"missing-before\"\npaths = [\"src/**\"]\ncommand = [\"check\"]", "before"),
        ("[[repo.checks]]\nid = \"bad-before\"\nbefore = [\"commit\"]\npaths = [\"src/**\"]\ncommand = [\"check\"]", "unsupported trigger"),
        ("[[repo.checks]]\nid = \"missing-paths\"\nbefore = [\"push\"]\ncommand = [\"check\"]", "paths"),
        ("[[repo.checks]]\nid = \"bad-glob\"\nbefore = [\"push\"]\npaths = [\"[\"]\ncommand = [\"check\"]", "glob"),
        ("[[repo.checks]]\nid = \"missing-command\"\nbefore = [\"push\"]\npaths = [\"src/**\"]", "command"),
    ];

    for (contents, expected) in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx/config.toml", contents);
        let environment = RuntimeEnvironment::new(workspace.path(), []);

        let error =
            RepositoryContext::discover(&environment).expect_err("invalid check is rejected");

        assert!(error.to_string().contains(expected), "{contents}: {error}");
    }
}

#[test]
fn repo_event_handlers_reject_invalid_shape() {
    // Verifies: Handler config validates event names, actions, and query terms up front.
    let cases = [
        (
            "[[repo.event_handlers]]\non = \"pull_request.closed\"\nrun = \"open_pull_request\"",
            "pull_request.created",
        ),
        (
            "[[repo.event_handlers]]\non = \"pull_request.created\"\nwhen = \"no:reviewers\"\nrun = \"open_pull_request\"",
            "unsupported term",
        ),
        (
            "[[repo.event_handlers]]\non = \"pull_request.created\"\nrun = \"add_labels\"",
            "labels",
        ),
        (
            "[[repo.event_handlers]]\nid = \"missing-event\"\nenabled = false",
            "",
        ),
        (
            "[[repo.event_handlers]]\nenabled = false",
            "id",
        ),
    ];

    for (contents, expected) in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx/config.toml", contents);
        let environment = RuntimeEnvironment::new(workspace.path(), []);
        let result = RepositoryContext::discover(&environment);

        if expected.is_empty() {
            result.expect("disabled handler with id is accepted");
        } else {
            let error = result.expect_err("invalid event handler is rejected");
            assert!(error.to_string().contains(expected), "{contents}: {error}");
        }
    }
}

#[test]
fn reviewer_config_accepts_github_team_reviewers() {
    // Verifies: Reviewer entries can target GitHub teams with `org/team` syntax.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
reviewers = ["ExampleOrg/platform"]

[[repo.path_reviewers]]
paths = ["src/**"]
reviewers = ["Foo/bar"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");
    let candidates = context
        .config
        .repo
        .reviewer_candidates_for(&context.origin.github, &["src/main.rs".to_owned()]);

    assert_eq!(
        candidates,
        vec![reviewer_team_candidate(
            "Foo/bar",
            "bar",
            ["src/** matched 1 file"]
        )]
    );
}

#[test]
fn path_reviewers_ignore_other_repos_and_unmatched_files() {
    // Verifies: Path reviewer config is gated by repo slug and selected commit paths.
    let config = RepoConfig {
        base: RepoPolicyConfig {
            reviewers: reviewer_users(["global-reviewer"]),
            ..RepoPolicyConfig::default()
        },
        rules: vec![RepoRuleConfig {
            repo: "example-owner/example-repo".to_owned(),
            policy: RepoPolicyConfig {
                reviewer_rules: vec![ReviewerPathRule {
                    paths: vec!["foo/bar/**".to_owned()],
                    reviewers: reviewer_users(["foo-reviewer"]),
                }],
                ..RepoPolicyConfig::default()
            },
        }],
    };
    let other_repository = GitHubRepository {
        owner: "other-owner".to_owned(),
        name: "example-repo".to_owned(),
    };
    let matching_repository = GitHubRepository {
        owner: "example-owner".to_owned(),
        name: "example-repo".to_owned(),
    };

    assert_eq!(
        config.reviewer_candidates_for(&other_repository, &["foo/bar/baz.rs".to_owned()]),
        Vec::<ReviewerCandidate>::new()
    );
    assert_eq!(
        config.reviewer_candidates_for(&matching_repository, &["src/main.rs".to_owned()]),
        Vec::<ReviewerCandidate>::new()
    );
}

#[test]
fn workspace_shared_paths_compose_normalize_and_deduplicate_for_matching_repo() {
    // Verifies: Effective shared workspace paths preserve policy order after normalization and dedupe.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/00-base.toml",
        r#"
[repo]
workspace_shared_paths = [" .pi ", "./tools//state", ".pi"]

[[repo.rules]]
repo = "example-owner/*"
workspace_shared_paths = [" .agent/state "]

[[repo.rules]]
repo = "other-owner/*"
workspace_shared_paths = ["ignored"]
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
workspace_shared_paths = ["./.cache/jx", "tools/state"]

[[repo.rules]]
repo = "example-owner/example-repo"
workspace_shared_paths = [".local", ".cache/jx"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context
            .config
            .repo
            .workspace_shared_paths_for(&context.origin.github)
            .expect("shared paths resolve"),
        vec![".pi", "tools/state", ".cache/jx", ".agent/state", ".local"]
    );
}

#[test]
fn workspace_shared_paths_reject_invalid_path_shapes() {
    // Verifies: Shared workspace path config accepts only normalized repo-relative paths.
    let cases = [
        ("[repo]\nworkspace_shared_paths = [\"\"]", "empty paths"),
        (
            "[repo]\nworkspace_shared_paths = [\"/absolute\"]",
            "repo-relative",
        ),
        (
            "[repo]\nworkspace_shared_paths = [\"../outside\"]",
            "must not contain `..`",
        ),
        (
            "[repo]\nworkspace_shared_paths = [\"foo/../bar\"]",
            "must not contain `..`",
        ),
        ("[repo]\nworkspace_shared_paths = [\".\"]", "empty paths"),
        (
            "[repo]\nworkspace_shared_paths = [\"foo\\\\bar\"]",
            "forward slash",
        ),
        (
            "[repo]\nworkspace_shared_paths = [\"C:/absolute\"]",
            "repo-relative",
        ),
    ];

    for (contents, expected_message) in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx/config.toml", contents);
        let environment = RuntimeEnvironment::new(workspace.path(), []);

        let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

        assert!(matches!(error, RepositoryError::InvalidConfig { .. }));
        assert!(
            error.to_string().contains(expected_message),
            "{expected_message}: {error}"
        );
    }
}

#[test]
fn workspace_shared_paths_reject_parent_child_overlaps() {
    // Verifies: Shared workspace paths reject ambiguous parent/child ownership after dedupe.
    let cases = [
        "[repo]\nworkspace_shared_paths = [\"foo\", \"foo/bar\"]",
        r#"
[[repo.rules]]
repo = "example-owner/*"
workspace_shared_paths = ["foo", "foo/bar"]
"#,
    ];

    for contents in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx/config.toml", contents);
        let environment = RuntimeEnvironment::new(workspace.path(), []);

        let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

        assert!(matches!(error, RepositoryError::InvalidConfig { .. }));
        assert!(
            error.to_string().contains("overlapping paths"),
            "overlap should be rejected: {error}"
        );
    }
}

#[test]
fn workspace_shared_paths_reject_effective_parent_child_overlaps() {
    // Verifies: Matching repo policy rejects overlaps across base and matching rule layers.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
workspace_shared_paths = [".pi"]

[[repo.rules]]
repo = "example-owner/*"
workspace_shared_paths = [".pi/cache"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");
    let error = context
        .config
        .repo
        .workspace_shared_paths_for(&context.origin.github)
        .expect_err("effective policy is rejected");

    assert!(matches!(error, RepositoryError::InvalidConfig { .. }));
    assert!(error.to_string().contains("overlapping paths"));
}

#[test]
fn loads_global_config_file_symlinks() {
    // Verifies: Global config composition accepts symlinked top-level TOML files.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/00-base.toml",
        r#"
[repo]
reviewers = ["global-reviewer"]
"#,
    );
    let target = workspace.path().join("work-config.toml");
    fs::write(
        &target,
        r#"
[repo]
reviewers = ["work-reviewer"]
"#,
    )
    .expect("write symlink target");
    let config_dir = workspace.path().join(".config/jx");
    if let Err(error) = symlink_file(&target, &config_dir.join("10-work.toml")) {
        eprintln!("skipping symlink test because symlinks are unavailable: {error}");
        return;
    }
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context.config.paths,
        vec![
            workspace.path().join(".config/jx/00-base.toml"),
            workspace.path().join(".config/jx/10-work.toml"),
        ]
    );
    assert_eq!(
        context.config.repo.base.reviewers,
        reviewer_users(["global-reviewer", "work-reviewer"])
    );
}

#[test]
fn project_config_does_not_load_fragment_directory() {
    // Verifies: Per-workspace composition is intentionally limited to `.jx/config.toml`.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
reviewers = ["project-reviewer"]
"#,
    );
    workspace.write_file(
        ".jx/10-work.toml",
        r#"
[repo]
reviewers = ["work-reviewer"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context.config.paths,
        vec![workspace.path().join(".jx/config.toml")]
    );
    assert_eq!(
        context.config.repo.base.reviewers,
        reviewer_users(["project-reviewer"])
    );
}

#[test]
fn project_config_merges_reviewers_and_overrides_global_keychain() {
    // Verifies: Project config composes reviewers while scalar auth settings override globally.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[repo]
reviewers = ["global-reviewer"]

[auth.keychain]
service = "global-jx"
account = "global-user"
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
reviewers = ["example-reviewer"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context.config.paths,
        vec![
            workspace.path().join(".config/jx/config.toml"),
            workspace.path().join(".jx/config.toml"),
        ]
    );
    assert_eq!(
        context.config.repo.base.reviewers,
        reviewer_users(["global-reviewer", "example-reviewer"])
    );
    assert_eq!(
        context.token_source,
        TokenSource::Keychain(KeychainConfig {
            service: "global-jx".to_owned(),
            account: "global-user".to_owned(),
        })
    );
}

#[test]
fn project_keychain_config_overrides_global_keychain_config() {
    // Verifies: Project keychain config overrides global keychain config.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".config/jx/config.toml",
        r#"
[auth.keychain]
service = "global-jx"
account = "global-user"
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[auth.keychain]
service = "project-jx"
account = "project-user"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context.token_source,
        TokenSource::Keychain(KeychainConfig {
            service: "project-jx".to_owned(),
            account: "project-user".to_owned(),
        })
    );
}

#[test]
fn loads_keychain_token_source_from_project_config() {
    // Verifies: Loads keychain token source from project config.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[auth.keychain]
service = "jx-example"
account = "example-user"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context.token_source,
        TokenSource::Keychain(KeychainConfig {
            service: "jx-example".to_owned(),
            account: "example-user".to_owned(),
        })
    );
    assert_eq!(
        context.token_source.summary(),
        "keychain account `example-user` for service `jx-example`"
    );
}

#[test]
fn environment_token_overrides_configured_keychain() {
    // Verifies: Environment token overrides configured keychain.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[auth.keychain]
service = "jx-example"
account = "example-user"
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(context.token_source, TokenSource::Environment("GH_TOKEN"));
}

#[test]
fn trims_and_deduplicates_default_reviewers() {
    // Verifies: Trims and deduplicates default reviewers.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
reviewers = [" example-reviewer ", "second-reviewer", "example-reviewer"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context.config.repo.base.reviewers,
        reviewer_users(["example-reviewer", "second-reviewer"])
    );
}

#[test]
fn rejects_configured_remotes_legacy_hooks_and_bookmark_roots() {
    // Verifies: Config parsing rejects configured remotes, legacy hook tables, and bookmark roots.
    let cases = [
            ("default_remote = \"origin\"", "default_remote"),
            ("reviewers = []", "reviewers"),
            ("[hooks]\npre_pr = []", "hooks"),
            ("bookmark_root = \"example-user\"", "bookmark_root"),
            ("[remotes]\ndefault = \"origin\"", "remotes"),
            ("[auth]\ntoken = \"placeholder-token\"", "auth.token"),
            (
                "[auth.keychain]\nservice = \"jx-example\"\naccount = \"example-user\"\nlabel = \"GitHub\"",
                "auth.keychain.label",
            ),
            ("[diff]\nrenderer = \"difft\"", "diff.renderer"),
            (
                "[diff.tools.difft]\nmode = \"external\"\ncommand = \"difft\"\nproducer_args = []",
                "diff.tools.difft.producer_args",
            ),
        ];

    for (contents, key) in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx/config.toml", contents);
        let environment = RuntimeEnvironment::new(workspace.path(), []);

        let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

        assert!(
            matches!(error, RepositoryError::UnsupportedConfigKey { key: ref rejected, .. } if rejected == key),
            "{key}: {error}"
        );
    }
}

#[test]
fn rejects_invalid_diff_config() {
    // Verifies: Diff tools require valid mode, command, and default references.
    let cases = [
        ("[diff]\ndefault_tool = \"missing\"", "diff.default_tool"),
        (
            "[diff.tools.difft]\nmode = \"unknown\"\ncommand = \"difft\"",
            "diff.tools.difft.mode",
        ),
        (
            "[diff.tools.difft]\nmode = \"external\"",
            "diff.tools.difft.command",
        ),
        (
            "[diff.tools.delta]\nmode = \"pipe\"\ncommand = \"delta\"\nargs = [\"\"]",
            "diff.tools.delta.args",
        ),
    ];

    for (contents, expected_message) in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx/config.toml", contents);
        let environment = RuntimeEnvironment::new(workspace.path(), []);

        let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

        assert!(
            matches!(error, RepositoryError::InvalidConfig { ref message, .. } if message.contains(expected_message)),
            "{expected_message}: {error}"
        );
    }
}

#[test]
fn rejects_unknown_reviewer_rule_keys() {
    // Verifies: Config parsing rejects unknown reviewer rule keys.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.path_reviewers]]
paths = ["src/**"]
reviewers = ["example-reviewer"]
teams = ["example-team"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

    assert!(matches!(
        error,
        RepositoryError::UnsupportedConfigKey { key, .. } if key == "repo.path_reviewers[0].teams"
    ));
}

#[test]
fn rejects_invalid_reviewer_rule_config() {
    // Verifies: Reviewer rules require valid repo, path, and reviewer lists.
    let cases = [
        (
            "[[repo.rules]]\nrepo = \"example-owner\"\nreviewers = [\"example-reviewer\"]",
            "repo.rules[0].repo",
        ),
        (
            "[[repo.path_reviewers]]\npaths = []\nreviewers = [\"example-reviewer\"]",
            "repo.path_reviewers[0].paths",
        ),
        (
            "[[repo.path_reviewers]]\npaths = [\"src/**\"]",
            "repo.path_reviewers[0].reviewers",
        ),
        (
            "[[repo.path_reviewers]]\npaths = [\"src/**\"]\nreviewers = []",
            "repo.path_reviewers[0].reviewers",
        ),
    ];

    for (contents, expected_message) in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx/config.toml", contents);
        let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

        let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

        assert!(
            error.to_string().contains(expected_message),
            "{expected_message}: {error}"
        );
    }
}

#[test]
fn rejects_invalid_keychain_config() {
    // Verifies: Config parsing rejects invalid keychain settings.
    let cases = [
        ("[auth]\nkeychain = true", "auth.keychain"),
        (
            "[auth.keychain]\naccount = \"example-user\"",
            "auth.keychain.service",
        ),
        (
            "[auth.keychain]\nservice = \"jx-example\"",
            "auth.keychain.account",
        ),
        (
            "[auth.keychain]\nservice = \"\"\naccount = \"example-user\"",
            "auth.keychain.service",
        ),
        (
            "[auth.keychain]\nservice = \"jx-example\"\naccount = 1",
            "auth.keychain.account",
        ),
    ];

    for (contents, expected_message) in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx/config.toml", contents);
        let environment = RuntimeEnvironment::new(workspace.path(), []);

        let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

        assert!(matches!(error, RepositoryError::InvalidConfig { .. }));
        assert!(
            error.to_string().contains(expected_message),
            "{expected_message}: {error}"
        );
    }
}

#[test]
fn rejects_invalid_default_reviewer_shape() {
    // Verifies: Config parsing rejects invalid default reviewer shapes.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx/config.toml",
        r#"
[repo]
reviewers = "example-reviewer"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

    assert!(matches!(error, RepositoryError::InvalidConfig { .. }));
    assert!(error.to_string().contains("reviewers"));
}

#[test]
fn rejects_invalid_reviewer_names() {
    // Verifies: Reviewer names are either GitHub logins or one-level `org/team` targets.
    let cases = [
        ("[repo]\nreviewers = [\"/bar\"]", "reviewer name"),
        ("[repo]\nreviewers = [\"Foo/bar/baz\"]", "org/team"),
    ];

    for (contents, expected_message) in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx/config.toml", contents);
        let environment = RuntimeEnvironment::new(workspace.path(), []);

        let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

        assert!(matches!(error, RepositoryError::InvalidConfig { .. }));
        assert!(
            error.to_string().contains(expected_message),
            "{expected_message}: {error}"
        );
    }
}

#[test]
fn rejects_missing_origin() {
    // Verifies: Repository discovery rejects workspaces without an origin remote.
    let workspace = TestWorkspace::new();
    workspace.write_git_config("");
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let error = RepositoryContext::discover(&environment).expect_err("origin is required");

    assert!(matches!(error, RepositoryError::MissingOrigin));
}

#[test]
fn rejects_non_github_origin() {
    // Verifies: Repository discovery rejects non-GitHub origin URLs.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://example.invalid/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let error = RepositoryContext::discover(&environment).expect_err("github origin is required");

    assert!(matches!(error, RepositoryError::OriginNotGitHub { .. }));
}

#[test]
fn parses_supported_github_url_shapes() {
    // Verifies: GitHub URL parsing accepts supported HTTPS and SSH remote shapes.
    let cases = [
        "https://github.com/example-owner/example-repo.git",
        "https://github.com/example-owner/example-repo",
        "git@github.com:example-owner/example-repo.git",
        "ssh://git@github.com/example-owner/example-repo.git",
        "ssh://github.com/example-owner/example-repo.git",
    ];

    for case in cases {
        let repository = GitHubRepository::parse(case).expect(case);

        assert_eq!(repository.slug(), "example-owner/example-repo");
    }
}

#[test]
fn rejects_unsupported_github_url_shapes() {
    // Verifies: GitHub URL parsing rejects unsupported remote shapes.
    let cases = [
        "https://example.invalid/example-owner/example-repo.git",
        "https://github.com/example-owner",
        "https://github.com/example-owner/example repo.git",
        "https://github.com/example-owner/example-repo/extra.git",
    ];

    for case in cases {
        assert!(GitHubRepository::parse(case).is_err(), "{case}");
    }
}

fn origin_config() -> &'static str {
    r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#
}

fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
}

fn test_settings() -> UserSettings {
    UserSettings::from_config(StackedConfig::with_defaults()).expect("test settings")
}

fn reviewer_users(names: impl IntoIterator<Item = &'static str>) -> Vec<ReviewerTarget> {
    names.into_iter().map(ReviewerTarget::user).collect()
}

fn reviewer_user_candidate(
    name: &'static str,
    reasons: impl IntoIterator<Item = &'static str>,
) -> ReviewerCandidate {
    ReviewerCandidate::new(
        ReviewerTarget::user(name),
        reasons.into_iter().map(str::to_owned).collect(),
    )
}

fn reviewer_team_candidate(
    name: &'static str,
    slug: &'static str,
    reasons: impl IntoIterator<Item = &'static str>,
) -> ReviewerCandidate {
    ReviewerCandidate::new(
        ReviewerTarget::team(name, slug),
        reasons.into_iter().map(str::to_owned).collect(),
    )
}

fn test_config_remotes(contents: &str) -> Vec<ConfiguredRemote> {
    let mut current_remote = None;
    let mut remotes: Vec<ConfiguredRemote> = Vec::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            current_remote = line
                .strip_prefix(r#"[remote "#)
                .and_then(|section| section.strip_prefix('"'))
                .and_then(|section| section.strip_suffix(r#""]"#))
                .map(str::to_owned);
            continue;
        }
        let Some(remote_name) = current_remote.as_deref() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        if key.trim() == "url" {
            remotes.push(ConfiguredRemote {
                name: remote_name.to_owned(),
                url: value.trim().trim_matches('"').to_owned(),
            });
        }
    }

    remotes
}

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let unique = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!("jx-test-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root).expect("create workspace root");
        let settings = test_settings();
        pollster::block_on(Workspace::init_internal_git(&settings, &root))
            .expect("initialize jj workspace");
        Self { root }
    }

    fn path(&self) -> PathBuf {
        self.root.clone()
    }

    fn create_dir(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(&path).expect("create directory");
        path
    }

    fn home_environment(&self) -> [(String, String); 1] {
        [("HOME".to_owned(), self.root.to_string_lossy().into_owned())]
    }

    fn write_file(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, contents).expect("write file");
    }

    fn write_git_config(&self, contents: &str) {
        for remote in test_config_remotes(contents) {
            let settings = test_settings();
            let store_factories = StoreFactories::default();
            let working_copy_factories = default_working_copy_factories();
            let workspace = Workspace::load(
                &settings,
                &self.root,
                &store_factories,
                &working_copy_factories,
            )
            .expect("load jj workspace");
            let repo =
                pollster::block_on(workspace.repo_loader().load_at_head()).expect("load jj repo");
            let mut tx = repo.start_transaction();

            git::add_remote(
                tx.repo_mut(),
                RemoteName::new(&remote.name),
                &remote.url,
                None,
                gix::remote::fetch::Tags::None,
            )
            .expect("add remote");
            pollster::block_on(tx.commit(format!("arrange test remote {}", remote.name)))
                .expect("commit remote");
        }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
