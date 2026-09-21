use super::*;

#[test]
fn action_layers_preserve_order_replace_whole_definitions_and_keep_local_overrides_last() {
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/10-base.toml",
        r#"
[[repo.stack_status_actions]]
id = "open"
title = "Global open"
command = ["global", "{pr_url}"]
cwd = "caller"
[[repo.stack_status_actions]]
id = "diff"
title = "Global diff"
command = ["jj", "diff", "-r", "{local_change_id}"]
[[repo.stack_status_actions]]
id = "remove"
title = "Remove me"
command = ["old"]
[[repo.stack_status_actions]]
id = "global-only"
title = "Global only"
command = ["global-only"]
[[repo.rules]]
repo = "owner/*"
[[repo.rules.stack_status_actions]]
id = "open"
title = "Wildcard open"
command = ["wildcard"]
cwd = "caller"
"#,
    );
    workspace.write_file(
        ".config/jx/20-overrides.toml",
        r#"
[[repo.stack_status_actions]]
id = "open"
title = "Later default"
command = ["later-default"]
[[repo.rules]]
repo = "  owner/repo  "
[[repo.rules.stack_status_actions]]
id = "open"
title = "Specific open"
command = ["specific"]
cwd = "caller"
[[repo.rules.stack_status_actions]]
id = "rule-added"
title = "Rule added"
command = ["rule-added"]
[[repo.rules]]
repo = "unrelated/*"
[[repo.rules.stack_status_actions]]
id = "open"
title = "Wrong repo"
command = ["wrong"]
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.stack_status_actions]]
id = "open"
title = "Local open"
command = ["local", "{pr_number}"]
[[repo.stack_status_actions]]
id = "remove"
enabled = false
[[repo.stack_status_actions]]
id = "local-added"
title = "Local added"
command = ["local-added"]
[[repo.rules]]
repo = "owner/repo"
[[repo.rules.stack_status_actions]]
id = "diff"
title = "Local diff"
command = ["local-diff"]
[[repo.rules.stack_status_actions]]
id = "rule-added"
enabled = false
[[repo.rules.stack_status_actions]]
id = "local-rule"
title = "Local rule"
command = ["local-rule"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repo = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    let global = WorkflowConfig::discover_global(&environment).unwrap();
    let actions = global.stack_status_actions.for_repository(&repo);
    assert_eq!(actions[0].action.title, "Specific open");
    assert_eq!(actions[0].source.scope, PrActionConfigScope::Global);
    assert!(actions[0].source.path.ends_with("20-overrides.toml"));
    let other = global
        .stack_status_actions
        .for_repository(&GitHubRepository::parse("https://github.com/owner/other").unwrap());
    assert_eq!(other[0].action.title, "Wildcard open");

    let config = WorkflowConfig::discover(&environment).unwrap();
    let actions = config.stack_status_actions.for_repository(&repo);
    assert_eq!(
        actions
            .iter()
            .map(|resolved| resolved.action.id.as_str())
            .collect::<Vec<_>>(),
        ["open", "diff", "global-only", "local-added", "local-rule"]
    );
    assert_eq!(actions[0].action.title, "Local open");
    assert_eq!(actions[0].action.command, ["local", "{pr_number}"]);
    assert_eq!(actions[0].action.cwd, PrActionWorkingDirectory::Repository);
    assert_eq!(actions[1].action.title, "Local diff");
    for index in [0, 1, 3, 4] {
        assert_eq!(actions[index].source.scope, PrActionConfigScope::Repository);
        assert_eq!(
            actions[index].source.path,
            workspace.path().join(".jx/config.toml")
        );
    }
    assert_eq!(actions[2].source.scope, PrActionConfigScope::Global);
    assert!(actions[2].source.path.ends_with("10-base.toml"));
    assert_eq!(config.stack_status_actions.for_repository(&repo), actions);
    assert!(config.review_actions.for_repository(&repo).is_empty());
}

