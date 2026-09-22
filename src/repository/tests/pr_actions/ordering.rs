use super::*;

#[test]
fn order_and_title_group_actions_independently_of_config_composition() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repository = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    for action_set in ["review_actions", "stack_status_actions"] {
        for reverse in [false, true] {
            let mut base = vec![
                definition(action_set, "open", "open", Some(-100)),
                definition(action_set, "copy", "copy url", None),
                definition(action_set, "middle", "amend", Some(0)),
            ];
            let dismiss = definition(action_set, "z-dismiss", "dismiss", Some(100));
            let fix_tests = definition(action_set, "a-fix", "dismiss/fix tests", Some(100));
            let rule = if reverse {
                base.push(fix_tests);
                base.reverse();
                dismiss
            } else {
                base.push(dismiss);
                fix_tests
            };
            workspace.write_file(".config/jx/10-base.toml", &base.concat());
            workspace.write_file(
                ".config/jx/20-rules.toml",
                &format!(
                    "[[repo.rules]]\nrepo='owner/*'\n{}",
                    rule.replace("[[repo.", "[[repo.rules.")
                ),
            );
            let config = WorkflowConfig::discover(&environment).unwrap();
            let actions = match action_set {
                "review_actions" => config.review_actions.for_repository(&repository),
                _ => config.stack_status_actions.for_repository(&repository),
            };
            assert_eq!(
                ids(&actions),
                ["open", "middle", "copy", "z-dismiss", "a-fix"]
            );
            assert_eq!(actions[2].action.order, 0);
            assert_eq!(actions[3].action.order, actions[4].action.order);
        }
    }
}

#[test]
fn final_override_order_and_title_control_placement_and_omission_resets_order() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repository = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    workspace.write_file(
        ".config/jx/base.toml",
        &[
            definition("review_actions", "move", "zzz", Some(100)),
            definition("review_actions", "anchor", "middle", None),
            "[[repo.rules]]\nrepo='owner/*'\n".to_owned(),
            definition("rules.review_actions", "move", "zzz", Some(-100)),
        ]
        .concat(),
    );
    let config = WorkflowConfig::discover(&environment).unwrap();
    let actions = config.review_actions.for_repository(&repository);
    assert_eq!(ids(&actions), ["move", "anchor"]);
    assert_eq!(actions[0].action.order, -100);

    workspace.write_file(
        ".jx/config.toml",
        &definition("review_actions", "move", "zzz", None),
    );
    let config = WorkflowConfig::discover(&environment).unwrap();
    let actions = config.review_actions.for_repository(&repository);
    assert_eq!(ids(&actions), ["anchor", "move"]);
    assert_eq!(actions[1].action.order, 0);
    assert_eq!(actions[1].source.scope, PrActionConfigScope::Repository);

    workspace.write_file(
        ".jx/config.toml",
        &[
            definition("review_actions", "move", "zzz", Some(100)),
            "[[repo.rules]]\nrepo='owner/*'\n".to_owned(),
            definition("rules.review_actions", "move", "aaa", None),
        ]
        .concat(),
    );
    let config = WorkflowConfig::discover(&environment).unwrap();
    let actions = config.review_actions.for_repository(&repository);
    assert_eq!(ids(&actions), ["move", "anchor"]);
    assert_eq!(actions[0].action.title, "aaa");
    assert_eq!(actions[0].action.order, 0);
}

#[test]
fn signed_order_bounds_are_supported_and_each_menu_sorts_independently() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repository = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    workspace.write_file(
        ".jx/config.toml",
        &[
            definition("review_actions", "last", "aaa", Some(i64::MAX)),
            definition("review_actions", "first", "zzz", Some(i64::MIN)),
            definition("review_actions", "middle", "middle", None),
            definition("stack_status_actions", "first", "zzz", Some(i64::MAX)),
            definition("stack_status_actions", "last", "aaa", Some(i64::MIN)),
        ]
        .concat(),
    );
    let config = WorkflowConfig::discover(&environment).unwrap();
    assert_eq!(
        ids(&config.review_actions.for_repository(&repository)),
        ["first", "middle", "last"]
    );
    assert_eq!(
        ids(&config.stack_status_actions.for_repository(&repository)),
        ["last", "first"]
    );
}

fn definition(action_set: &str, id: &str, title: &str, order: Option<i64>) -> String {
    let order = order
        .map(|value| format!("order={value}\n"))
        .unwrap_or_default();
    format!("[[repo.{action_set}]]\nid='{id}'\ntitle='{title}'\ncommand=['action']\n{order}")
}

fn ids(actions: &[ResolvedPrAction]) -> Vec<&str> {
    actions
        .iter()
        .map(|entry| entry.action.id.as_str())
        .collect()
}
