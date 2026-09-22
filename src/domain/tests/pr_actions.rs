use super::*;
use crate::repository::PrAction;

#[test]
fn preparation_preserves_argv_boundaries_and_substitutes_only_once() {
    let mut context = context();
    context.title = "A title with spaces; $(not-executed) '{pr_number}'".to_owned();
    let definition = action(&[
        "not-a-program",
        "{title}",
        "--url={pr_url}",
        "{{pr_number}}",
        "",
        "{repo}",
        "{pr_number}",
    ]);
    let prepared = prepare_pr_action(&definition, &context, &std::env::temp_dir()).unwrap();
    assert_eq!(
        prepared.command,
        [
            "not-a-program",
            context.title.as_str(),
            "--url=https://github.com/owner/repo/pull/12",
            "{pr_number}",
            "",
            "owner/repo",
            "12",
        ]
    );
    assert_eq!(prepared.target, context.key());
    assert_eq!(prepared.cwd, context.repository_root.clone().unwrap());
}

#[test]
fn missing_context_disables_actions_instead_of_using_empty_values_or_the_current_commit() {
    let context = context();
    for placeholder in ["{local_commit_id}", "{local_change_id}"] {
        let error = prepare_pr_action(
            &action(&["jj", "show", placeholder]),
            &context,
            &std::env::temp_dir(),
        )
        .unwrap_err();
        assert!(error.reason.contains(placeholder));
    }
    let mut external = context.clone();
    external.repository_root = None;
    assert!(
        prepare_pr_action(&action(&["tool"]), &external, &std::env::temp_dir())
            .unwrap_err()
            .reason
            .contains("checkout")
    );
    let mut remote = action(&["open", "{pr_url}"]);
    remote.action.cwd = PrActionWorkingDirectory::Caller;
    assert_eq!(
        prepare_pr_action(&remote, &external, &std::env::temp_dir())
            .unwrap()
            .cwd,
        std::env::temp_dir()
    );
    remote.action.command = vec!["tool".to_owned(), "{repo_root}".to_owned()];
    assert!(prepare_pr_action(&remote, &external, &std::env::temp_dir()).is_err());
    remote.action.command = vec!["tool".to_owned(), "{head_oid}".to_owned()];
    external.head_oid = None;
    assert!(prepare_pr_action(&remote, &external, &std::env::temp_dir()).is_err());
}

#[test]
fn prepared_actions_distinguish_local_revisions_and_retain_override_provenance() {
    let mut context = context();
    context.local_commit_id = Some("local-tip".to_owned());
    context.local_change_id = Some("local-change".to_owned());
    let mut definition = action(&[
        "tool",
        "{head_oid}",
        "{local_commit_id}",
        "{local_change_id}",
        "{branch}",
        "{base_branch}",
        "{repo_root}",
    ]);
    let prepared = prepare_pr_action(&definition, &context, &std::env::temp_dir()).unwrap();
    assert_eq!(
        &prepared.command[1..6],
        [
            "remote-head",
            "local-tip",
            "local-change",
            "feature",
            "main"
        ]
    );
    assert_eq!(
        prepared.command[6],
        context.repository_root.as_ref().unwrap().to_str().unwrap()
    );
    assert!(!prepared.requires_confirmation());
    definition.action.on_success = PrActionOnSuccess::RefreshLocal;
    definition.source.scope = PrActionConfigScope::Repository;
    definition.source.path = std::env::temp_dir().join("checkout/.jx/config.toml");
    let replaced = prepare_pr_action(&definition, &context, &std::env::temp_dir()).unwrap();
    assert_eq!(replaced.source, definition.source);
    assert_eq!(replaced.on_success, PrActionOnSuccess::RefreshLocal);
    assert!(replaced.requires_confirmation());
}

#[test]
fn target_identity_does_not_depend_on_display_or_local_location() {
    let first = context();
    let mut second = first.clone();
    second.title = "A new title".to_owned();
    second.repository_root = None;
    second.branch = "renamed".to_owned();
    assert_eq!(first.key(), second.key());
    second.repository.name = "other-repository".to_owned();
    assert_ne!(first.key(), second.key());
}

#[test]
fn invalid_templates_paths_and_expanded_commands_never_prepare() {
    let mut context = context();
    for command in [
        vec![],
        vec![""],
        vec!["tool", "{nope}"],
        vec!["tool", "{"],
        vec!["tool", "}"],
    ] {
        assert!(prepare_pr_action(&action(&command), &context, &std::env::temp_dir()).is_err());
    }
    context.title = "invalid\0argument".to_owned();
    assert!(prepare_pr_action(
        &action(&["tool", "{title}"]),
        &context,
        &std::env::temp_dir()
    )
    .is_err());
    context.repository_root = Some(PathBuf::from("relative"));
    assert!(prepare_pr_action(&action(&["tool"]), &context, &std::env::temp_dir()).is_err());
}

fn context() -> PrActionContext {
    PrActionContext {
        repository: GitHubRepository {
            owner: "owner".to_owned(),
            name: "repo".to_owned(),
        },
        repository_root: Some(std::env::temp_dir().join("checkout")),
        pr_number: 12,
        pr_url: "https://github.com/owner/repo/pull/12".to_owned(),
        title: "Title".to_owned(),
        branch: "feature".to_owned(),
        base_branch: "main".to_owned(),
        head_oid: Some("remote-head".to_owned()),
        local_commit_id: None,
        local_change_id: None,
    }
}

fn action(command: &[&str]) -> ResolvedPrAction {
    ResolvedPrAction {
        action: PrAction {
            id: "inspect".to_owned(),
            title: "Inspect".to_owned(),
            order: 0,
            command: command.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd: PrActionWorkingDirectory::Repository,
            on_success: PrActionOnSuccess::default(),
        },
        source: PrActionSource {
            path: PathBuf::from("/config/jx/actions.toml"),
            scope: PrActionConfigScope::Global,
        },
    }
}
