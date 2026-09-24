use super::*;
use crate::commands::dashboard::test_support::context;

#[test]
fn clearing_a_notice_returns_the_footer_row_to_the_pr_list() {
    let now = Instant::now();
    let mut keyboard = DashboardKeyboard::new(crate::repository::DashboardKeyBindings::default());
    let mut frame = PullRequestTableFrame::default();
    frame.push_line("repository");
    for number in 1..=8 {
        frame.push_pr_line(
            &format!("  #{number} title"),
            Some(context(number, "owner/repo")),
        );
    }
    let mut nav = DashboardNavigation::default();
    nav.reconcile(Some(&frame));
    nav.handle_command(DashboardCommand::Last, &frame, 5);
    let mut status = DashboardStatus::default();
    status.refreshed(Err("offline".to_owned()), now);
    for height in [5, 3] {
        let screen = dashboard_screen(
            Some(&frame),
            DashboardTerminalSize::new(100, height),
            &mut nav,
            DashboardControls {
                menu: None,
                keyboard: &mut keyboard,
            },
            &status,
            None,
            now,
        );
        assert_eq!(screen.content_size.height, usize::from(height) - 1);
        assert_eq!(screen.marker, Some(usize::from(height) - 2));
        assert!(screen.output.ends_with("#8 title"));
        assert!(!screen.output.contains("Refresh failed"));
        let mut bytes = Vec::new();
        write_dashboard_screen(&mut bytes, &screen).unwrap();
        let rendered = String::from_utf8(bytes).unwrap();
        assert!(rendered.contains(&format!("\x1b[?7l\x1b[{height};1H\x1b[0;48;2;236;233;219m")));
        assert!(rendered.contains("\x1b[0m\x1b[?7h"));
    }
    status.clear_notice();
    nav.handle_command(DashboardCommand::First, &frame, 5);
    let screen = dashboard_screen(
        Some(&frame),
        DashboardTerminalSize::new(100, 5),
        &mut nav,
        DashboardControls {
            menu: None,
            keyboard: &mut keyboard,
        },
        &status,
        None,
        now,
    );
    assert_eq!(screen.content_size.height, 5);
    assert!(screen.footer.is_none());
    assert_eq!(nav.selected(&frame).unwrap().pr_number, 1);
}

#[test]
fn pending_prefix_does_not_change_the_screen_and_help_preserves_selection_on_resize() {
    let now = Instant::now();
    let mut keyboard = DashboardKeyboard::new(crate::repository::DashboardKeyBindings::default());
    let mut frame = PullRequestTableFrame::default();
    for number in 1..=3 {
        frame.push_pr_line(&format!("PR {number}"), Some(context(number, "owner/repo")));
    }
    let mut nav = DashboardNavigation::default();
    nav.reconcile(Some(&frame));
    nav.handle_command(DashboardCommand::Down, &frame, 20);
    let status = DashboardStatus::default();
    let before = dashboard_screen(
        Some(&frame),
        DashboardTerminalSize::new(100, 20),
        &mut nav,
        DashboardControls {
            menu: None,
            keyboard: &mut keyboard,
        },
        &status,
        None,
        now,
    );
    keyboard.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    let screen = dashboard_screen(
        Some(&frame),
        DashboardTerminalSize::new(100, 20),
        &mut nav,
        DashboardControls {
            menu: None,
            keyboard: &mut keyboard,
        },
        &status,
        None,
        now,
    );
    assert!(screen.footer.is_none());
    assert_eq!(screen.content_size.height, 20);
    let mut before_bytes = Vec::new();
    let mut after_bytes = Vec::new();
    write_dashboard_screen(&mut before_bytes, &before).unwrap();
    write_dashboard_screen(&mut after_bytes, &screen).unwrap();
    assert_eq!(before_bytes, after_bytes);
    assert_eq!(nav.selected(&frame).unwrap().pr_number, 2);
    keyboard.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    keyboard.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    for (width, height) in [(100, 20), (30, 8), (1, 1), (0, 0)] {
        let screen = dashboard_screen(
            Some(&frame),
            DashboardTerminalSize::new(width, height),
            &mut nav,
            DashboardControls {
                menu: None,
                keyboard: &mut keyboard,
            },
            &status,
            None,
            now,
        );
        let help = screen.menu.unwrap();
        assert!(screen.footer.is_none());
        assert_eq!(screen.content_size.height, usize::from(height));
        assert!(help.y + help.lines.len() <= screen.content_size.height);
        assert_eq!(nav.selected(&frame).unwrap().pr_number, 2);
    }
    assert_eq!(
        keyboard.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        DashboardInput::None
    );
    assert!(!keyboard.help_open());
    assert_eq!(nav.selected(&frame).unwrap().pr_number, 2);
}

#[test]
fn pending_prefix_does_not_replace_the_running_refresh_status() {
    let now = Instant::now();
    let mut keyboard = DashboardKeyboard::new(crate::repository::DashboardKeyBindings::default());
    let mut status = DashboardStatus::default();
    status.request_refresh();
    status.refresh_started(DashboardRefreshKind::Live, false, now);
    let mut nav = DashboardNavigation::default();
    keyboard.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    let screen = dashboard_screen(
        None,
        DashboardTerminalSize::new(100, 20),
        &mut nav,
        DashboardControls {
            menu: None,
            keyboard: &mut keyboard,
        },
        &status,
        None,
        now,
    );
    assert_eq!(screen.footer, status.line(None, now, 100));
    assert!(screen.footer.unwrap().contains("Refreshing pull requests"));
    assert_eq!(screen.content_size.height, 19);
}

#[test]
fn menus_stay_above_the_status_line_and_tiny_panes_do_not_underflow() {
    let now = Instant::now();
    let mut keyboard = DashboardKeyboard::new(crate::repository::DashboardKeyBindings::default());
    let mut status = DashboardStatus::default();
    status.refreshed(Err("offline".to_owned()), now);
    let mut nav = DashboardNavigation::default();
    let mut menu = PrActionMenu::new(&context(1, "owner/repo"), Ok(Vec::new()));
    for height in [0, 1, 10] {
        let screen = dashboard_screen(
            None,
            DashboardTerminalSize::new(80, height),
            &mut nav,
            DashboardControls {
                menu: Some(&mut menu),
                keyboard: &mut keyboard,
            },
            &status,
            None,
            now,
        );
        let menu = screen.menu.as_ref().unwrap();
        assert!(menu.y + menu.lines.len() <= screen.content_size.height);
        let mut bytes = Vec::new();
        write_dashboard_screen(&mut bytes, &screen).unwrap();
    }
    status.clear_notice();
    let screen = dashboard_screen(
        None,
        DashboardTerminalSize::new(80, 10),
        &mut nav,
        DashboardControls {
            menu: Some(&mut menu),
            keyboard: &mut keyboard,
        },
        &status,
        None,
        now,
    );
    assert!(screen.footer.is_none());
    assert_eq!(screen.content_size.height, 10);
    assert_eq!(
        clipped_dashboard_lines("abcdef\nok\nthird", DashboardTerminalSize::new(4, 2)),
        ["abc…", "ok"]
    );
}
