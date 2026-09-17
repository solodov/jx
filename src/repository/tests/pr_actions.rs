use super::*;

#[test]
fn action_layers_preserve_order_replace_whole_definitions_and_keep_local_overrides_last() {
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".config/jx/10-base.toml",
        r#"
[[repo.actions]]
id = "open"
title = "Global open"
command = ["global", "{pr_url}"]
cwd = "caller"
[[repo.actions]]
id = "diff"
title = "Global diff"
command = ["jj", "diff", "-r", "{local_change_id}"]
[[repo.actions]]
id = "remove"
title = "Remove me"
command = ["old"]
[[repo.actions]]
id = "global-only"
title = "Global only"
command = ["global-only"]
[[repo.rules]]
repo = "owner/*"
[[repo.rules.actions]]
id = "open"
title = "Wildcard open"
command = ["wildcard"]
cwd = "caller"
"#,
    );
    workspace.write_file(
        ".config/jx/20-overrides.toml",
        r#"
[[repo.actions]]
id = "open"
title = "Later default"
command = ["later-default"]
[[repo.rules]]
repo = "  owner/repo  "
[[repo.rules.actions]]
id = "open"
title = "Specific open"
command = ["specific"]
cwd = "caller"
[[repo.rules.actions]]
id = "rule-added"
title = "Rule added"
command = ["rule-added"]
[[repo.rules]]
repo = "unrelated/*"
[[repo.rules.actions]]
id = "open"
title = "Wrong repo"
command = ["wrong"]
"#,
    );
    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.actions]]
id = "open"
title = "Local open"
command = ["local", "{pr_number}"]
[[repo.actions]]
id = "remove"
enabled = false
[[repo.actions]]
id = "local-added"
title = "Local added"
command = ["local-added"]
[[repo.rules]]
repo = "owner/repo"
[[repo.rules.actions]]
id = "diff"
title = "Local diff"
command = ["local-diff"]
[[repo.rules.actions]]
id = "rule-added"
enabled = false
[[repo.rules.actions]]
id = "local-rule"
title = "Local rule"
command = ["local-rule"]
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repo = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    let global = WorkflowConfig::discover_global(&environment)
        .unwrap()
        .actions
        .for_repository(&repo);
    assert_eq!(global[0].action.title, "Specific open");
    assert_eq!(global[0].source.scope, PrActionConfigScope::Global);
    assert!(global[0].source.path.ends_with("20-overrides.toml"));
    let other = WorkflowConfig::discover_global(&environment)
        .unwrap()
        .actions
        .for_repository(&GitHubRepository::parse("https://github.com/owner/other").unwrap());
    assert_eq!(other[0].action.title, "Wildcard open");

    let config = WorkflowConfig::discover(&environment).unwrap();
    let actions = config.actions.for_repository(&repo);
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
    assert_eq!(config.actions.for_repository(&repo), actions);
}

#[test]
fn action_config_rejects_invalid_definitions_and_duplicate_ids_in_each_list() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    for body in [
        "id = 'x'\ntitle = 'X'\ncommand = 'echo hello'",
        "id = 'x'\ntitle = 'X'\ncommand = []",
        "id = 'x'\ntitle = 'X'\ncommand = ['  ']",
        "id = 'x'\ntitle = 'X'\ncommand = ['echo', 7]",
        "id = 'x'\ntitle = 'X'\ncommand = ['echo', '{unknown}']",
        "id = 'x'\ntitle = 'X'\ncommand = ['echo', '{title']",
        "id = 'x'\ntitle = 'X'\ncommand = ['echo', 'title}']",
        "id = 'x'\ntitle = 'X'\ncommand = ['echo']\ncwd = 'other'",
        "id = 'x'\ntitle = 'X'\ncommand = ['echo']\nenabled = 'no'",
        "id = 'x'\nenabled = false\ncommand = ['echo']",
        "id = 'x'\ntitle = 'X'\ncommand = ['echo']\nextra = true",
        "id = ''\ntitle = 'X'\ncommand = ['echo']",
        "id = 'x'\ntitle = ''\ncommand = ['echo']",
        "title = 'X'\ncommand = ['echo']",
        "id = 'x'\ncommand = ['echo']",
    ] {
        workspace.write_file(".jx/config.toml", &format!("[[repo.actions]]\n{body}\n"));
        assert!(WorkflowConfig::discover(&environment).is_err(), "{body}");
    }
    for prefix in ["", "[[repo.rules]]\nrepo = 'owner/*'\n"] {
        let path = if prefix.is_empty() {
            "repo.actions"
        } else {
            "repo.rules.actions"
        };
        workspace.write_file(".jx/config.toml", &format!("{prefix}[[{path}]]\nid='same'\nenabled=false\n[[{path}]]\nid='same'\ntitle='Again'\ncommand=['echo']\n"));
        let error = WorkflowConfig::discover(&environment)
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate action id"), "{error}");
    }
    workspace.write_file(".jx/config.toml", "[repo.actions]\nid='not-an-array'\n");
    assert!(WorkflowConfig::discover(&environment).is_err());
}

#[test]
fn action_disables_can_be_reintroduced_and_templates_allow_literal_braces_and_empty_arguments() {
    let workspace = TestWorkspace::new();
    workspace.write_file(".config/jx/base.toml", "[[repo.actions]]\nid='x'\ntitle='Old'\ncommand=['old']\n[[repo.rules]]\nrepo='owner/*'\n[[repo.rules.actions]]\nid='x'\nenabled=false\n");
    workspace.write_file(".jx/config.toml", "[[repo.actions]]\nid='x'\ntitle='New'\ncommand=['echo', '{{literal}}', '', '{head_oid}']\n");
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repo = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    assert!(WorkflowConfig::discover_global(&environment)
        .unwrap()
        .actions
        .for_repository(&repo)
        .is_empty());
    let actions = WorkflowConfig::discover(&environment)
        .unwrap()
        .actions
        .for_repository(&repo);
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0].action.command,
        ["echo", "{{literal}}", "", "{head_oid}"]
    );
    assert_eq!(actions[0].source.scope, PrActionConfigScope::Repository);
}
