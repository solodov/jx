use super::*;

#[test]
fn status_and_publish_share_commit_content_for_plain_and_colored_output() {
    let workspace = TestWorkspace::new();
    workspace.write_file(".jj/repo/config.toml", "[ui]\ncolor='always'\n[colors]\n'diff added'='green'\n'diff modified'='cyan'\n'diff removed'='red'\n'diff renamed'='cyan'\n'diff copied'='green'\n'diff untracked'='magenta'\n");
    let long_path = format!(
        "A {}測試 with spaces.kt",
        "catalogs/long-directory/".repeat(10)
    );
    let mut plan = preview_plan();
    plan.title = "Shared commit title".to_owned();
    plan.body =
        "A longer description with **Markdown** that wraps at narrow terminal widths.".to_owned();
    plan.change_lines = vec![
        "A added.txt".to_owned(),
        "M modified.txt".to_owned(),
        "D deleted.txt".to_owned(),
        "R {old => new}/file.txt".to_owned(),
        "C {source => copy}/file.txt".to_owned(),
        long_path.clone(),
    ];
    let status = WorkspaceStatus {
        commit_lines: vec!["Selected commit: abcdef".to_owned()],
        description: format!("{}\n\n{}", plan.title, plan.body),
        change_lines: plan.change_lines.clone(),
        extra_lines: vec!["Tracked bookmark sync:".to_owned()],
    };
    for color in [false, true] {
        for width in [28, 120] {
            let output = commit_output(color, width);
            let content = render_commit_content(
                &status.description,
                &status.change_lines,
                &workspace.path(),
                output,
            )
            .unwrap();
            let status_output =
                render_workspace_status(&status, &workspace.path(), output).unwrap();
            let preview =
                render_pull_request_preview(&plan, &workspace.path(), &[], output).unwrap();
            assert!(status_output.contains(&format!("\n\n{content}\n\n")));
            assert!(preview.contains(&format!("\n\n{content}\n\n")));
            assert!(
                content.contains(&long_path),
                "full paths must not be truncated or rewrapped"
            );
            assert!(!content.contains("  A "));
            assert!(!preview.contains("Selected commit:"));
            assert!(!preview.contains("Tracked bookmark sync:"));
            if color {
                assert!(content.contains("\x1b[38;5;2mA added.txt\x1b[39m"));
                assert!(content.contains("\x1b[38;5;6mM modified.txt\x1b[39m"));
                assert!(content.contains("\x1b[38;5;1mD deleted.txt\x1b[39m"));
                assert!(content.contains("\x1b[38;5;6mR {old => new}/file.txt\x1b[39m"));
                assert!(content.contains("\x1b[38;5;2mC {source => copy}/file.txt\x1b[39m"));
            } else {
                assert!(content.ends_with(&plan.change_lines.join("\n")));
            }
        }
    }
}

#[test]
fn publish_matches_native_jj_status_file_colors_including_repo_overrides() {
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".jj/repo/config.toml",
        "[ui]\ncolor='never'\n[colors]\n'diff added'={fg='#224466',bold=true}\n",
    );
    workspace.write_file("added.txt", "new file\n");
    let native = JjWorkspace::current_status(&workspace.path(), true).unwrap();
    let mut plan = preview_plan();
    plan.change_lines = vec!["A added.txt".to_owned()];
    let preview =
        render_pull_request_preview(&plan, &workspace.path(), &[], commit_output(true, 80))
            .unwrap();
    let file_line = native
        .change_lines
        .iter()
        .find(|line| line.contains("A added.txt"))
        .unwrap();
    assert!(file_line.contains("\x1b["));
    assert!(
        preview.lines().any(|line| line == file_line),
        "native: {file_line:?}\npreview: {preview:?}"
    );
    let status_output =
        render_workspace_status(&native, &workspace.path(), commit_output(true, 80)).unwrap();
    assert!(status_output.lines().any(|line| line == file_line));
    let plain =
        render_pull_request_preview(&plan, &workspace.path(), &[], commit_output(false, 80))
            .unwrap();
    assert!(plain.lines().any(|line| line == "A added.txt"));
}

#[test]
fn shared_commit_content_preserves_native_ansi_and_status_notes() {
    let workspace = TestWorkspace::new();
    workspace.write_file(
        ".jj/repo/config.toml",
        "[ui]\ncolor='always'\n[colors]\n'diff modified'='cyan'\n'diff untracked'='magenta'\n",
    );
    let native = "\x1b[38;2;12;34;56mA already-colored.txt\x1b[39m";
    let lines = vec![
        native.to_owned(),
        "M plain.txt".to_owned(),
        "Untracked paths:".to_owned(),
        "? untracked.txt".to_owned(),
    ];
    let rendered =
        render_commit_content("", &lines, &workspace.path(), commit_output(true, 80)).unwrap();
    assert!(rendered.starts_with(native));
    assert_eq!(rendered.matches(native).count(), 1);
    assert!(rendered.contains("\x1b[38;5;6mM plain.txt\x1b[39m"));
    assert!(rendered.contains("\nUntracked paths:\n"));
    assert!(rendered.contains("\x1b[38;5;5m? untracked.txt\x1b[39m"));
}

#[test]
fn empty_commit_sections_do_not_add_blank_blocks_or_style_status_messages() {
    let current_dir = Path::new("/no-workspace-needed");
    let output = commit_output(false, 80);
    assert_eq!(
        render_commit_content("", &[], current_dir, output).unwrap(),
        ""
    );
    assert_eq!(
        render_commit_content("Title", &[], current_dir, output).unwrap(),
        "Title"
    );
    for message in [
        "The working copy has no changes.",
        "The selected commit has no changes.",
    ] {
        assert_eq!(
            render_commit_content("Title", &[message.to_owned()], current_dir, output).unwrap(),
            format!("Title\n\n{message}")
        );
    }
    let line = "A name with trailing spaces  ".to_owned();
    assert_eq!(
        render_commit_content("", std::slice::from_ref(&line), current_dir, output).unwrap(),
        line
    );
}
