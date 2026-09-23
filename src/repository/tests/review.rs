use super::*;

#[test]
fn pending_check_visibility_defaults_off_and_faire_rules_can_be_overridden() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let faire = GitHubRepository::parse("https://github.com/Faire/backend").unwrap();
    let other = GitHubRepository::parse("https://github.com/other/repo").unwrap();
    let config = WorkflowConfig::discover(&environment).unwrap();
    assert_eq!(config.repo.review_for(&faire).hide_pending_checks, None);

    workspace.write_file(
        ".config/jx/10-review.toml",
        r#"
[[repo.rules]]
repo = "Faire/*"
[repo.rules.review]
hide_pending_checks = true
"#,
    );
    workspace.write_file(
        ".config/jx/20-review.toml",
        r#"
[[repo.rules]]
repo = "Faire/*"
[repo.rules.review]
ignored_labels = ["noise"]
"#,
    );
    let config = WorkflowConfig::discover(&environment).unwrap();
    assert_eq!(
        config.repo.review_for(&faire).hide_pending_checks,
        Some(true)
    );
    assert_eq!(config.repo.review_for(&other).hide_pending_checks, None);
    assert!(config.repo.review_for(&faire).ignores_label("noise"));

    workspace.write_file(
        ".jx/config.toml",
        r#"
[[repo.rules]]
repo = "Faire/backend"
[repo.rules.review]
hide_pending_checks = false
"#,
    );
    let config = WorkflowConfig::discover(&environment).unwrap();
    assert_eq!(
        config.repo.review_for(&faire).hide_pending_checks,
        Some(false)
    );
    let another_faire_repo = GitHubRepository::parse("https://github.com/Faire/other").unwrap();
    assert_eq!(
        config
            .repo
            .review_for(&another_faire_repo)
            .hide_pending_checks,
        Some(true)
    );
    assert_eq!(
        config.repo.stack_status_for(&faire),
        RepoStackStatusConfig::default()
    );
}

#[test]
fn base_review_layers_preserve_unset_values_and_accept_explicit_false() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let repo = GitHubRepository::parse("https://github.com/owner/repo").unwrap();
    workspace.write_file(
        ".config/jx/10-review.toml",
        "[repo.review]\nhide_pending_checks = true\n",
    );
    workspace.write_file(
        ".config/jx/20-review.toml",
        "[repo.review]\nignored_labels = ['noise']\n",
    );
    let config = WorkflowConfig::discover(&environment).unwrap();
    assert_eq!(
        config.repo.review_for(&repo).hide_pending_checks,
        Some(true)
    );
    workspace.write_file(
        ".jx/config.toml",
        "[repo.review]\nhide_pending_checks = false\n",
    );
    let config = WorkflowConfig::discover(&environment).unwrap();
    assert_eq!(
        config.repo.review_for(&repo).hide_pending_checks,
        Some(false)
    );
}

#[test]
fn pending_check_visibility_requires_a_boolean() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    for value in ["'true'", "1", "[]", "{}"] {
        workspace.write_file(
            ".jx/config.toml",
            &format!("[repo.review]\nhide_pending_checks = {value}\n"),
        );
        let error = WorkflowConfig::discover(&environment)
            .unwrap_err()
            .to_string();
        assert!(error.contains("repo.review.hide_pending_checks"), "{error}");
        assert!(error.contains("must be a boolean"), "{error}");
    }
}
