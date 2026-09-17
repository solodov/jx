use super::*;

#[test]
fn author_first_names_and_login_fallbacks_work_in_both_table_layouts() {
    for draft in [false, true] {
        let view = ReviewRequestsView {
            viewer: "example-reviewer".to_owned(),
            repositories: vec![ReviewRequestRepositoryView {
                repository: GitHubRepository {
                    owner: "example-owner".to_owned(),
                    name: "api-alpha".to_owned(),
                },
                layout_key: None,
                root: None,
                display_root: None,
                rows: vec![ReviewRequestRowView {
                    status: review_status_record(12, "A short title", "example-author", draft),
                    state: crate::domain::ReviewRequestState::New,
                    viewer_signal: ReviewRequestViewerSignal::None,
                    lag_since_unix: None,
                    dismissal: None,
                }],
                external: false,
                review_wait_threshold_seconds: None,
            }],
        };
        for (name, expected) in [
            (Some("Alice Example"), "Alice"),
            (Some("  Élodie\u{a0}Martin  "), "Élodie"),
            (Some("Jean-Luc Picard"), "Jean-Luc"),
            (Some(""), "example-author"),
            (Some(" \t\n"), "example-author"),
            (None, "example-author"),
        ] {
            let display_names = name
                .map(|name| ("example-author".to_owned(), name.to_owned()))
                .into_iter()
                .collect();
            for color in [false, true] {
                for layout in [
                    PullRequestTableLayout::Flow,
                    PullRequestTableLayout::FitTerminal,
                ] {
                    let output =
                        render_review_requests(&view, color, Some(100), layout, &display_names);
                    let row = output
                        .lines()
                        .find(|line| line.contains("#12"))
                        .expect("review row renders");
                    let suffix = if color && draft {
                        format!("  {expected}{RESET_STYLE}")
                    } else if color {
                        format!("  {BOLD_STYLE}{expected}{RESET_STYLE}")
                    } else {
                        format!("  {expected}")
                    };
                    assert!(row.ends_with(&suffix), "{row:?}");
                    assert!(rendered_visible_width(row) <= 100, "{row:?}");
                    if color && draft {
                        assert!(row.starts_with(DRAFT_ROW_STYLE), "{row:?}");
                        assert!(!row.contains(BOLD_STYLE), "{row:?}");
                    }
                }
            }
        }
    }
}
