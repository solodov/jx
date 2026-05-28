use super::*;

#[test]
fn external_diff_args_append_jj_paths_after_configured_and_extra_args() {
    // Verifies: External tools receive configured/extra args before jj's left/right trees.
    let tool = ExternalDiffTool {
        command: "difft".to_owned(),
        args: vec![
            "--display=side-by-side".to_owned(),
            "--display=inline".to_owned(),
        ],
    };

    assert_eq!(
        external_diff_args(&tool),
        vec![
            "--display=side-by-side",
            "--display=inline",
            "$left",
            "$right"
        ]
    );
}

#[test]
fn no_tests_filter_keeps_source_paths_and_excludes_common_test_paths() {
    // Verifies: Diff test exclusion preserves source paths while dropping common test conventions.
    let paths = [
        "src/main.rs",
        "tests/cli.rs",
        "pkg/test/helper.js",
        "web/__tests__/view.tsx",
        "cmd/foo_test.go",
        "scripts/test_data.py",
        "scripts/test_runner.py",
        "frontend/button.test.tsx",
        "frontend/button.spec.tsx",
        "src/FooTest.java",
        "src/FooTests.kt",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();

    assert_eq!(diff_paths_without_tests(&paths), vec!["src/main.rs"]);
}
