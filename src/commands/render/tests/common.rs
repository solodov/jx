use super::*;

#[test]
fn elastic_table_row_shrinks_title_above_minimum_before_right_metadata() {
    let row = render_elastic_table_row(
        "  #12      ✓    ?    <1h   ",
        "Implement a very long synthetic pull request title",
        "[workflow]",
        "Example Reviewer",
        Some(100),
    );

    assert_eq!(rendered_visible_width(&row), 100);
    assert!(row.ends_with("Example Reviewer"));
    assert!(row.contains("… [workflow]"));
    assert!(!row.contains("request title"));
}

#[test]
fn elastic_table_row_right_aligns_metadata_when_title_fits() {
    let row = render_elastic_table_row(
        "  #12      ✓    ?    <1h   ",
        "Short title",
        "[workflow]",
        "Example Reviewer",
        Some(72),
    );

    assert_eq!(rendered_visible_width(&row), 72);
    assert!(row.ends_with("Example Reviewer"));
    assert!(row.contains("Short title [workflow]"));
}

#[test]
fn elastic_table_row_reserves_forty_columns_before_crowded_metadata() {
    let title = "t".repeat(80);
    let reviewers = "Reviewer One, Reviewer Two, Reviewer Three, Reviewer Four";
    let row = render_elastic_table_row("#12  ", &title, "[bug]", reviewers, Some(80));

    assert_eq!(rendered_visible_width(&row), 80);
    assert_eq!(
        row,
        format!(
            "#12  {}… [bug]  Reviewer One, Reviewer Two…",
            "t".repeat(39)
        ),
    );
}

#[test]
fn elastic_table_row_reserves_only_the_actual_width_of_short_titles() {
    let row = render_elastic_table_row(
        "#12  ",
        "Short title",
        "",
        "Reviewer One, Reviewer Two, Reviewer Three",
        Some(40),
    );

    assert_eq!(row, "#12  Short title  Reviewer One, Reviewe…");
}

#[test]
fn elastic_table_row_uses_last_column_without_reducing_title_minimum() {
    let row = render_elastic_table_row("#12  ", &"t".repeat(80), "", "Reviewer", Some(48));

    assert_eq!(row, format!("#12  {}…  …", "t".repeat(39)));
    assert_eq!(rendered_visible_width(&row), 48);
}

#[test]
fn elastic_table_row_prioritizes_title_over_long_labels() {
    let row = render_elastic_table_row(
        "#12  ",
        &"t".repeat(80),
        &"label".repeat(20),
        "Reviewer",
        Some(60),
    );

    assert_eq!(row, format!("#12  {}… labellabellab…", "t".repeat(39)),);
}

#[test]
fn elastic_table_row_prioritizes_title_on_narrow_terminals() {
    let prefix = "#12  ";
    let title = "t".repeat(80);
    for width in 0..=45 {
        let row = render_elastic_table_row(prefix, &title, "[bug]", "Reviewer", Some(width));

        assert_eq!(rendered_visible_width(&row), width, "width: {width}");
        assert!(!row.contains("[bug]"));
        assert!(!row.contains("Reviewer"));
        if width > prefix.len() {
            assert_eq!(
                row,
                format!("{prefix}{}…", "t".repeat(width - prefix.len() - 1)),
            );
        }
    }
}

#[test]
fn elastic_table_row_uses_spare_space_for_title_without_metadata() {
    let title = "t".repeat(80);
    let row = render_elastic_table_row("#12  ", &title, "", "", Some(60));

    assert_eq!(row, format!("#12  {}…", "t".repeat(54)));
    assert_eq!(
        render_elastic_table_row("#12  ", "Short title", "", "", Some(60)),
        "#12  Short title",
    );
}

#[test]
fn elastic_table_row_preserves_flow_when_terminal_width_is_unknown() {
    let title = "t".repeat(80);
    let row = render_elastic_table_row("#12  ", &title, "[bug]", "Reviewer", None);

    assert_eq!(row, format!("#12  {title} [bug]  Reviewer"));
}

#[test]
fn flow_rows_keep_two_spaces_before_reviewers_without_adding_trailing_padding() {
    assert_eq!(
        flow_table_row("#12 ", "Title", "", "Reviewer"),
        "#12 Title  Reviewer"
    );
    assert_eq!(
        flow_table_row("#12 ", "Title", "[bug]", "Reviewer"),
        "#12 Title [bug]  Reviewer"
    );
    assert_eq!(
        flow_table_row("#12 ", "Title", "[bug]", ""),
        "#12 Title [bug]"
    );
    assert_eq!(flow_table_row("#12 ", "Title", "", ""), "#12 Title");
}

#[test]
fn elastic_table_row_truncates_styled_metadata_without_leaking_links_or_styles() {
    let title = format!("{GREEN_STYLE}{}{RESET_STYLE}", "t".repeat(80));
    let right = osc8_link(
        "https://github.com/reviewer",
        &format!("{BOLD_STYLE}{}{RESET_STYLE}", "Reviewer".repeat(10)),
    );
    let row = render_elastic_table_row("#12  ", &title, "", &right, Some(60));

    assert_eq!(rendered_visible_width(&row), 60);
    assert!(row.starts_with(&format!(
        "#12  {GREEN_STYLE}{}…{RESET_STYLE}  ",
        "t".repeat(39)
    )));
    assert!(row.ends_with(&format!("ReviewerRevi…\x1b]8;;\x1b\\{RESET_STYLE}")));
    assert_eq!(
        row.matches("\x1b]8;;https://github.com/reviewer").count(),
        1
    );
    assert_eq!(row.matches("\x1b]8;;\x1b\\").count(), 1);
}
