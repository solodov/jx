use super::*;

#[test]
fn stack_frames_skip_headers_errors_and_unpublished_branches_without_losing_links() {
    let mut snapshot = PullRequestStackSnapshot::from_metadata(
        &StackMetadata::default(),
        &[],
        &[pull_request_choice_record(
            12,
            "Full PR title",
            "feature",
            "main",
            false,
        )],
        PullRequestStackSelection::default(),
    );
    let mut unpublished = snapshot.nodes[0].clone();
    unpublished.pull_request = None;
    unpublished.branch = "unpublished".to_owned();
    unpublished.title = "Unpublished change".to_owned();
    snapshot.nodes.push(unpublished);
    let report = PullRequestStackStatusReport {
        repository: preview_plan().repository,
        snapshot,
        statuses: BTreeMap::new(),
        trunk: None,
        review_wait_threshold_seconds: None,
    };
    let mut empty = report.clone();
    empty.snapshot.nodes.clear();
    let entries = [
        GlobalStackStatusEntry {
            key: Some("failed".to_owned()),
            root: PathBuf::from("/failed"),
            display_root: "/failed".to_owned(),
            repository: None,
            result: Err("#999 failed\nmore detail".to_owned()),
        },
        GlobalStackStatusEntry::current(PathBuf::from("/empty"), &empty),
        GlobalStackStatusEntry::current(PathBuf::from("/correct-root"), &report),
    ];
    for color in [false, true] {
        let frame = render_global_stack_status(
            &entries,
            3,
            Path::new("/unrelated-caller"),
            color,
            Some(120),
            PullRequestTableLayout::FitTerminal,
            &BTreeMap::new(),
        );
        assert_eq!(frame.rows.len(), 1);
        let row = &frame.rows[0];
        assert_eq!(row.context.pr_number, 12);
        assert_eq!(row.context.title, "Full PR title");
        assert_eq!(
            row.context.repository_root.as_deref(),
            Some(Path::new("/correct-root"))
        );
        assert_eq!(row.context.head_oid, None);
        assert_eq!(row.context.local_commit_id, None);
        assert!(frame.text.contains("No stack state"));
        assert!(frame.text.contains("Unpublished change"));
        let text = frame.text.lines().nth(row.line).expect("PR row exists");
        assert!(
            text.contains(&osc8_link(&row.context.pr_url, "#12")),
            "{text:?}"
        );
    }
}

#[test]
fn review_frames_keep_clickable_links_and_distinct_targets_across_resizes() {
    let title =
        "A complete PR title that must remain available even when the visible title is clipped";
    let repositories = [
        ("local-repo", Some(PathBuf::from("/checkout/local"))),
        ("external-repo", None),
    ]
    .into_iter()
    .map(|(name, root)| {
        let mut status = review_status_record(12, title, "author-login", false);
        status.url = None;
        ReviewRequestRepositoryView {
            repository: GitHubRepository {
                owner: "owner".to_owned(),
                name: name.to_owned(),
            },
            layout_key: None,
            external: root.is_none(),
            root,
            display_root: None,
            rows: vec![ReviewRequestRowView {
                status,
                state: crate::domain::ReviewRequestState::New,
                viewer_signal: ReviewRequestViewerSignal::None,
                lag_since_unix: None,
                dismissal: None,
            }],
            review_wait_threshold_seconds: None,
        }
    })
    .collect();
    let view = ReviewRequestsView {
        viewer: "example-reviewer".to_owned(),
        repositories,
    };
    let wide = render_review_requests(
        &view,
        true,
        Some(160),
        PullRequestTableLayout::FitTerminal,
        &BTreeMap::new(),
    );
    let expected_keys = wide
        .rows
        .iter()
        .map(|row| row.context.key())
        .collect::<Vec<_>>();
    assert_ne!(expected_keys[0], expected_keys[1]);
    for color in [false, true] {
        for width in [24, 60, 160] {
            for layout in [
                PullRequestTableLayout::Flow,
                PullRequestTableLayout::FitTerminal,
            ] {
                let frame =
                    render_review_requests(&view, color, Some(width), layout, &BTreeMap::new());
                assert_eq!(
                    frame
                        .rows
                        .iter()
                        .map(|row| row.context.key())
                        .collect::<Vec<_>>(),
                    expected_keys
                );
                assert_eq!(
                    frame.rows[0].context.repository_root.as_deref(),
                    Some(Path::new("/checkout/local"))
                );
                assert_eq!(frame.rows[1].context.repository_root, None);
                for row in &frame.rows {
                    let text = frame.text.lines().nth(row.line).expect("PR line exists");
                    assert_eq!(row.context.title, title);
                    assert_eq!(row.context.branch, "topic/review-12");
                    assert_eq!(row.context.head_oid.as_deref(), Some("commit-12"));
                    assert_eq!(row.context.local_commit_id, None);
                    assert_eq!(row.context.local_change_id, None);
                    if color {
                        assert!(
                            text.contains(&osc8_link(&row.context.pr_url, "#12")),
                            "{text:?}"
                        );
                    }
                }
            }
        }
    }
}
