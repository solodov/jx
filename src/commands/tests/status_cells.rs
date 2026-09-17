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
        for row_style in ["", DRAFT_ROW_STYLE, PASTEL_BLUE_STYLE, CONFLICT_STYLE] {
            for color in [false, true] {
                let cell = pull_request_check_cell(Some(&status), false, color, row_style);
                let expected = if symbol.is_empty() {
                    "   ".to_owned()
                } else if color {
                    format!("\x1b[22m{style}Chk{RESET_STYLE}{row_style}")
                } else {
                    format!("{symbol:<3}")
                };
                assert_eq!(cell, expected);
                assert_eq!(rendered_visible_width(&cell), 3);
            }
        }
    }
    assert_eq!(pull_request_check_cell(None, false, true, ""), "   ");
    assert_eq!(
        pull_request_check_cell(None, true, true, ""),
        format!("\x1b[22m{GREEN_STYLE}Chk{RESET_STYLE}"),
    );
}

#[test]
fn stack_review_cells_require_actual_reviewers_in_every_lifecycle() {
    let mut status = review_status_record(12, "Title", "author", false);
    status.requested_reviewers = ReviewerSelection::default();
    for suggestions in [vec![], vec!["suggested-reviewer".to_owned()]] {
        status.suggested_reviewers = suggestions;
        for review_status in [
            PullRequestReviewStatus::Unknown,
            PullRequestReviewStatus::NotReviewed,
            PullRequestReviewStatus::ReviewRequired,
            PullRequestReviewStatus::ReviewRequested,
            PullRequestReviewStatus::Approved,
            PullRequestReviewStatus::ChangesRequested,
        ] {
            status.review_status = review_status;
            for (draft, closed, merged) in [
                (false, false, false),
                (true, false, false),
                (false, true, false),
                (false, true, true),
            ] {
                status.draft = draft;
                status.closed = closed;
                status.merged = merged;
                for color in [false, true] {
                    assert_eq!(
                        pull_request_review_cell(Some(&status), merged, color, false, ""),
                        "   "
                    );
                    assert_eq!(
                        pull_request_review_cell(None, merged, color, false, ""),
                        "   "
                    );
                }
            }
        }
    }
}

#[test]
fn reviewer_presence_counts_requests_and_past_reviews_but_not_suggestions() {
    let sources: &[fn(&mut PullRequestStatusRecord)] = &[
        |status| status.requested_reviewers.users.push("reviewer".to_owned()),
        |status| status.requested_reviewers.teams.push("platform".to_owned()),
        |status| status.approved_reviewers.push("reviewer".to_owned()),
        |status| {
            status
                .changes_requested_reviewers
                .push("reviewer".to_owned())
        },
        |status| status.commented_reviewers.push("reviewer".to_owned()),
        |status| status.addressed_reviewers.push("reviewer".to_owned()),
        |status| status.dismissed_reviewers.push("reviewer".to_owned()),
        |status| {
            status.review_activity.push(PullRequestReviewActivity {
                reviewer: "reviewer".to_owned(),
                reviewed_at: "2026-01-01T00:00:00Z".to_owned(),
            })
        },
    ];
    for add_reviewer in sources {
        let mut status = review_status_record(12, "Title", "author", true);
        status.requested_reviewers = ReviewerSelection::default();
        status.suggested_reviewers = vec!["suggested-reviewer".to_owned()];
        assert!(!pull_request_has_reviewers(&status));
        add_reviewer(&mut status);
        assert!(pull_request_has_reviewers(&status));
    }
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
        for (draft, closed, row_style) in [
            (false, false, ""),
            (true, false, DRAFT_ROW_STYLE),
            (false, true, PASTEL_BLUE_STYLE),
        ] {
            status.draft = draft;
            status.closed = closed;
            status.base_branch = "topic/parent".to_owned();
            for color in [false, true] {
                let cell =
                    pull_request_review_cell(Some(&status), false, color, overdue, row_style);
                let expected = if color {
                    format!("\x1b[22m{style}Rev{RESET_STYLE}{row_style}")
                } else {
                    format!("{symbol:<3}")
                };
                assert_eq!(cell, expected);
                assert_eq!(rendered_visible_width(&cell), 3);
            }
        }
    }
}

#[test]
fn stack_review_cells_keep_submitted_reviews_visible_after_requests_clear() {
    let mut approved = review_status_record(12, "Title", "author", false);
    approved.requested_reviewers = ReviewerSelection::default();
    approved.review_status = PullRequestReviewStatus::Approved;
    approved.approved_reviewers = vec!["reviewer".to_owned()];
    let mut changed = approved.clone();
    changed.approved_reviewers.clear();
    changed.review_status = PullRequestReviewStatus::Unknown;
    changed.changes_requested_reviewers = vec!["reviewer".to_owned()];
    let mut commented = changed.clone();
    commented.changes_requested_reviewers.clear();
    commented.commented_reviewers = vec!["reviewer".to_owned()];
    for (mut status, style, symbol) in [
        (approved, GREEN_STYLE, "✓"),
        (changed, RED_BOLD_STYLE, "!"),
        (commented, "\x1b[38;2;194;95;0m", "!"),
    ] {
        for draft in [false, true] {
            status.draft = draft;
            let row_style = if draft { DRAFT_ROW_STYLE } else { "" };
            assert_eq!(
                pull_request_review_cell(Some(&status), false, true, false, row_style),
                format!("\x1b[22m{style}Rev{RESET_STYLE}{row_style}"),
            );
            assert_eq!(
                pull_request_review_cell(Some(&status), false, false, false, row_style),
                format!("{symbol:<3}"),
            );
        }
    }
}

