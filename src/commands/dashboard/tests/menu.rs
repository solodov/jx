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
    let preview = menu.screen(size(), None).lines.join("\n");
    assert!(preview.contains("/source/config.toml"));
    assert!(preview.contains("cwd: /caller"));
    assert!(preview.contains("argv[0]: \"open\""));
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
    menu.handle_key(key(KeyCode::Tab), false, size());
    for (width, height) in [(100, 30), (30, 10), (10, 5), (0, 0)] {
        let screen = menu.screen(DashboardTerminalSize::new(width, height), None);
        assert!(screen.lines.len() <= usize::from(height));
        for line in screen.lines {
            let plain = unstyled(&line);
            assert!(!plain.contains('\x1b'));
            assert!(plain.width() <= usize::from(width));
        }
    }
    assert!(plain_text(&ctx.title).contains("\\u{1b}"));
    menu.handle_key(key(KeyCode::PageDown), false, size());
    let screen = menu.screen(size(), None);
    assert!(menu.detail_offset > 0);
    assert!(screen.lines.iter().any(|line| line.contains("漢字")));
    assert!(menu.details().join("\n").contains("argv[2]: \"\""));
    assert!(menu.details().join("\n").contains("/source/config.toml"));
}

fn unstyled(text: &str) -> String {
    text.replace(BODY, "")
        .replace(SELECTED, "")
        .replace("\x1b[0m", "")
}

#[test]
fn compact_menu_matches_acme_colors_and_keeps_details_off_the_action_list() {
    let entries = ["put", "send", "look", "definition"]
        .map(|title| {
            let mut action = entry(false, &["open"]);
            action.definition.action.title = title.to_owned();
            action
        })
        .into_iter()
        .collect();
    let mut menu = PrActionMenu::new(&context(12, "owner/repo"), Ok(entries));
    let screen = menu.screen(size(), Some(5));
    assert_eq!((screen.x, screen.y), (2, 6));
    assert_eq!(unstyled(&screen.lines.join("\n")), "┌────────────┐\n│ put        │\n│ send       │\n│ look       │\n│ definition │\n└────────────┘");
    assert_eq!(
        screen.lines[1],
        format!("{BODY}│{SELECTED} put        {BODY}│\x1b[0m")
    );
    assert_eq!(BODY, "\x1b[0;38;2;31;91;42;48;2;228;246;211m");
    assert_eq!(SELECTED, "\x1b[0;1;38;2;228;246;211;48;2;31;91;42m");
    menu.handle_key(key(KeyCode::Char('?')), false, size());
    assert!(menu
        .screen(size(), None)
        .lines
        .join("\n")
        .contains("argv[0]"));
    menu.handle_key(key(KeyCode::Esc), false, size());
    assert!(!menu.showing_details);
    for _ in 0..4 {
        menu.handle_key(key(KeyCode::Down), false, size());
    }
    assert_eq!(menu.selected, 3);
    assert!(matches!(
        menu.handle_key(key(KeyCode::Enter), false, size()),
        MenuIntent::Run(_)
    ));
    assert!(matches!(
        menu.handle_key(key(KeyCode::Esc), false, size()),
        MenuIntent::Close
    ));
}

#[test]
fn long_action_lists_scroll_and_stay_inside_the_terminal() {
    let entries = (0..30)
        .map(|index| {
            let mut action = entry(false, &["open"]);
            action.definition.action.title = format!("action {index}");
            action
        })
        .collect();
    let mut menu = PrActionMenu::new(&context(12, "owner/repo"), Ok(entries));
    let size = DashboardTerminalSize::new(24, 10);
    for _ in 0..29 {
        menu.handle_key(key(KeyCode::Down), false, size);
    }
    for (anchor, top, height) in [(9, 0, 9), (4, 5, 5), (0, 1, 9)] {
        let screen = menu.screen(size, Some(anchor));
        assert_eq!((screen.y, screen.lines.len()), (top, height));
        assert!(screen.y > anchor || screen.y + screen.lines.len() <= anchor);
        assert!(screen
            .lines
            .iter()
            .any(|line| line.contains("action 29") && line.contains(SELECTED)));
        assert!(screen
            .lines
            .iter()
            .all(|line| unstyled(line).width() + screen.x <= size.width));
        assert!(!screen.lines.iter().any(|line| line.contains("refreshing")));
    }
}

