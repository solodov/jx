use super::*;
use crate::commands::dashboard::test_support::context;

#[test]
fn menu_freezes_rows_until_the_queued_snapshot_can_be_applied_and_reflowed() {
    let size = DashboardTerminalSize::new(100, 30);
    let resized = DashboardTerminalSize::new(42, 20);
    let mut view = DashboardView {
        pending: Some(Ok(snapshot(&[1, 2]))),
        ..DashboardView::default()
    };
    assert!(matches!(
        view.update(false, size),
        Some(DashboardViewUpdate::Loaded(Ok(())))
    ));
    let before = view.frame.clone();
    let mut navigation = DashboardNavigation::default();
    navigation.reconcile(view.frame.as_ref());
    navigation.handle_command(
        DashboardCommand::Down,
        view.frame.as_ref().unwrap(),
        size.height,
    );

    view.pending = Some(Ok(snapshot(&[2, 3, 1])));
    assert!(view.update(true, resized).is_none());
    assert_eq!(view.frame, before);
    assert!(view.pending.is_some());
    assert!(matches!(
        view.update(false, resized),
        Some(DashboardViewUpdate::Loaded(Ok(())))
    ));
    navigation.reconcile(view.frame.as_ref());
    let frame = view.frame.as_ref().unwrap();
    assert_eq!(frame.text, "width=42\nPR 2\nPR 3\nPR 1\n");
    assert_eq!(navigation.selected(frame).unwrap().pr_number, 2);
    assert!(view.update(false, resized).is_none());
    assert!(matches!(
        view.update(false, size),
        Some(DashboardViewUpdate::Reflowed(Ok(())))
    ));
}

#[test]
fn failed_loads_or_rendering_preserve_rows_and_report_the_error_only_once() {
    let size = DashboardTerminalSize::new(100, 30);
    for result in [
        Err("network down".to_owned()),
        Ok(DashboardFrameSnapshot::new(|_| {
            Err("render failed".to_owned())
        })),
    ] {
        let mut view = DashboardView {
            pending: Some(Ok(snapshot(&[12]))),
            ..DashboardView::default()
        };
        view.update(false, size);
        let before = view.frame.clone();
        view.pending = Some(result);
        assert!(view.update(true, size).is_none());
        assert!(matches!(
            view.update(false, size),
            Some(DashboardViewUpdate::Loaded(Err(_)))
        ));
        assert_eq!(view.frame, before);
        assert!(view.update(false, size).is_none());
    }
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
        view.update(false, size);
        let mut navigation = DashboardNavigation::default();
        navigation.reconcile(view.frame.as_ref());
        for _ in 1..selected {
            navigation.handle_command(
                DashboardCommand::Down,
                view.frame.as_ref().unwrap(),
                size.height,
            );
        }
        view.pending = Some(Ok(snapshot(&remaining)));
        for width in [100, 42] {
            view.update(false, DashboardTerminalSize::new(width, 30));
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

#[test]
fn grouped_dismissal_keeps_focus_in_the_repository_after_menu_close_and_resize() {
    let grouped_snapshot = |numbers: &[u64]| {
        let repository = snapshot(numbers);
        DashboardFrameSnapshot::new(move |options| {
            let mut frame = repository.render(options)?;
            frame.push_line("another repository");
            for number in [4, 5] {
                frame.push_pr_line(
                    &format!("PR {number}"),
                    Some(context(number, "owner/other")),
                );
            }
            Ok(frame)
        })
    };
    let size = DashboardTerminalSize::new(100, 30);
    let mut view = DashboardView {
        pending: Some(Ok(grouped_snapshot(&[1, 2, 3]))),
        ..DashboardView::default()
    };
    view.update(false, size);
    let mut navigation = DashboardNavigation::default();
    navigation.reconcile(view.frame.as_ref());
    for _ in 0..2 {
        navigation.handle_command(
            DashboardCommand::Down,
            view.frame.as_ref().unwrap(),
            size.height,
        );
    }
    view.pending = Some(Ok(grouped_snapshot(&[1, 2])));
    assert!(view.update(true, size).is_none());
    navigation.reconcile(view.frame.as_ref());
    assert_eq!(
        navigation
            .selected(view.frame.as_ref().unwrap())
            .unwrap()
            .pr_number,
        3
    );
    for width in [100, 42] {
        view.update(false, DashboardTerminalSize::new(width, 30));
        navigation.reconcile(view.frame.as_ref());
        assert_eq!(
            navigation
                .selected(view.frame.as_ref().unwrap())
                .unwrap()
                .pr_number,
            2
        );
    }
    navigation.handle_command(
        DashboardCommand::Down,
        view.frame.as_ref().unwrap(),
        size.height,
    );
    assert_eq!(
        navigation
            .selected(view.frame.as_ref().unwrap())
            .unwrap()
            .pr_number,
        4
    );
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
