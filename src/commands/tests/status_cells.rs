use super::*;

const RED_BOLD_STYLE: &str = "\x1b[1m\x1b[31m";
const YELLOW_STYLE: &str = "\x1b[33m";
const CYAN_STYLE: &str = "\x1b[36m";

#[test]
fn check_cells_use_colored_labels_or_plain_symbols_and_blank_unavailable_status() {
    let mut status = review_status_record(12, "Title", "author", false);
    for (check_status, symbol, style) in [
        (PullRequestCheckStatus::Passing, "✓", GREEN_STYLE),
        (PullRequestCheckStatus::Failing, "✗", RED_BOLD_STYLE),
        (PullRequestCheckStatus::Pending, "◷", YELLOW_STYLE),
        (PullRequestCheckStatus::Missing, "", ""),
        (PullRequestCheckStatus::Unknown, "", ""),
    ] {
        status.check_status = check_status;
        for color in [false, true] {
            let cell = pull_request_check_cell(Some(&status), false, color, DRAFT_ROW_STYLE);
            let expected = if symbol.is_empty() {
                "   ".to_owned()
            } else if color {
                format!("{style}Chk{RESET_STYLE}{DRAFT_ROW_STYLE}")
            } else {
                format!("{symbol:<3}")
            };
            assert_eq!(cell, expected);
            assert_eq!(rendered_visible_width(&cell), 3);
        }
    }
    assert_eq!(pull_request_check_cell(None, false, true, ""), "   ");
    assert_eq!(
        pull_request_check_cell(None, true, true, ""),
        format!("{GREEN_STYLE}Chk{RESET_STYLE}"),
    );
}

#[test]
fn stack_review_cells_leave_drafts_and_undefined_reviews_blank() {
    let mut status = review_status_record(12, "Title", "author", true);
    status.review_status = PullRequestReviewStatus::Approved;
    status.approved_reviewers = vec!["reviewer".to_owned()];
    for color in [false, true] {
        assert_eq!(
            pull_request_review_cell(Some(&status), false, color, false, ""),
            "   "
        );
        assert_eq!(
            pull_request_review_cell(None, false, color, false, ""),
            "   "
        );
    }
    status.draft = false;
    status.base_branch = "topic/parent".to_owned();
    assert_eq!(
        pull_request_review_cell(Some(&status), false, true, false, ""),
        "   "
    );
    status.base_branch = "main".to_owned();
    status.approved_reviewers.clear();
    status.requested_reviewers = ReviewerSelection::default();
    for review_status in [
        PullRequestReviewStatus::Unknown,
        PullRequestReviewStatus::NotReviewed,
    ] {
        status.review_status = review_status;
        assert_eq!(
            pull_request_review_cell(Some(&status), false, true, false, ""),
            "   "
        );
    }
    assert_eq!(
        pull_request_review_cell(Some(&status), true, true, false, ""),
        format!("{GREEN_STYLE}Rev{RESET_STYLE}"),
    );
}

#[test]
fn stack_review_cells_preserve_review_state_colors_and_plain_symbols() {
    let mut status = review_status_record(12, "Title", "author", false);
    for (review_status, overdue, symbol, style) in [
        (PullRequestReviewStatus::Approved, false, "✓", GREEN_STYLE),
        (
            PullRequestReviewStatus::ChangesRequested,
            false,
            "!",
            RED_BOLD_STYLE,
        ),
        (
            PullRequestReviewStatus::ReviewRequested,
            false,
            "?",
            CYAN_STYLE,
        ),
        (
            PullRequestReviewStatus::ReviewRequested,
            true,
            "?",
            RED_BOLD_STYLE,
        ),
    ] {
        status.review_status = review_status;
        for color in [false, true] {
            let cell = pull_request_review_cell(Some(&status), false, color, overdue, "");
            let expected = if color {
                format!("{style}Rev{RESET_STYLE}")
            } else {
                format!("{symbol:<3}")
            };
            assert_eq!(cell, expected);
            assert_eq!(rendered_visible_width(&cell), 3);
        }
    }
}

#[test]
fn both_tables_keep_compact_columns_aligned_with_blank_statuses() {
    let mut draft = review_status_record(13, "Aligned title", "author", true);
    draft.check_status = PullRequestCheckStatus::Missing;
    let statuses = [
        review_status_record(12, "Aligned title", "author", false),
        draft,
        review_status_record(14, "Aligned title", "author", false),
    ];
    let repository = ReviewRequestRepositoryView {
        repository: GitHubRepository {
            owner: "example-owner".to_owned(),
            name: "repo".to_owned(),
        },
        layout_key: None,
        root: None,
        display_root: None,
        rows: statuses
            .iter()
            .map(|status| ReviewRequestRowView {
                status: status.clone(),
                state: crate::domain::ReviewRequestState::New,
                viewer_signal: ReviewRequestViewerSignal::None,
                lag_since_unix: None,
                dismissal: None,
            })
            .collect(),
        external: false,
        review_wait_threshold_seconds: None,
    };
    let view = ReviewRequestsView {
        viewer: "example-reviewer".to_owned(),
        repositories: vec![repository.clone(), repository],
    };
    let pull_requests = statuses
        .iter()
        .map(|status| {
            pull_request_choice_record(
                status.number,
                &status.title,
                &status.head_branch,
                "main",
                status.draft,
            )
        })
        .collect::<Vec<_>>();
    let report = PullRequestStackStatusReport {
        repository: preview_plan().repository,
        snapshot: PullRequestStackSnapshot::from_metadata(
            &StackMetadata::default(),
            &[],
            &pull_requests,
            PullRequestStackSelection::default(),
        ),
        statuses: statuses
            .into_iter()
            .map(|status| (status.number, status))
            .collect(),
        trunk: None,
        review_wait_threshold_seconds: None,
    };
    let entry = GlobalStackStatusEntry::current(PathBuf::from("/repo"), &report);
    for color in [false, true] {
        for layout in [
            PullRequestTableLayout::Flow,
            PullRequestTableLayout::FitTerminal,
        ] {
            let review = render_review_requests(&view, color, Some(120), layout, &BTreeMap::new());
            let stack = render_global_stack_status(
                &[entry.clone(), entry.clone()],
                2,
                Path::new("/repo"),
                color,
                Some(120),
                layout,
                &BTreeMap::new(),
            )
            .expect("global stack renders");
            for output in [review, stack] {
                assert_eq!(
                    output.matches("Chk Rev Lag").count(),
                    if color { 0 } else { 2 }
                );
                let rows = output
                    .lines()
                    .filter(|line| line.contains("Aligned title"))
                    .collect::<Vec<_>>();
                assert_eq!(rows.len(), 6);
                for row in rows {
                    let before_title = row.split_once("Aligned title").expect("title exists").0;
                    // Two-space indentation, four one-space separators, and a two-column lifecycle marker.
                    let title_column =
                        2 + PULL_REQUEST_STATUS_PR_WIDTH + 3 + 3 + REVIEW_LAG_WIDTH + 4 + 2;
                    assert_eq!(
                        rendered_visible_width(before_title),
                        title_column,
                        "{row:?}"
                    );
                    if row.contains("#13") {
                        assert!(!row.contains("Chk"), "{row:?}");
                        assert!(!row.contains("Rev"), "{row:?}");
                    } else if color {
                        assert!(row.contains("Chk"), "{row:?}");
                        assert!(row.contains("Rev"), "{row:?}");
                    }
                    assert!(rendered_visible_width(row) <= 120);
                }
            }
        }
    }
}