#[test]
fn both_tables_preserve_status_colors_and_alignment_on_subdued_rows() {
    let mut draft = review_status_record(13, "Aligned title", "author", true);
    draft.check_status = PullRequestCheckStatus::Missing;
    draft.requested_reviewers = ReviewerSelection::default();
    draft.suggested_reviewers = vec!["suggested-reviewer".to_owned()];
    let mut reviewed = review_status_record(15, "Aligned title", "author", true);
    reviewed.requested_reviewers = ReviewerSelection::default();
    reviewed.approved_reviewers = vec!["example-reviewer".to_owned()];
    reviewed.review_status = PullRequestReviewStatus::Approved;
    let mut closed = review_status_record(16, "Aligned title", "author", false);
    closed.closed = true;
    let mut no_reviewers = review_status_record(17, "Aligned title", "author", false);
    no_reviewers.requested_reviewers = ReviewerSelection::default();
    no_reviewers.review_status = PullRequestReviewStatus::Approved;
    let mut merged = no_reviewers.clone();
    merged.number = 18;
    merged.head_branch = "topic/merged".to_owned();
    merged.merged = true;
    merged.closed = true;
    let mut conflict = review_status_record(19, "Aligned title", "author", true);
    conflict.merge_status = PullRequestMergeStatus::Conflicting;
    conflict.auto_merge_status = PullRequestAutoMergeStatus::Missing;
    let statuses = [
        review_status_record(12, "Aligned title", "author", false),
        draft,
        review_status_record(14, "Aligned title", "author", true),
        reviewed,
        closed,
        no_reviewers,
        merged,
        conflict,
    ];
    let row_count = statuses.len();
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
                state: if status.approved_reviewers.is_empty() {
                    crate::domain::ReviewRequestState::New
                } else {
                    crate::domain::ReviewRequestState::Approved
                },
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
            );
            for frame in [review, stack] {
                assert_eq!(frame.rows.len(), 2 * row_count);
                for row in &frame.rows {
                    assert_eq!(row.context.title, "Aligned title");
                    assert!(frame
                        .text
                        .lines()
                        .nth(row.line)
                        .expect("PR row has a line")
                        .contains(&format!("#{}", row.context.pr_number)));
                }
                let output = frame.text;
                assert_eq!(
                    output.matches("Chk Rev Lag").count(),
                    if color { 0 } else { 2 }
                );
                let rows = output
                    .lines()
                    .filter(|line| line.contains("Aligned title"))
                    .collect::<Vec<_>>();
                assert_eq!(rows.len(), 2 * row_count);
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
                    let draft = [13, 14, 15, 19]
                        .iter()
                        .any(|number| row.contains(&format!("#{number}")));
                    let row_style = if draft {
                        DRAFT_ROW_STYLE
                    } else if row.contains("#16") {
                        PASTEL_BLUE_STYLE
                    } else {
                        ""
                    };
                    if row.contains("#13") {
                        assert!(!row.contains("Chk"), "{row:?}");
                    } else if color {
                        assert!(
                            row.contains(&format!(
                                "\x1b[22m{GREEN_STYLE}Chk{RESET_STYLE}{row_style}"
                            )),
                            "{row:?}"
                        );
                    }
                    if [13, 17, 18]
                        .iter()
                        .any(|number| row.contains(&format!("#{number}")))
                    {
                        assert!(!row.contains("Rev"), "{row:?}");
                        if !color {
                            let before_lag = row.split_once('—').expect("lag renders").0;
                            assert!(before_lag.ends_with("    "), "{row:?}");
                        }
                    } else if color {
                        let review_style = if row.contains("#15") {
                            GREEN_STYLE
                        } else {
                            CYAN_STYLE
                        };
                        assert!(
                            row.contains(&format!(
                                "\x1b[22m{review_style}Rev{RESET_STYLE}{row_style}"
                            )),
                            "{row:?}"
                        );
                    }
                    if color && draft {
                        assert!(row.starts_with(DRAFT_ROW_STYLE), "{row:?}");
                        assert!(row.contains(&format!("\x1b[22m\x1b[48;2;232;232;232m\x1b[38;2;98;98;98m backend {RESET_STYLE}{DRAFT_ROW_STYLE}")), "{row:?}");
                        assert!(!row.contains(BOLD_STYLE), "{row:?}");
                        assert!(!row.contains("\x1b[38;2;194;95;0m"), "{row:?}");
                        assert!(row.ends_with(RESET_STYLE), "{row:?}");
                    } else if color && row.contains("#16") {
                        assert!(row.starts_with(PASTEL_BLUE_STYLE), "{row:?}");
                        assert!(!row.contains("backend"), "{row:?}");
                    }
                    assert!(rendered_visible_width(row) <= 120);
                }
            }
        }
    }
}
