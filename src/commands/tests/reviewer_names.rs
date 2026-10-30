use super::*;

#[test]
fn reviewer_picker_and_selection_summary_use_full_names_with_login_fallbacks() {
    for (name, expected) in [
        (Some("Alice Example"), "Alice Example"),
        (Some("  Élodie\u{a0}Martin  "), "Élodie\u{a0}Martin"),
        (Some("Jean-Luc Picard"), "Jean-Luc Picard"),
        (Some(""), "alice"),
        (Some(" \t\n"), "alice"),
        (None, "alice"),
    ] {
        let display_names = name
            .map(|name| ("alice".to_owned(), name.to_owned()))
            .into_iter()
            .collect();
        let choices = reviewer_choices(&[], &[ReviewerTarget::user("alice")]);
        assert_eq!(choices[0].label(&display_names), expected);
        assert_eq!(
            reviewer_selection_summary(&choices, &[0], &display_names),
            expected
        );
        assert_eq!(selection_from_indexes(&choices, &[0]).users, ["alice"]);
    }
}

#[test]
fn full_names_do_not_change_reviewer_identity_or_team_labels() {
    let display_names = BTreeMap::from([
        ("alice".to_owned(), "Alex Example".to_owned()),
        ("bob".to_owned(), "Alex Example".to_owned()),
        ("ExampleOrg/platform".to_owned(), "Not a user".to_owned()),
    ]);
    let choices = reviewer_choices(
        &[],
        &[
            ReviewerTarget::user("alice"),
            ReviewerTarget::user("bob"),
            ReviewerTarget::team("ExampleOrg/platform", "platform"),
        ],
    );
    assert_eq!(choices[0].label(&display_names), "Alex Example");
    assert_eq!(choices[1].label(&display_names), "Alex Example");
    assert_eq!(choices[2].label(&display_names), "ExampleOrg/platform");
    assert_eq!(
        reviewer_selection_summary(&choices, &[1, 2], &display_names),
        "Alex Example, ExampleOrg/platform",
    );
    assert_eq!(
        selection_from_indexes(&choices, &[1, 2]),
        ReviewerSelection::new(["bob"], ["platform"]),
    );
    assert_eq!(
        reviewer_selection_summary(&choices, &[], &display_names),
        "none"
    );
}

#[test]
fn reviewer_picker_preserves_reasons_with_full_names() {
    let choices = reviewer_choices(
        &[ReviewerCandidate::new(
            ReviewerTarget::user("alice"),
            vec!["matched 2 files".to_owned(), "already approved".to_owned()],
        )],
        &[],
    );
    let display_names = BTreeMap::from([("alice".to_owned(), "Alice Example".to_owned())]);
    assert_eq!(
        choices[0].label(&display_names),
        format!("{:<24} {REVIEWER_HINT_STYLE}matched 2 files; already approved{REVIEWER_HINT_RESET_STYLE}", "Alice Example"),
    );
}
