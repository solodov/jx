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
        ".jx.toml",
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
        vec![workspace.path().join(".jx.toml")]
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
    // Verifies: Workspace metadata stays local by ignoring the metadata directory.
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
        "*\n"
    );
    assert_eq!(
        read_workspace_metadata(&workspace.path()).expect("metadata reads"),
        WorkspaceMetadata {
            task_id: Some("ABC-123".to_owned()),
        }
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
        ".jx.toml",
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
        ".jx.toml",
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
        ".jx.toml",
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
fn reviewer_rules_add_candidates_for_matching_repo_and_files() {
    // Verifies: Reviewer rules contribute candidates with pattern reasons for matching files.
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

[[repo.rules.reviewer_rules]]
paths = ["foo/bar/**", "bar/bux/*.py"]
reviewers = ["foo-reviewer", "global-reviewer"]

[[repo.rules.reviewer_rules]]
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
                "global-reviewer",
                [
                    "repo",
                    "foo/bar/** matched 1 file",
                    "bar/bux/*.py matched 1 file",
                ],
            ),
            reviewer_user_candidate(
                "foo-reviewer",
                ["foo/bar/** matched 1 file", "bar/bux/*.py matched 1 file"],
            ),
            reviewer_user_candidate("docs-reviewer", ["docs/** matched 1 file"]),
        ]
    );
}

#[test]
fn reviewer_config_accepts_github_team_reviewers() {
    // Verifies: Reviewer entries can target GitHub teams with `org/team` syntax.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx.toml",
        r#"
[repo]
reviewers = ["ExampleOrg/platform"]

[[repo.reviewer_rules]]
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
        vec![
            reviewer_team_candidate("ExampleOrg/platform", "platform", ["repo"]),
            reviewer_team_candidate("Foo/bar", "bar", ["src/** matched 1 file"]),
        ]
    );
}

#[test]
fn reviewer_rules_ignore_other_repos_and_unmatched_files() {
    // Verifies: Reviewer rules are gated by repo slug and selected commit paths.
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
        vec![reviewer_user_candidate("global-reviewer", ["repo"])]
    );
    assert_eq!(
        config.reviewer_candidates_for(&matching_repository, &["src/main.rs".to_owned()]),
        vec![reviewer_user_candidate("global-reviewer", ["repo"])]
    );
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
    // Verifies: Per-workspace composition is intentionally limited to `.jx.toml`.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(origin_config());
    workspace.write_file(
        ".jx.toml",
        r#"
[repo]
reviewers = ["project-reviewer"]
"#,
    );
    workspace.write_file(
        ".jx.d/10-work.toml",
        r#"
[repo]
reviewers = ["work-reviewer"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let context = RepositoryContext::discover(&environment).expect("context discovers");

    assert_eq!(
        context.config.paths,
        vec![workspace.path().join(".jx.toml")]
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
        ".jx.toml",
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
            workspace.path().join(".jx.toml"),
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
        ".jx.toml",
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
        ".jx.toml",
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
        ".jx.toml",
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
        ".jx.toml",
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
fn rejects_configured_remotes_hooks_and_bookmark_roots() {
    // Verifies: Config parsing rejects configured remotes, hooks, and bookmark roots.
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
        workspace.write_file(".jx.toml", contents);
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
        workspace.write_file(".jx.toml", contents);
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
        ".jx.toml",
        r#"
[[repo.reviewer_rules]]
paths = ["src/**"]
reviewers = ["example-reviewer"]
teams = ["example-team"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);

    let error = RepositoryContext::discover(&environment).expect_err("config is rejected");

    assert!(matches!(
        error,
        RepositoryError::UnsupportedConfigKey { key, .. } if key == "repo.reviewer_rules[0].teams"
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
            "[[repo.reviewer_rules]]\npaths = []\nreviewers = [\"example-reviewer\"]",
            "repo.reviewer_rules[0].paths",
        ),
        (
            "[[repo.reviewer_rules]]\npaths = [\"src/**\"]",
            "repo.reviewer_rules[0].reviewers",
        ),
        (
            "[[repo.reviewer_rules]]\npaths = [\"src/**\"]\nreviewers = []",
            "repo.reviewer_rules[0].reviewers",
        ),
    ];

    for (contents, expected_message) in cases {
        let workspace = TestWorkspace::new();
        workspace.write_git_config(origin_config());
        workspace.write_file(".jx.toml", contents);
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
        workspace.write_file(".jx.toml", contents);
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
        ".jx.toml",
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
        workspace.write_file(".jx.toml", contents);
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