#[test]
fn action_sets_merge_overrides_and_disables_without_crossing_namespaces() {
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/actions.toml",
        r#"
[[repo.review_actions]]
id = "open"
title = "Global review"
command = ["global-review"]
cwd = "caller"
[[repo.review_actions]]
id = "keep"
title = "Keep review"
command = ["keep-review"]
[[repo.stack_status_actions]]
id = "open"
title = "Global stack"
command = ["global-stack"]
[[repo.stack_status_actions]]
id = "keep"
title = "Keep stack"
command = ["keep-stack"]
[[repo.rules]]
repo = "owner/*"
[[repo.rules.review_actions]]
id = "open"
title = "Rule review"
command = ["rule-review"]
cwd = "caller"
[[repo.rules.stack_status_actions]]
id = "keep"
enabled = false
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.review_actions]]
id = "open"
title = "Local review"
command = ["local-review"]
[[repo.stack_status_actions]]
id = "keep"
title = "Local keep stack"
command = ["local-keep-stack"]
[[repo.rules]]
repo = "owner/repo"
[[repo.rules.review_actions]]
id = "keep"
enabled = false
[[repo.rules.stack_status_actions]]
id = "open"
title = "Local stack"
command = ["local-stack"]
cwd = "caller"
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repo = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    let global = WorkflowConfig::discover_global(&environment).unwrap();
    let review = global.review_actions.for_repository(&repo);
    assert_eq!(review.len(), 2);
    assert_eq!(review[0].action.command, ["rule-review"]);
    assert_eq!(review[1].action.command, ["keep-review"]);
    let stack = global.stack_status_actions.for_repository(&repo);
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0].action.command, ["global-stack"]);

    let local = WorkflowConfig::discover(&environment).unwrap();
    let review = local.review_actions.for_repository(&repo);
    let stack = local.stack_status_actions.for_repository(&repo);
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].action.command, ["local-review"]);
    assert_eq!(review[0].action.cwd, PrActionWorkingDirectory::Repository);
    assert_eq!(
        stack
            .iter()
            .map(|entry| entry.action.id.as_str())
            .collect::<Vec<_>>(),
        ["open", "keep"]
    );
    assert_eq!(stack[0].action.command, ["local-stack"]);
    assert_eq!(stack[0].action.cwd, PrActionWorkingDirectory::Caller);
    assert_eq!(stack[1].action.command, ["local-keep-stack"]);
    for action in review.iter().chain(&stack) {
        assert_eq!(action.source.scope, PrActionConfigScope::Repository);
        assert_eq!(action.source.path, workspace.path().join(".jx/config.toml"));
    }
}

#[test]
fn an_unconfigured_action_set_stays_empty() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repo = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    for action_set in ["review_actions", "stack_status_actions"] {
        workspace.write_file(
            ".jx/config.toml",
            &format!("[[repo.{action_set}]]\nid='open'\ntitle='Open'\ncommand=['open']\n"),
        );
        let config = WorkflowConfig::discover(&environment).unwrap();
        assert_eq!(
            config.review_actions.for_repository(&repo).len(),
            usize::from(action_set == "review_actions")
        );
        assert_eq!(
            config.stack_status_actions.for_repository(&repo).len(),
            usize::from(action_set == "stack_status_actions")
        );
    }
}

#[test]
fn review_actions_can_request_local_refresh_and_overrides_replace_the_policy() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repo = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    workspace.write_file(".config/jx/actions.toml", "[[repo.review_actions]]\nid='dismiss'\ntitle='Dismiss'\ncommand=['jx', 'review', '--cached', 'dismiss', '{pr_url}']\non_success='refresh-local'\n");
    let config = WorkflowConfig::discover(&environment).unwrap();
    assert_eq!(
        config.review_actions.for_repository(&repo)[0]
            .action
            .on_success,
        PrActionOnSuccess::RefreshLocal
    );

    workspace.write_file(".jx/config.toml", "[[repo.rules]]\nrepo='owner/*'\n[[repo.rules.review_actions]]\nid='dismiss'\ntitle='Dismiss live'\ncommand=['jx', 'review', 'dismiss', '{pr_url}']\n");
    let config = WorkflowConfig::discover(&environment).unwrap();
    assert_eq!(
        config.review_actions.for_repository(&repo)[0]
            .action
            .on_success,
        PrActionOnSuccess::Refresh
    );
}

#[test]
fn local_refresh_is_rejected_for_stack_actions_in_defaults_and_rules() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    for (prefix, key) in [
        ("", "repo.stack_status_actions"),
        (
            "[[repo.rules]]\nrepo='owner/*'\n",
            "repo.rules.stack_status_actions",
        ),
    ] {
        workspace.write_file(
            ".jx/config.toml",
            &format!(
                "{prefix}[[{key}]]\nid='x'\ntitle='X'\ncommand=['x']\non_success='refresh-local'\n"
            ),
        );
        let error = WorkflowConfig::discover(&environment)
            .unwrap_err()
            .to_string();
        assert!(error.contains("on_success"));
        assert!(error.contains("review actions"));
    }
}

