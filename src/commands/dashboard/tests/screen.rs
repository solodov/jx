use super::*;
use crate::commands::dashboard::test_support::context;

#[test]
fn notice_reserves_the_bottom_row_until_interaction_clears_it() {
    let now = Instant::now();
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
    nav.handle_key(KeyCode::End, &frame, 5);
    let mut status = DashboardStatus::default();
    status.refreshed(Err("offline".to_owned()), now);
    for height in [5, 3] {
        let screen = dashboard_screen(
            Some(&frame),
            DashboardTerminalSize::new(100, height),
            &mut nav,
            None,
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
    nav.handle_key(KeyCode::Home, &frame, 5);
    let screen = dashboard_screen(
        Some(&frame),
        DashboardTerminalSize::new(100, 5),
        &mut nav,
        None,
        &status,
        None,
        now,
    );
    assert_eq!(screen.content_size.height, 5);
    assert!(screen.footer.is_none());
    assert_eq!(nav.selected(&frame).unwrap().pr_number, 1);
}

#[test]
fn menus_stay_above_the_status_line_and_tiny_panes_do_not_underflow() {
    let now = Instant::now();
    let mut status = DashboardStatus::default();
    status.refreshed(Err("offline".to_owned()), now);
    let mut nav = DashboardNavigation::default();
    let mut menu = PrActionMenu::new(&context(1, "owner/repo"), Ok(Vec::new()));
    for height in [0, 1, 10] {
        let screen = dashboard_screen(
            None,
            DashboardTerminalSize::new(80, height),
            &mut nav,
            Some(&mut menu),
            &status,
            None,
            now,
        );
        let menu = screen.menu.as_ref().unwrap();
        assert!(menu.y + menu.lines.len() <= screen.content_size.height);
        let mut bytes = Vec::new();
        write_dashboard_screen(&mut bytes, &screen).unwrap();
    }
    assert_eq!(
        clipped_dashboard_lines("abcdef\nok\nthird", DashboardTerminalSize::new(4, 2)),
        ["abc…", "ok"]
    );
}
