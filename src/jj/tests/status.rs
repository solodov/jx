use super::*;

#[test]
fn workspace_status_parser_reuses_jj_commit_and_change_lines() {
    // Verifies: Status parsing preserves jj-rendered commit and file summary lines for reordering.
    let status = workspace_status_from_jj_status(
            "Working copy changes:\nM README.md\nM src/commands.rs\nWorking copy  (@) : kvxvwztp b9e8f888\nParent commit (@-): xskrmynn 6257dd5a main | parent\n",
            "Add status output".to_owned(),
        );

    assert_eq!(
        status,
        WorkspaceStatus {
            commit_lines: vec![
                "Working copy  (@) : kvxvwztp b9e8f888".to_owned(),
                "Parent commit (@-): xskrmynn 6257dd5a main | parent".to_owned(),
            ],
            description: "Add status output".to_owned(),
            change_lines: vec!["M README.md".to_owned(), "M src/commands.rs".to_owned()],
            extra_lines: Vec::new(),
        }
    );
}

#[test]
fn workspace_status_parser_keeps_no_change_summary() {
    // Verifies: Empty jj status output still produces a status line after the description.
    let status = workspace_status_from_jj_status(
            "The working copy has no changes.\nWorking copy  (@) : abcdef12 11112222\nParent commit (@-): 33334444 55556666 main | parent\n",
            "No local changes".to_owned(),
        );

    assert_eq!(
        status.change_lines,
        vec!["The working copy has no changes.".to_owned()]
    );
}

#[test]
fn workspace_status_parser_preserves_jj_colors() {
    // Verifies: Colored jj status lines survive parsing so jx status can render like jj.
    let status = workspace_status_from_jj_status(
            "Working copy changes:\n\x1b[38;5;6mM README.md\x1b[39m\nWorking copy  (@) : \x1b[1m\x1b[38;5;13mk\x1b[38;5;8mvxvwztp\x1b[0m\n",
            "Colored status".to_owned(),
        );

    assert_eq!(
        status.commit_lines,
        vec!["Working copy  (@) : \x1b[1m\x1b[38;5;13mk\x1b[38;5;8mvxvwztp\x1b[0m".to_owned()]
    );
    assert_eq!(
        status.change_lines,
        vec!["\x1b[38;5;6mM README.md\x1b[39m".to_owned()]
    );
}
