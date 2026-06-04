use super::*;

#[test]
fn pull_request_selection_formats_draft_state_as_color_only() {
    // Verifies: Draft PR choices keep text aligned and signal state with subdued color only.
    let ready = PullRequestRecord {
        number: 42,
        title: "Ready change".to_owned(),
        body: None,
        head_branch: "topic/ready".to_owned(),
        base_branch: "main".to_owned(),
        html_url: None,
        draft: false,
        merged: false,
        reviewers: ReviewerSelection::default(),
    };
    let draft = PullRequestRecord {
        number: 43,
        title: "Work in progress".to_owned(),
        body: None,
        head_branch: "topic/wip".to_owned(),
        base_branch: "main".to_owned(),
        html_url: None,
        draft: true,
        merged: false,
        reviewers: ReviewerSelection::default(),
    };

    assert_eq!(pull_request_choice_label(&ready), "◯ #42     Ready change");
    assert_eq!(
        pull_request_choice_label(&draft),
        "\x1b[2m\x1b[38;2;190;184;176m◌ #43     Work in progress\x1b[0m"
    );
    assert!(!pull_request_choice_label(&draft).contains("draft "));
    assert!(!pull_request_choice_label(&ready).contains("topic/ready"));
}

#[test]
fn pull_request_selection_renders_newest_stack_first_with_dependency_order() {
    // Verifies: PR choices show newer stacks first while preserving parent-before-child order inside each stack.
    let pull_requests = vec![
        pull_request_choice_record(12, "Child 2", "topic/child-2", "topic/root", false),
        pull_request_choice_record(1, "Draft root", "topic/draft-root", "main", true),
        pull_request_choice_record(14, "Child 11", "topic/child-11", "topic/child-1", false),
        pull_request_choice_record(10, "Root", "topic/root", "main", false),
        pull_request_choice_record(2, "Other root", "topic/other", "main", false),
        pull_request_choice_record(11, "Child 1", "topic/child-1", "topic/root", false),
    ];

    let local_branches = pull_requests
        .iter()
        .map(|pull_request| pull_request.head_branch.clone())
        .collect::<Vec<_>>();
    let snapshot = PullRequestStackSnapshot::from_metadata(
        &StackMetadata::default(),
        &local_branches,
        &pull_requests,
        PullRequestStackSelection::default(),
    );
    let rows = pull_request_choice_rows(&snapshot);

    assert_eq!(
        rows.iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "◯ #10     Root",
            "├─ ◯ #11     Child 1",
            "│  └─ ◯ #14     Child 11",
            "└─ ◯ #12     Child 2",
            "◯ #2      Other root",
            "\x1b[2m\x1b[38;2;190;184;176m◌ #1      Draft root\x1b[0m",
        ]
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.pull_request.number)
            .collect::<Vec<_>>(),
        vec![10, 11, 14, 12, 2, 1]
    );
}

#[test]
fn reviewer_selection_formats_cli_reviewers_first_and_summarizes_reasons() {
    // Verifies: Reviewer choices show explicit reviewers first and keep ownership hints concise.
    let candidates = vec![
        ReviewerCandidate::new(
            ReviewerTarget::user("example-reviewer"),
            vec!["global".to_owned()],
        ),
        ReviewerCandidate::new(
            ReviewerTarget::team("ExampleOrg/frontend", "frontend"),
            vec![
                "src/** matched 2 files".to_owned(),
                "tests/** matched 1 file".to_owned(),
            ],
        ),
    ];
    let choices = reviewer_choices(
        &candidates,
        &[
            ReviewerTarget::user("cli-reviewer"),
            ReviewerTarget::team("ExampleOrg/frontend", "frontend"),
        ],
    );

    assert_eq!(choices[0].target.display_name(), "cli-reviewer");
    assert!(choices[0].checked);
    assert_eq!(choices[0].label(), "cli-reviewer");
    assert_eq!(choices[1].target.display_name(), "ExampleOrg/frontend");
    assert!(choices[1].checked);
    assert_eq!(
        choices[1].label(),
        "ExampleOrg/frontend      \x1b[38;5;244mmatched 3 files\x1b[0m"
    );
    assert_eq!(
        choices[2].label(),
        "example-reviewer         \x1b[38;5;244mglobal\x1b[0m"
    );
    assert!(!choices[2].checked);
    assert_eq!(
        selection_from_indexes(&choices, &[1]),
        ReviewerSelection {
            users: Vec::new(),
            teams: vec!["frontend".to_owned()],
        }
    );
}