#[test]
fn refresh_completion_does_not_change_menu_layout_or_entries() {
    let mut menu = PrActionMenu::new(
        &context(12, "owner/repo"),
        Ok(vec![entry(false, &["open", "{pr_url}"])]),
    );
    let before = menu.screen(size(), Some(5));
    assert!(matches!(
        menu.handle_key(key(KeyCode::Enter), true, size()),
        MenuIntent::None
    ));
    let after = menu.screen(size(), Some(5));
    assert_eq!(
        (before.x, before.y, before.lines),
        (after.x, after.y, after.lines)
    );
    let MenuIntent::Run(action) = menu.handle_key(key(KeyCode::Enter), false, size()) else {
        panic!("action runs once refresh finishes");
    };
    assert_eq!(action.target.number, 12);
    assert_eq!(
        action.command,
        ["open", "https://github.com/owner/repo/pull/12"]
    );
}

#[test]
fn empty_menu_only_shows_no_actions_configured() {
    let mut menu = PrActionMenu::new(&context(12, "owner/repo"), Ok(Vec::new()));
    let screen = menu.screen(size(), Some(5));
    assert_eq!((screen.x, screen.y), (2, 6));
    assert_eq!(
        unstyled(&screen.lines.join("\n")),
        "┌───────────────────────┐\n│ no actions configured │\n└───────────────────────┘"
    );
    let small = DashboardTerminalSize::new(21, 1);
    assert!(menu.screen(small, Some(0)).lines.is_empty());
    assert_eq!(
        unstyled(&menu.screen(small, None).lines.join("\n")),
        "no actions configured"
    );
    assert!(matches!(
        menu.handle_key(key(KeyCode::Enter), false, small),
        MenuIntent::Close
    ));
}

#[test]
fn compact_and_empty_menus_prefer_below_then_above_the_pr_row() {
    let mut empty = PrActionMenu::new(&context(12, "owner/repo"), Ok(Vec::new()));
    let mut actions = PrActionMenu::new(
        &context(12, "owner/repo"),
        Ok(vec![entry(false, &["open"])]),
    );
    for (anchor, top) in [(0, 1), (5, 6), (25, 26), (26, 27), (28, 25), (29, 26)] {
        for menu in [&mut empty, &mut actions] {
            let screen = menu.screen(size(), Some(anchor));
            assert_eq!((screen.y, screen.lines.len()), (top, 3));
            assert!(screen.y > anchor || screen.y + screen.lines.len() <= anchor);
        }
    }
}

#[test]
fn action_failures_show_a_small_log_notice_not_command_output() {
    let failure = pr_actions::PrActionFailure {
        message: "open failed".to_owned(),
        log_path: Some(PathBuf::from("/logs/jx-actions.log")),
    };
    let screen = action_failure_screen(&failure, size(), Some(5));
    assert_eq!(screen.lines.len(), 5);
    assert_eq!(screen.y, 6);
    let text = unstyled(&screen.lines.join("\n"));
    assert!(text.contains("open failed"));
    assert!(text.contains("See log for details:"));
    assert!(text.contains("/logs/jx-actions.log"));
    let failure = pr_actions::PrActionFailure {
        message: "Cannot open log: permission denied".to_owned(),
        log_path: None,
    };
    let text = unstyled(
        &action_failure_screen(&failure, size(), None)
            .lines
            .join("\n"),
    );
    assert!(text.contains("Cannot open log: permission denied"));
    assert!(!text.contains("See log"));
}

#[test]
fn invalid_configuration_is_not_misreported_as_no_actions() {
    let mut menu = PrActionMenu::new(&context(12, "owner/repo"), Err("invalid config".to_owned()));
    let screen = unstyled(&menu.screen(size(), None).lines.join("\n"));
    assert!(screen.contains("Cannot load actions: invalid config"));
    assert!(!screen.contains("no actions configured"));
    assert!(matches!(
        menu.handle_key(key(KeyCode::Enter), false, size()),
        MenuIntent::Close
    ));
}
