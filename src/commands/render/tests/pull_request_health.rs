use super::*;

#[test]
fn user_short_names_use_the_first_word_when_available() {
    for (display_name, expected) in [
        ("Alice Example", "Alice"),
        ("Alice", "Alice"),
        ("  Alice\tExample  ", "Alice"),
        ("Élodie\u{a0}Martin", "Élodie"),
        ("Jean-Luc Picard", "Jean-Luc"),
        ("李小龙", "李小龙"),
        ("", "reviewer-login"),
        (" \t\n\u{a0}", "reviewer-login"),
    ] {
        let display_names =
            BTreeMap::from([("reviewer-login".to_owned(), display_name.to_owned())]);
        assert_eq!(
            pull_request_user_short_name("reviewer-login", &display_names),
            expected,
        );
    }
    assert_eq!(
        pull_request_user_short_name("reviewer-login", &BTreeMap::new()),
        "reviewer-login",
    );
}

#[test]
fn reviewer_team_labels_are_not_shortened_or_replaced() {
    let display_names = BTreeMap::from([(
        "team/platform-infra".to_owned(),
        "Platform Infrastructure".to_owned(),
    )]);
    assert_eq!(
        pull_request_user_short_name("team/platform-infra", &display_names),
        "team/platform-infra",
    );
    assert_eq!(
        pull_request_user_short_name("team/platform-infra", &BTreeMap::new()),
        "team/platform-infra",
    );
}

#[test]
fn reviewer_tokens_keep_styles_and_age_with_short_names() {
    let display_names = BTreeMap::from([("reviewer-login".to_owned(), "Alice Example".to_owned())]);
    for (state, style) in [
        (ReviewerTokenState::Requested, BLACK_BOLD_STYLE),
        (ReviewerTokenState::ChangesRequested, RED_BOLD_STYLE),
        (ReviewerTokenState::Commented, ORANGE_STYLE),
        (ReviewerTokenState::Addressed, BLACK_ITALIC_STYLE),
        (ReviewerTokenState::Approved, GREEN_STYLE),
        (ReviewerTokenState::ApprovedWithComments, GREEN_ITALIC_STYLE),
        (
            ReviewerTokenState::MergedApproved,
            MERGED_APPROVED_REVIEWER_STYLE,
        ),
    ] {
        for (age, label) in [(None, "Alice"), (Some("2h".to_owned()), "Alice 2h")] {
            for color in [false, true] {
                let token = pull_request_reviewer_token(
                    "reviewer-login",
                    state,
                    age.clone(),
                    color,
                    &display_names,
                );
                let expected = if color {
                    format!("{style}{label}{RESET_STYLE}")
                } else {
                    label.to_owned()
                };
                assert_eq!(token, expected);
            }
        }
    }
}
