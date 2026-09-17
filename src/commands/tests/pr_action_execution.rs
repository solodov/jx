use super::*;
use crate::commands::dashboard::test_support::context;
use crate::repository::{PrActionConfigScope, PrActionSource};
use std::fs;

fn invocation(command: &[&str], cwd: &Path) -> PreparedPrAction {
    PreparedPrAction {
        id: "test".to_owned(),
        title: "Test".to_owned(),
        target: context(12, "owner/repo").key(),
        command: command.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: cwd.to_path_buf(),
        source: PrActionSource {
            path: cwd.join("config.toml"),
            scope: PrActionConfigScope::Global,
        },
    }
}

#[cfg(unix)]
#[test]
fn execution_preserves_argv_cwd_and_exit_status_without_an_implicit_shell() {
    let temp = tempfile::tempdir().unwrap();
    let action = invocation(
        &[
            "sh",
            "-c",
            "printf '%s\\n' \"$1\" \"$2\" > result; exit 7",
            "action",
            "$(touch injected) ; {title}",
            "two words",
        ],
        temp.path(),
    );
    let status = execute_pr_action(&action).unwrap();
    assert_eq!(status.code(), Some(7));
    assert_eq!(
        fs::read_to_string(temp.path().join("result")).unwrap(),
        "$(touch injected) ; {title}\ntwo words\n"
    );
    assert!(!temp.path().join("injected").exists());
    assert!(
        execute_pr_action(&invocation(&["/definitely/missing/jx-action"], temp.path())).is_err()
    );
}

#[cfg(unix)]
#[test]
fn foreground_sigint_does_not_exit_parent_or_leak_into_resumed_dashboard() {
    use std::os::unix::process::ExitStatusExt;
    let interrupts = DashboardInterrupts::enter().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let action = invocation(
        &["sh", "-c", "kill -INT \"$PPID\"; kill -INT $$"],
        temp.path(),
    );
    let status = execute_pr_action(&action).unwrap();
    assert_eq!(status.signal(), Some(signal_hook::consts::signal::SIGINT));
    assert!(interrupts.take_pending());
    // The signal worker may wake later, but synchronous receipt was already acknowledged.
    std::thread::sleep(Duration::from_millis(50));
    assert!(!interrupts.take_pending());
}

#[test]
fn action_loading_uses_selected_checkout_not_caller_and_external_prs_use_global_config() {
    let temp = tempfile::tempdir().unwrap();
    let caller = temp.path().join("caller");
    let selected = temp.path().join("selected");
    let global = temp.path().join(".config/jx");
    for root in [&caller, &selected] {
        fs::create_dir_all(root.join(".jj")).unwrap();
        fs::create_dir_all(root.join(".jx")).unwrap();
    }
    fs::create_dir_all(&global).unwrap();
    fs::write(
        global.join("actions.toml"),
        "[[repo.actions]]\nid='open'\ntitle='Global'\ncommand=['open','{pr_url}']\ncwd='caller'\n",
    )
    .unwrap();
    fs::write(
        caller.join(".jx/config.toml"),
        "[[repo.actions]]\nid='open'\ntitle='Wrong caller'\ncommand=['wrong']\n",
    )
    .unwrap();
    fs::write(
        selected.join(".jx/config.toml"),
        "[[repo.actions]]\nid='open'\ntitle='Selected'\ncommand=['selected']\n",
    )
    .unwrap();
    let environment = RuntimeEnvironment::new(
        &caller,
        [("HOME".to_owned(), temp.path().display().to_string())],
    );
    let mut ctx = context(12, "owner/repo");
    let external = load_pr_actions(ctx.clone(), &environment).unwrap();
    let action = external[0].prepared.as_ref().unwrap();
    assert_eq!(action.title, "Global");
    assert_eq!(action.cwd, caller);
    assert!(!action.requires_confirmation());
    ctx.repository_root = Some(selected.clone());
    let local = load_pr_actions(ctx, &environment).unwrap();
    let action = local[0].prepared.as_ref().unwrap();
    assert_eq!(action.title, "Selected");
    assert_eq!(action.cwd, selected);
    assert!(action.requires_confirmation());
}
