use super::*;

#[test]
fn same_repository_head_keeps_user_scoped_branch_namespace() {
    // Verifies: Same-repository PR heads preserve user-scoped branch namespaces.
    let head = PullRequestHead::same_repository("example-owner", "example-user/ABC-123-00-a1b2c3d");

    assert_eq!(
        head.label(),
        "example-owner:example-user/ABC-123-00-a1b2c3d"
    );
}

#[test]
fn label_application_is_normalized() {
    // Verifies: Label requests trim names, drop empties, and deduplicate entries.
    let labels = LabelApplyResult {
        labels: normalize_names([" bug ", "", "bug", "help wanted"]),
    };

    assert_eq!(
        labels.labels,
        vec!["bug".to_owned(), "help wanted".to_owned()]
    );
}

#[test]
fn reviewer_selection_is_normalized() {
    // Verifies: Reviewer selection trims names, drops empties, and deduplicates entries.
    let selection = ReviewerSelection::new(
        [
            "example-reviewer",
            "",
            "example-reviewer",
            "second-reviewer",
        ],
        ["example-team", " example-team "],
    );

    assert_eq!(
        selection.users,
        vec!["example-reviewer".to_owned(), "second-reviewer".to_owned()]
    );
    assert_eq!(selection.teams, vec!["example-team".to_owned()]);
}

#[test]
fn reviewer_difference_preserves_sorted_left_order() {
    // Verifies: Reviewer set differences preserve the normalized left-hand order.
    let missing = difference(
        &["example-a".to_owned(), "example-b".to_owned()],
        &["example-b".to_owned()],
    );

    assert_eq!(missing, vec!["example-a".to_owned()]);
}

#[test]
fn token_source_build_rejects_missing_token() {
    // Verifies: GitHub client construction rejects a missing token source.
    let environment = RuntimeEnvironment::new("/workspace", []);
    let error = match OctocrabGitHubClient::from_token_source(&TokenSource::Missing, &environment) {
        Ok(_) => panic!("token is required"),
        Err(error) => error,
    };

    assert!(matches!(error, GitHubError::MissingToken));
}

#[test]
fn token_source_reads_discovered_environment_value() {
    // Verifies: Token lookup reads the discovered environment value without storing secrets.
    let environment = RuntimeEnvironment::new(
        "/workspace",
        [("JX_GITHUB_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let token_source = TokenSource::Environment("JX_GITHUB_TOKEN");

    assert_eq!(
        token_source.token(&environment).expect("token reads"),
        Some("placeholder-token".to_owned())
    );
}

#[test]
fn deserializes_minimal_compare_response() {
    // Verifies: Compare deserialization ignores nested fields GitHub may omit.
    let comparison: CompareCommitsResponse = serde_json::from_str(
        r#"
{
  "status": "behind",
  "ahead_by": 0,
  "behind_by": 3,
  "base_commit": { "sha": "1111222233334444" }
}
"#,
    )
    .expect("compare response deserializes");

    assert_eq!(
        map_comparison_status(comparison.status),
        ComparisonStatus::Behind
    );
    assert_eq!(comparison.ahead_by, 0);
    assert_eq!(comparison.behind_by, 3);
}

#[test]
fn maps_compare_status_to_domain_status() {
    // Verifies: GitHub compare status strings map to stable domain states.
    assert_eq!(
        map_comparison_status(CompareCommitsStatus::Ahead),
        ComparisonStatus::Ahead
    );
    assert_eq!(
        map_comparison_status(CompareCommitsStatus::Behind),
        ComparisonStatus::Behind
    );
    assert_eq!(
        map_comparison_status(CompareCommitsStatus::Diverged),
        ComparisonStatus::Diverged
    );
    assert_eq!(
        map_comparison_status(CompareCommitsStatus::Identical),
        ComparisonStatus::Identical
    );
}