#[test]
fn workspace_status_renderer_orders_commit_description_and_jj_changes() {
    // Verifies: Status rendering puts jj commit lines first, then description, then jj file lines.
    let status = WorkspaceStatus {
        commit_lines: vec![
            "Working copy  (@) : kvxvwztp b9e8f888".to_owned(),
            "Parent commit (@-): xskrmynn 6257dd5a main | parent".to_owned(),
        ],
        description: "Add rebase-on-trunk command".to_owned(),
        change_lines: vec!["M README.md".to_owned(), "M src/commands.rs".to_owned()],
        extra_lines: Vec::new(),
    };

    assert_eq!(
            render_workspace_status_with_width(&status, 80),
            "Working copy  (@) : kvxvwztp b9e8f888\nParent commit (@-): xskrmynn 6257dd5a main | parent\n\nAdd rebase-on-trunk command\n\nM README.md\nM src/commands.rs\n"
        );
}

#[test]
fn workspace_status_renderer_renders_markdown_description_without_preview_indent() {
    // Verifies: jx status uses the shared PR markdown renderer without adding preview indentation.
    let status = WorkspaceStatus {
        commit_lines: vec!["Working copy  (@) : kvxvwztp b9e8f888".to_owned()],
        description: "This is **important** markdown with enough words to wrap.".to_owned(),
        change_lines: Vec::new(),
        extra_lines: Vec::new(),
    };

    let rendered = render_workspace_status_with_width(&status, 28);
    let description_block = rendered
        .split("\n\n")
        .nth(1)
        .expect("status renders description after commit lines");

    assert!(description_block.lines().count() > 1, "{rendered:?}");
    assert!(description_block.contains("important"), "{rendered:?}");
    assert!(!description_block.contains("**important**"), "{rendered:?}");
    assert!(
        description_block
            .lines()
            .all(|line| !line.starts_with("  ")),
        "{rendered:?}"
    );
}

#[test]
fn pull_request_preview_focuses_on_publish_state_and_changed_files() {
    // Verifies: PR preview omits commit headers while keeping description, planned changed files, and metadata.
    let mut plan = preview_plan();
    plan.labels = vec!["bug".to_owned(), "help wanted".to_owned()];
    plan.base_pull_request = Some(existing_pull_request(false));
    plan.changed_files = vec!["src/main.rs".to_owned(), "src/lib.rs".to_owned()];
    plan.change_lines = vec!["M src/main.rs".to_owned(), "A src/lib.rs".to_owned()];
    let mut status = workspace_status();
    status.change_lines = vec!["M stale-current-workspace-file.rs".to_owned()];
    let prepare_effects = [PullRequestEventEffect {
        event: crate::repository::RepoEvent::PullRequestPrepare,
        handler_id: Some("prepend-task".to_owned()),
        kind: PullRequestEventEffectKind::UpdatedTitle {
            title: "example change".to_owned(),
        },
    }];

    let preview = render_pull_request_preview(&plan, &status, &prepare_effects);

    assert_eq!(
        preview,
        format!(
            "Creating: {} → {}\nEvent[prepend-task]: Added task ID to the title\n\n  example change\n\n  M src/main.rs\n  A src/lib.rs\n\nLabels: bug, help wanted\n",
            example_bookmark_link("example-user/02-zzzzzzzz"),
            example_pull_request_link(7),
        )
    );
    let colored = render_pull_request_preview_with_style(&plan, &status, &prepare_effects, true);
    assert!(colored.contains("\x1b[38;5;6mM src/main.rs\x1b[39m"));
    assert_eq!(pull_request_confirmation_prompt(&plan), "Create?");
    plan.draft = true;
    assert_eq!(pull_request_confirmation_prompt(&plan), "Create draft?");
    plan.draft = false;
    plan.existing_pull_request = Some(existing_pull_request(false));
    assert_eq!(pull_request_confirmation_prompt(&plan), "Update?");
    plan.existing_pull_request = Some(existing_pull_request(true));
    assert_eq!(
        pull_request_confirmation_prompt(&plan),
        "Update and mark ready?"
    );
    plan.draft = true;
    plan.existing_pull_request = Some(existing_pull_request(false));
    assert_eq!(
        pull_request_confirmation_prompt(&plan),
        "Update and mark draft?"
    );
    plan.existing_pull_request = Some(existing_pull_request(true));
    assert_eq!(pull_request_confirmation_prompt(&plan), "Update draft?");
}

#[test]
fn pull_request_preview_wraps_description_inside_content_indent() {
    // Verifies: Indented PR content still reserves indentation width before markdown wrapping.
    let mut plan = preview_plan();
    plan.title = "Example preview title".to_owned();
    plan.body = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda".to_owned();
    let preview = render_pull_request_preview_for_width(&plan, &workspace_status(), &[], 28);

    let indented_lines = preview
        .lines()
        .filter(|line| line.starts_with("  ") && !line.trim().is_empty())
        .collect::<Vec<_>>();

    assert!(indented_lines.len() > 3, "{preview:?}");
    for line in indented_lines {
        assert!(
            line.len() <= 28,
            "line exceeded preview width: {line:?}\n{preview}"
        );
    }
}