#[test]
fn action_config_rejects_invalid_definitions_and_duplicate_ids_in_each_list() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    for action_set in ["review_actions", "stack_status_actions"] {
        for body in [
            "id = 'x'\ntitle = 'X'\ncommand = 'echo hello'",
            "id = 'x'\ntitle = 'X'\ncommand = []",
            "id = 'x'\ntitle = 'X'\ncommand = ['  ']",
            "id = 'x'\ntitle = 'X'\ncommand = ['echo', 7]",
            "id = 'x'\ntitle = 'X'\ncommand = ['echo', '{unknown}']",
            "id = 'x'\ntitle = 'X'\ncommand = ['echo', '{title']",
            "id = 'x'\ntitle = 'X'\ncommand = ['echo', 'title}']",
            "id = 'x'\ntitle = 'X'\ncommand = ['echo']\ncwd = 'other'",
            "id = 'x'\ntitle = 'X'\ncommand = ['echo']\non_success = 'other'",
            "id = 'x'\ntitle = 'X'\ncommand = ['echo']\non_success = true",
            "id = 'x'\nenabled = false\non_success = 'refresh'",
            "id = 'x'\ntitle = 'X'\ncommand = ['echo']\nenabled = 'no'",
            "id = 'x'\nenabled = false\ncommand = ['echo']",
            "id = 'x'\ntitle = 'X'\ncommand = ['echo']\nextra = true",
            "id = ''\ntitle = 'X'\ncommand = ['echo']",
            "id = 'x'\ntitle = ''\ncommand = ['echo']",
            "title = 'X'\ncommand = ['echo']",
            "id = 'x'\ncommand = ['echo']",
        ] {
            workspace.write_file(
                ".jx/config.toml",
                &format!("[[repo.{action_set}]]\n{body}\n"),
            );
            assert!(
                WorkflowConfig::discover(&environment).is_err(),
                "{action_set}: {body}"
            );
        }
        for prefix in ["", "[[repo.rules]]\nrepo = 'owner/*'\n"] {
            let path = if prefix.is_empty() {
                format!("repo.{action_set}")
            } else {
                format!("repo.rules.{action_set}")
            };
            workspace.write_file(".jx/config.toml", &format!("{prefix}[[{path}]]\nid='same'\nenabled=false\n[[{path}]]\nid='same'\ntitle='Again'\ncommand=['echo']\n"));
            let error = WorkflowConfig::discover(&environment)
                .unwrap_err()
                .to_string();
            assert!(error.contains("duplicate action id"), "{error}");
            assert!(error.contains(action_set), "{error}");
        }
        workspace.write_file(
            ".jx/config.toml",
            &format!("[repo.{action_set}]\nid='not-an-array'\n"),
        );
        assert!(WorkflowConfig::discover(&environment).is_err());
    }
    for old_config in [
        "[[repo.actions]]\nid='x'\nenabled=false\n",
        "[[repo.rules]]\nrepo='owner/*'\n[[repo.rules.actions]]\nid='x'\nenabled=false\n",
    ] {
        workspace.write_file(".jx/config.toml", old_config);
        assert!(matches!(
            WorkflowConfig::discover(&environment),
            Err(RepositoryError::UnsupportedConfigKey { .. })
        ));
    }
}

#[test]
fn action_disables_can_be_reintroduced_and_templates_allow_literal_braces_and_empty_arguments() {
    let workspace = TestWorkspace::new();
    workspace.write_file(".config/jx/base.toml", "[[repo.review_actions]]\nid='x'\ntitle='Old'\ncommand=['old']\n[[repo.rules]]\nrepo='owner/*'\n[[repo.rules.review_actions]]\nid='x'\nenabled=false\n");
    workspace.write_file(".jx/config.toml", "[[repo.review_actions]]\nid='x'\ntitle='New'\ncommand=['echo', '{{literal}}', '', '{head_oid}']\n");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repo = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    assert!(WorkflowConfig::discover_global(&environment)
        .unwrap()
        .review_actions
        .for_repository(&repo)
        .is_empty());
    let actions = WorkflowConfig::discover(&environment)
        .unwrap()
        .review_actions
        .for_repository(&repo);
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0].action.command,
        ["echo", "{{literal}}", "", "{head_oid}"]
    );
    assert_eq!(actions[0].source.scope, PrActionConfigScope::Repository);
}
