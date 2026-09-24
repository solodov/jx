use super::*;
use crate::commands::dashboard::test_support::context;

fn frame(numbers: &[u64]) -> PullRequestTableFrame {
    let mut frame = PullRequestTableFrame::default();
    frame.push_line("repository heading");
    frame.push_line("unpublished branch");
    for number in numbers {
        let context = context(*number, "owner/repo");
        frame.push_pr_line(
            &format!(
                "  {} title",
                osc8_link(&context.pr_url, &format!("#{number}"))
            ),
            Some(context),
        );
    }
    frame
}

#[test]
fn selection_tracks_identity_through_reordering_and_disappearance() {
    let mut nav = DashboardNavigation::default();
    let first = frame(&[3, 2, 1]);
    nav.reconcile(Some(&first));
    nav.handle_command(DashboardCommand::Down, &first, 10);
    assert_eq!(nav.selected(&first).unwrap().pr_number, 2);
    let sorted = frame(&[2, 3, 1]);
    nav.reconcile(Some(&sorted));
    assert_eq!(nav.selected(&sorted).unwrap().pr_number, 2);
    assert_eq!(nav.selected_line, Some(2));
    let removed = frame(&[3, 1]);
    nav.reconcile(Some(&removed));
    assert_eq!(nav.selected(&removed).unwrap().pr_number, 3);
    nav.reconcile(Some(&frame(&[])));
    assert_eq!(nav.selected_line, None);
    assert_eq!(nav.scroll_top, 0);
}

#[test]
fn removal_keeps_focus_in_the_repository_even_when_another_group_follows() {
    let first = grouped_frame(&[
        ("owner/repo", None, &[1, 2, 3]),
        ("owner/other", None, &[4, 5]),
    ]);
    for (selected, remaining, expected) in
        [(3, vec![1, 2], 2), (2, vec![1, 3], 3), (1, vec![2, 3], 2)]
    {
        let mut nav = DashboardNavigation::default();
        nav.reconcile(Some(&first));
        for _ in 1..selected {
            nav.handle_command(DashboardCommand::Down, &first, 10);
        }
        let removed = grouped_frame(&[
            ("owner/repo", None, &remaining),
            ("owner/other", None, &[4, 5]),
        ]);
        nav.reconcile(Some(&removed));
        assert_eq!(nav.selected(&removed).unwrap().pr_number, expected);
        assert_eq!(
            nav.selected(&removed).unwrap().repository.slug(),
            "owner/repo"
        );
        assert_eq!(
            nav.selected_line,
            Some(
                removed
                    .rows
                    .iter()
                    .find(|row| row.context.pr_number == expected)
                    .unwrap()
                    .line
            )
        );
    }
}

#[test]
fn repository_local_position_is_independent_of_changes_to_other_groups() {
    let first = grouped_frame(&[
        ("owner/before", None, &[9, 10]),
        ("owner/repo", None, &[1, 2, 3, 6]),
        ("owner/other", None, &[4, 5]),
    ]);
    for refreshed in [
        grouped_frame(&[
            ("owner/repo", None, &[1, 3, 6]),
            ("owner/other", None, &[4, 5]),
        ]),
        grouped_frame(&[
            ("owner/before", None, &[9, 10, 11, 12]),
            ("owner/repo", None, &[1, 3, 6]),
            ("owner/other", None, &[4, 5]),
        ]),
        grouped_frame(&[
            ("owner/other", None, &[4, 5]),
            ("owner/repo", None, &[1, 3, 6]),
            ("owner/before", None, &[9, 10]),
        ]),
    ] {
        let mut nav = DashboardNavigation::default();
        nav.reconcile(Some(&first));
        for _ in 0..3 {
            nav.handle_command(DashboardCommand::Down, &first, 10);
        }
        assert_eq!(nav.selected(&first).unwrap().pr_number, 2);
        nav.reconcile(Some(&refreshed));
        assert_eq!(nav.selected(&refreshed).unwrap().pr_number, 3);
    }
}

