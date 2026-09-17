use super::*;
use crate::commands::dashboard::test_support::context;
use crate::domain::prepare_pr_action;
use crate::repository::{
    PrAction, PrActionConfigScope, PrActionSource, PrActionWorkingDirectory, ResolvedPrAction,
};

fn entry(local: bool, command: &[&str]) -> AvailablePrAction {
    let definition = ResolvedPrAction {
        action: PrAction {
            id: "test".to_owned(),
            title: "Test action".to_owned(),
            command: command.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd: PrActionWorkingDirectory::Caller,
        },
        source: PrActionSource {
            path: PathBuf::from("/source/config.toml"),
            scope: if local {
                PrActionConfigScope::Repository
            } else {
                PrActionConfigScope::Global
            },
        },
    };
    let prepared = prepare_pr_action(
        &definition,
        &context(12, "owner/repo"),
        Path::new("/caller"),
    );
    AvailablePrAction {
        definition,
        prepared,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn size() -> DashboardTerminalSize {
    DashboardTerminalSize::new(100, 30)
}

#[test]
fn local_overrides_require_explicit_confirmation_of_frozen_invocation() {
    let mut menu = PrActionMenu::new(
        &context(12, "owner/repo"),
        Ok(vec![entry(true, &["open", "{pr_url}"])]),
    );
    assert!(matches!(
        menu.handle_key(key(KeyCode::Enter), false, size()),
        MenuIntent::None
    ));
    assert!(menu.confirming);
    assert!(matches!(
        menu.handle_key(key(KeyCode::Enter), false, size()),
        MenuIntent::None
    ));
    let mut repeat = key(KeyCode::Char('y'));
    repeat.kind = KeyEventKind::Repeat;
    assert!(matches!(
        menu.handle_key(repeat, false, size()),
        MenuIntent::None
    ));
    let MenuIntent::Run(action) = menu.handle_key(key(KeyCode::Char('y')), false, size()) else {
        panic!("confirmed invocation")
    };
    assert_eq!(action.target.number, 12);
    assert_eq!(
        action.command,
        ["open", "https://github.com/owner/repo/pull/12"]
    );
    assert_eq!(action.cwd, Path::new("/caller"));
}

#[test]
fn escape_cancels_confirmation_without_running_and_then_closes_menu() {
    let mut menu = PrActionMenu::new(&context(12, "owner/repo"), Ok(vec![entry(true, &["open"])]));
    menu.handle_key(key(KeyCode::Enter), false, size());
    assert!(matches!(
        menu.handle_key(key(KeyCode::Esc), false, size()),
        MenuIntent::None
    ));
    assert!(!menu.confirming);
    assert!(matches!(
        menu.handle_key(key(KeyCode::Esc), false, size()),
        MenuIntent::Close
    ));
}

#[test]
fn missing_context_refreshes_and_tiny_terminals_prevent_execution() {
    let mut menu = PrActionMenu::new(
        &context(12, "owner/repo"),
        Ok(vec![
            entry(false, &["jj", "diff", "-r", "{local_change_id}"]),
            entry(false, &["open"]),
        ]),
    );
    assert!(matches!(
        menu.handle_key(key(KeyCode::Enter), false, size()),
        MenuIntent::None
    ));
    assert!(menu.details().join("\n").contains("local_change_id"));
    menu.handle_key(key(KeyCode::Down), false, size());
    assert!(matches!(
        menu.handle_key(key(KeyCode::Enter), true, size()),
        MenuIntent::None
    ));
    assert!(matches!(
        menu.handle_key(
            key(KeyCode::Enter),
            false,
            DashboardTerminalSize::new(10, 5)
        ),
        MenuIntent::None
    ));
    assert!(matches!(
        menu.handle_key(key(KeyCode::Enter), false, size()),
        MenuIntent::Run(_)
    ));
}

#[test]
fn menu_is_bounded_sanitized_and_preview_is_pageable() {
    let mut ctx = context(12, "owner/repo");
    ctx.title = "title\x1b[2J\nspoof\u{202e}".to_owned();
    let long = "漢字x".repeat(400);
    let mut menu = PrActionMenu::new(
        &ctx,
        Ok(vec![entry(true, &["program", &long, "", "literal\nvalue"])]),
    );
    for (width, height) in [(100, 30), (30, 10), (10, 5), (0, 0)] {
        let screen = menu.screen(DashboardTerminalSize::new(width, height), false);
        assert!(screen.lines.len() <= usize::from(height));
        for line in screen.lines {
            let plain = line
                .split_once('m')
                .unwrap()
                .1
                .strip_suffix("\x1b[0m")
                .unwrap();
            assert!(!plain.contains('\x1b'));
            assert!(plain.width() <= usize::from(width));
        }
    }
    assert!(plain_text(&ctx.title).contains("\\u{1b}"));
    menu.handle_key(key(KeyCode::PageDown), false, size());
    let screen = menu.screen(size(), false);
    assert!(menu.detail_offset > 0);
    assert!(screen.lines.iter().any(|line| line.contains("漢字")));
    assert!(menu.details().join("\n").contains("argv[2]: \"\""));
    assert!(menu.details().join("\n").contains("/source/config.toml"));
}

#[test]
fn empty_and_invalid_configuration_are_explanatory_not_executable() {
    for result in [Ok(Vec::new()), Err("invalid config".to_owned())] {
        let mut menu = PrActionMenu::new(&context(12, "owner/repo"), result);
        assert!(menu
            .screen(size(), false)
            .lines
            .iter()
            .any(|line| line.contains("Configure") || line.contains("Cannot load")));
        assert!(matches!(
            menu.handle_key(key(KeyCode::Enter), false, size()),
            MenuIntent::Close
        ));
    }
}
