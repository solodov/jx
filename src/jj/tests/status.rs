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
fn jj_status_failure_summary_includes_captured_stderr() {
    // Verifies: quiet internal jj status calls suppress successful stderr but preserve failed diagnostics.
    assert_eq!(
        jj_status_failure_summary("exit code 1".to_owned(), b"snapshot failed\n"),
        "exit code 1: snapshot failed"
    );
    assert_eq!(
        jj_status_failure_summary("exit code 1".to_owned(), b"\n"),
        "exit code 1"
    );
}

#[test]
fn selected_revision_status_accepts_bookmark_fragments_without_moving_current() {
    // Verifies: status -r uses the same local bookmark fragment selection as stack publishing.
    let fixture = TestWorkspace::new("selected-status-bookmark-fragment");
    let settings = user_settings().expect("settings");
    let (workspace, repo, selected, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let selected = write_child_with_files(
            tx.repo_mut(),
            &root,
            "selected ancestor\n\nBody",
            &[("README.md", b"readme\n".as_slice())],
        )
        .await;
        let current = write_child_with_files(
            tx.repo_mut(),
            &selected,
            "current change",
            &[("src/main.rs", b"fn main() {}\n".as_slice())],
        )
        .await;

        set_local_bookmark(tx.repo_mut(), "example-user/selected", selected.id());
        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        let repo = tx
            .commit("arrange selected status workspace")
            .await
            .expect("commit");
        (workspace, repo, selected, current)
    });
    let subject = JjWorkspace { workspace, repo };

    let status = subject
        .status_for_revision("selected")
        .expect("selected status loads");

    assert_eq!(status.description, "selected ancestor\n\nBody");
    assert_eq!(status.change_lines, vec!["A README.md".to_owned()]);
    assert!(
        status.commit_lines[0].contains(&short_commit_id(selected.id())),
        "{:?}",
        status.commit_lines
    );
    assert!(status.commit_lines[0].contains("example-user/selected"));
    assert!(status.commit_lines[0].contains("selected ancestor"));
    assert_eq!(
        subject.current_commit().expect("current commit").id(),
        current.id()
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
