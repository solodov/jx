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
    nav.handle_key(KeyCode::Down, &first, 10);
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
fn identical_pr_numbers_in_other_repositories_and_clones_do_not_steal_selection() {
    let mut first = frame(&[12]);
    let mut clone = context(12, "owner/repo");
    clone.repository_root = Some(PathBuf::from("/clone"));
    first.push_pr_line("  #12 clone", Some(clone));
    first.push_pr_line("  #12 other", Some(context(12, "owner/other")));
    let mut nav = DashboardNavigation::default();
    nav.reconcile(Some(&first));
    nav.handle_key(KeyCode::Down, &first, 10);
    first.rows.reverse();
    nav.reconcile(Some(&first));
    assert_eq!(
        nav.selected(&first).unwrap().repository_root.as_deref(),
        Some(Path::new("/clone"))
    );
    nav.handle_key(KeyCode::Home, &first, 10);
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
    nav.handle_key(KeyCode::End, &frame, 3);
    let (text, marker) = nav.viewport(&frame.text, 3);
    assert_eq!(marker, Some(2));
    assert_eq!(text.lines().last(), frame.text.lines().last());
    assert!(text.contains(&osc8_link("https://github.com/owner/repo/pull/5", "#5")));
    nav.handle_key(KeyCode::Home, &frame, 3);
    let (_, marker) = nav.viewport(&frame.text, 20);
    assert_eq!(marker, Some(2));
    assert_eq!(nav.scroll_top, 0);
    let (text, marker) = nav.viewport(&frame.text, 0);
    assert!(text.is_empty());
    assert_eq!(marker, None);
}
