use super::*;
use crate::commands::dashboard::test_support::context;

#[test]
fn menu_freezes_pr_rows_and_selection_until_queued_refresh_is_applied() {
    let size = DashboardTerminalSize::new(100, 30);
    let resized = DashboardTerminalSize::new(42, 20);
    let mut view = DashboardView {
        pending: Some(Ok(snapshot(&[1, 2]))),
        ..DashboardView::default()
    };
    view.update(false, false, size);
    let before = view.frame.clone();
    let mut navigation = DashboardNavigation::default();
    navigation.reconcile(view.frame.as_ref());
    navigation.handle_key(KeyCode::Down, view.frame.as_ref().unwrap(), size.height);
    assert_eq!(navigation.selected_line, Some(2));

    view.pending = Some(Ok(snapshot(&[2, 3, 1])));
    for _ in 0..3 {
        view.update(true, false, resized);
        navigation.reconcile(view.frame.as_ref());
        assert_eq!(view.frame, before);
        assert!(view.pending.is_some());
        assert_eq!(navigation.selected_line, Some(2));
        assert_eq!(
            navigation
                .selected(view.frame.as_ref().unwrap())
                .unwrap()
                .pr_number,
            2
        );
    }

    view.update(false, false, resized);
    navigation.reconcile(view.frame.as_ref());
    assert!(view.pending.is_none());
    let frame = view.frame.as_ref().unwrap();
    assert_eq!(frame.text, "width=42\nPR 2\nPR 3\nPR 1\n");
    assert_eq!(navigation.selected_line, Some(1));
    assert_eq!(navigation.selected(frame).unwrap().pr_number, 2);
}

#[test]
fn refresh_errors_and_timeouts_do_not_shift_the_table_under_an_open_menu() {
    let size = DashboardTerminalSize::new(100, 30);
    for timed_out in [false, true] {
        let mut view = DashboardView {
            pending: Some(Ok(snapshot(&[12]))),
            ..DashboardView::default()
        };
        view.update(false, false, size);
        view.error = Some("previous failure".to_owned());
        let before = view.frame.clone();
        if !timed_out {
            view.pending = Some(Err("network down".to_owned()));
        }

        view.update(true, timed_out, size);
        assert_eq!(view.frame, before);
        assert_eq!(view.error.as_deref(), Some("previous failure"));
        assert_eq!(view.pending.is_some(), !timed_out);

        view.update(false, timed_out, size);
        assert_eq!(view.frame, before);
        assert!(view.pending.is_none());
        assert_eq!(
            view.error,
            Some(if timed_out {
                dashboard_refresh_timeout_error()
            } else {
                "network down".to_owned()
            }),
        );
    }
}

#[test]
fn snapshot_reflow_waits_for_menu_close_and_uses_current_terminal_width() {
    let size = DashboardTerminalSize::new(100, 30);
    let resized = DashboardTerminalSize::new(42, 20);
    let mut view = DashboardView {
        pending: Some(Ok(snapshot(&[12]))),
        ..DashboardView::default()
    };
    view.update(false, false, size);
    let before = view.frame.clone();

    view.update(true, false, resized);
    assert_eq!(view.frame, before);
    view.update(false, false, resized);
    assert_eq!(view.frame.as_ref().unwrap().text, "width=42\nPR 12\n");
    assert!(view.error.is_none());
}

#[test]
fn failed_snapshot_render_preserves_previous_rows_until_and_after_menu_close() {
    let size = DashboardTerminalSize::new(100, 30);
    let mut view = DashboardView {
        pending: Some(Ok(snapshot(&[12]))),
        ..DashboardView::default()
    };
    view.update(false, false, size);
    let before = view.frame.clone();
    view.pending = Some(Ok(DashboardFrameSnapshot::new(|_| {
        Err("render failed".to_owned())
    })));

    view.update(true, false, size);
    assert_eq!(view.frame, before);
    assert!(view.error.is_none());
    view.update(false, false, size);
    assert_eq!(view.frame, before);
    assert_eq!(view.error.as_deref(), Some("render failed"));
}

#[test]
fn removing_a_pr_preserves_selection_or_selects_the_next_row_across_resizes() {
    let size = DashboardTerminalSize::new(100, 30);
    for (selected, remaining, expected) in [
        (2, vec![2, 3], Some(2)),
        (2, vec![1, 3], Some(3)),
        (3, vec![1, 2], Some(2)),
        (1, vec![], None),
    ] {
        let mut view = DashboardView {
            pending: Some(Ok(snapshot(&[1, 2, 3]))),
            ..DashboardView::default()
        };
        view.update(false, false, size);
        let mut navigation = DashboardNavigation::default();
        navigation.reconcile(view.frame.as_ref());
        for _ in 1..selected {
            navigation.handle_key(KeyCode::Down, view.frame.as_ref().unwrap(), size.height);
        }
        view.pending = Some(Ok(snapshot(&remaining)));
        for width in [100, 42] {
            view.update(false, false, DashboardTerminalSize::new(width, 30));
            navigation.reconcile(view.frame.as_ref());
            let frame = view.frame.as_ref().unwrap();
            assert_eq!(
                navigation.selected(frame).map(|row| row.pr_number),
                expected
            );
            assert_eq!(frame.rows.len(), remaining.len());
        }
    }
}

fn snapshot(numbers: &[u64]) -> DashboardFrameSnapshot {
    let numbers = numbers.to_vec();
    DashboardFrameSnapshot::new(move |options| {
        let mut frame = PullRequestTableFrame::default();
        frame.push_line(&format!(
            "width={}",
            options.terminal_width.unwrap_or_default()
        ));
        for number in &numbers {
            frame.push_pr_line(
                &format!("PR {number}"),
                Some(context(*number, "owner/repo")),
            );
        }
        Ok(frame)
    })
}