#[test]
fn removed_selection_prefers_its_checkout_over_an_identical_pr_in_another_clone() {
    let first = grouped_frame(&[
        ("owner/repo", Some("/first"), &[1, 2, 3]),
        ("owner/repo", Some("/second"), &[3]),
    ]);
    let mut nav = DashboardNavigation::default();
    nav.reconcile(Some(&first));
    nav.handle_command(DashboardCommand::Down, &first, 10);
    nav.handle_command(DashboardCommand::Down, &first, 10);
    let removed = grouped_frame(&[
        ("owner/repo", Some("/first"), &[1, 2]),
        ("owner/repo", Some("/second"), &[3]),
    ]);
    nav.reconcile(Some(&removed));
    let selected = nav.selected(&removed).unwrap();
    assert_eq!(selected.pr_number, 2);
    assert_eq!(
        selected.repository_root.as_deref(),
        Some(Path::new("/first"))
    );
}

#[test]
fn selection_can_leave_a_repository_when_its_last_pr_disappears() {
    for (groups, initial_index, expected) in [
        (
            vec![
                ("owner/repo", None, vec![1]),
                ("owner/other", None, vec![4, 5]),
            ],
            0,
            4,
        ),
        (
            vec![
                ("owner/other", None, vec![4, 5]),
                ("owner/repo", None, vec![1]),
            ],
            2,
            5,
        ),
    ] {
        let borrowed = groups
            .iter()
            .map(|(repo, root, numbers)| (*repo, *root, numbers.as_slice()))
            .collect::<Vec<_>>();
        let first = grouped_frame(&borrowed);
        let mut nav = DashboardNavigation::default();
        nav.reconcile(Some(&first));
        for _ in 0..initial_index {
            nav.handle_command(DashboardCommand::Down, &first, 10);
        }
        assert_eq!(nav.selected(&first).unwrap().pr_number, 1);
        let removed = grouped_frame(&[("owner/other", None, &[4, 5])]);
        nav.reconcile(Some(&removed));
        assert_eq!(nav.selected(&removed).unwrap().pr_number, expected);
    }
}

#[test]
fn identical_pr_numbers_in_other_repositories_and_clones_do_not_steal_selection() {
    let mut first = frame(&[12]);
    let mut clone = context(12, "owner/repo");
    clone.repository_root = Some(PathBuf::from("/clone"));
    first.push_pr_line("  #12 clone", Some(clone));
    first.push_pr_line("  #12 other", Some(context(12, "owner/other")));
    let mut nav = DashboardNavigation::default();
    nav.reconcile(Some(&first));
    nav.handle_command(DashboardCommand::Down, &first, 10);
    first.rows.reverse();
    nav.reconcile(Some(&first));
    assert_eq!(
        nav.selected(&first).unwrap().repository_root.as_deref(),
        Some(Path::new("/clone"))
    );
    nav.handle_command(DashboardCommand::First, &first, 10);
    assert_eq!(
        nav.selected(&first).unwrap().repository.slug(),
        "owner/other"
    );
}

#[test]
fn viewport_skips_non_pr_lines_for_selection_and_preserves_osc8_bytes() {
    let frame = frame(&[1, 2, 3, 4, 5]);
    let mut nav = DashboardNavigation::default();
    nav.reconcile(Some(&frame));
    nav.handle_command(DashboardCommand::Last, &frame, 3);
    let (text, marker) = nav.viewport(&frame.text, 3);
    assert_eq!(marker, Some(2));
    assert_eq!(text.lines().last(), frame.text.lines().last());
    assert!(text.contains(&osc8_link("https://github.com/owner/repo/pull/5", "#5")));
    nav.handle_command(DashboardCommand::First, &frame, 3);
    let (_, marker) = nav.viewport(&frame.text, 20);
    assert_eq!(marker, Some(2));
    assert_eq!(nav.scroll_top, 0);
    let (text, marker) = nav.viewport(&frame.text, 0);
    assert!(text.is_empty());
    assert_eq!(marker, None);
}

fn grouped_frame(groups: &[(&str, Option<&str>, &[u64])]) -> PullRequestTableFrame {
    let mut frame = PullRequestTableFrame::default();
    for (repository, root, numbers) in groups {
        frame.push_line(repository);
        for number in *numbers {
            let mut context = context(*number, repository);
            context.repository_root = root.map(PathBuf::from);
            frame.push_pr_line(&format!("PR {number}"), Some(context));
        }
    }
    frame
}
