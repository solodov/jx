use super::*;

#[test]
fn pull_request_label_chips_use_readable_pastels_and_restore_row_style() {
    // Verifies: saturated and neutral colors become pastel chips, never raw GitHub backgrounds.
    for (source, active_rgb, muted_rgb) in [
        ("d93f0b", [240, 200, 184], [244, 224, 214]),
        ("5319e7", [207, 191, 239], [228, 219, 241]),
        ("1d76db", [193, 214, 236], [222, 231, 239]),
        ("0e8a16", [190, 219, 187], [220, 233, 216]),
        ("fbca04", [249, 235, 183], [248, 241, 213]),
        ("000000", [186, 185, 182], [218, 216, 213]),
        ("ffffff", [250, 248, 245], [249, 247, 244]),
    ] {
        let labels = [PullRequestLabel {
            name: "area: \tbackend".to_owned(),
            color: source.to_owned(),
        }];
        for (chips, [red, green, blue], text, restore) in [
            (
                pull_request_label_chips(&labels, true, false),
                active_rgb,
                "52;49;46",
                "",
            ),
            (
                pull_request_label_chips(&labels, true, true),
                muted_rgb,
                "98;93;86",
                DRAFT_ROW_STYLE,
            ),
            (
                muted_pull_request_label_chips(&labels, true),
                muted_rgb,
                "98;93;86",
                "",
            ),
        ] {
            assert_eq!(chips, [format!(
                "\x1b[22m\x1b[48;2;{red};{green};{blue}m\x1b[38;2;{text}m area:backend \x1b[0m{restore}"
            )], "source: {source}");
        }
    }
}

#[test]
fn pull_request_label_text_meets_srgb_contrast_threshold() {
    // Verifies: all palette variants remain readable across the RGB cube, including black
    // as the darkest possible blend, rather than relying on a perceived-brightness cutoff.
    let channels = [0, 32, 64, 96, 128, 160, 192, 224, 255];
    for red in channels {
        for green in channels {
            for blue in channels {
                let labels = [PullRequestLabel {
                    name: "label".to_owned(),
                    color: format!("{red:02x}{green:02x}{blue:02x}"),
                }];
                for chips in [
                    pull_request_label_chips(&labels, true, false),
                    pull_request_label_chips(&labels, true, true),
                    muted_pull_request_label_chips(&labels, true),
                ] {
                    assert_label_chip_contrast(&chips[0]);
                }
            }
        }
    }
}

#[test]
fn pull_request_label_chips_fall_back_to_readable_neutral_colors() {
    // Verifies: malformed color metadata never produces malformed ANSI or low-contrast chips.
    for source in ["", "invalid", "123", "1234567", "gggggg", "ééé"] {
        let labels = [PullRequestLabel {
            name: "label".to_owned(),
            color: source.to_owned(),
        }];
        for (chips, expected_background) in [
            (
                pull_request_label_chips(&labels, true, false),
                [221, 221, 221],
            ),
            (
                pull_request_label_chips(&labels, true, true),
                [232, 228, 222],
            ),
            (
                muted_pull_request_label_chips(&labels, true),
                [232, 228, 222],
            ),
        ] {
            assert_eq!(label_chip_rgb(&chips[0], "\x1b[48;2;"), expected_background);
            assert_label_chip_contrast(&chips[0]);
        }
    }
}

#[test]
fn pull_request_label_chips_accept_normalized_hex_colors() {
    // Verifies: optional hash prefixes, whitespace, and case keep the same palette.
    let label = PullRequestLabel {
        name: "label".to_owned(),
        color: "5319e7".to_owned(),
    };
    let expected = pull_request_label_chips(std::slice::from_ref(&label), true, false);
    for source in ["5319E7", "#5319e7", "  #5319E7  "] {
        let labels = [PullRequestLabel {
            color: source.to_owned(),
            ..label.clone()
        }];
        assert_eq!(pull_request_label_chips(&labels, true, false), expected);
    }
}

#[test]
fn plain_pull_request_labels_remain_unstyled() {
    // Verifies: disabling color preserves compact bracketed labels in every lifecycle state.
    let labels = [PullRequestLabel {
        name: "area: \tbackend".to_owned(),
        color: "5319e7".to_owned(),
    }];
    for chips in [
        pull_request_label_chips(&labels, false, false),
        pull_request_label_chips(&labels, false, true),
        muted_pull_request_label_chips(&labels, false),
    ] {
        assert_eq!(chips, ["[area:backend]"]);
    }
    assert_eq!(pull_request_label_separator(false), " ");
    assert_eq!(pull_request_label_separator(true), "");
}

fn assert_label_chip_contrast(chip: &str) {
    assert!(
        chip.starts_with("\x1b[22m"),
        "chip must clear inherited intensity: {chip:?}"
    );
    let background = label_relative_luminance(label_chip_rgb(chip, "\x1b[48;2;"));
    let text = label_relative_luminance(label_chip_rgb(chip, "\x1b[38;2;"));
    let contrast = (background.max(text) + 0.05) / (background.min(text) + 0.05);
    assert!(
        contrast >= 4.5,
        "label contrast {contrast:.2}:1 is below 4.5:1: {chip:?}"
    );
}

fn label_chip_rgb(chip: &str, prefix: &str) -> [u8; 3] {
    let (_, channels) = chip.split_once(prefix).expect("chip has an RGB color");
    let (channels, _) = channels
        .split_once('m')
        .expect("color ends with SGR terminator");
    channels
        .split(';')
        .map(|channel| channel.parse::<u8>().expect("color channel is a byte"))
        .collect::<Vec<_>>()
        .try_into()
        .expect("color has three channels")
}

fn label_relative_luminance(rgb: [u8; 3]) -> f64 {
    let [red, green, blue] = rgb.map(|channel| {
        let srgb = f64::from(channel) / 255.0;
        if srgb <= 0.04045 {
            srgb / 12.92
        } else {
            ((srgb + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

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
            "├ ◯ #11     Child 1",
            "│ └ ◯ #14     Child 11",
            "└ ◯ #12     Child 2",
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
        description: "Add stack trunk move".to_owned(),
        change_lines: vec!["M README.md".to_owned(), "M src/commands.rs".to_owned()],
        extra_lines: Vec::new(),
    };

    assert_eq!(
            render_workspace_status_with_width(&status, 80),
            "Working copy  (@) : kvxvwztp b9e8f888\nParent commit (@-): xskrmynn 6257dd5a main | parent\n\nAdd stack trunk move\n\nM README.md\nM src/commands.rs\n"
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
fn workspace_status_renderer_hides_stack_context_comment_markers() {
    // Verifies: local markdown rendering does not expose sync-only PR stack anchors.
    let status = WorkspaceStatus {
        commit_lines: vec!["Working copy  (@) : kvxvwztp b9e8f888".to_owned()],
        description: "Title\n\n<!-- jx-stack:start -->\nPull request stack\n\n◯ Root\n└ ◉ Child — this PR\n<!-- jx-stack:end -->".to_owned(),
        change_lines: Vec::new(),
        extra_lines: Vec::new(),
    };

    let rendered = render_workspace_status_with_width(&status, 120);

    assert!(!rendered.contains("jx-stack"), "{rendered:?}");
    assert!(!rendered.contains("<!--"), "{rendered:?}");
    assert!(rendered.contains("Pull request stack"), "{rendered:?}");
}

#[test]
fn pull_request_preview_renders_legacy_stack_context_as_terminal_links() {
    // Verifies: previously stored GitHub tree blocks remain readable until they are resynced.
    let mut plan = preview_plan();
    plan.title = "Child change".to_owned();
    plan.body = "Authored body\n\n<!-- jx-stack:start -->\n### Pull request stack\n\n◯ [#6 Root](https://github.com/example-owner/example-repo/pull/6)\n└ ◉ **[#7 Child **notes**](https://github.com/example-owner/example-repo/pull/7)** — this PR\n&nbsp;&nbsp;└ ◌ [#8 Draft](https://github.com/example-owner/example-repo/pull/8) — draft\n<!-- jx-stack:end -->".to_owned();

    let preview = render_pull_request_preview_for_width(&plan, &workspace_status(), &[], 160);

    assert!(!preview.contains("jx-stack"), "{preview:?}");
    assert!(!preview.contains("]("), "{preview:?}");
    assert!(!preview.contains("&nbsp;"), "{preview:?}");
    assert!(
        preview.contains(&osc8_link(
            "https://github.com/example-owner/example-repo/pull/6",
            "#6 Root",
        )),
        "{preview:?}"
    );
    assert!(
        preview.contains(&osc8_link(
            "https://github.com/example-owner/example-repo/pull/7",
            "#7 Child **notes**",
        )),
        "{preview:?}"
    );
    assert!(preview.contains("    └ ◌ "), "{preview:?}");
}

#[test]
fn pull_request_preview_renders_nested_stack_lists_with_number_only_links() {
    // Verifies: nested lists retain indentation, current/draft emphasis, and literal title punctuation.
    let mut plan = preview_plan();
    plan.title = "Child change".to_owned();
    plan.body = concat!(
        "Authored body\n\n<!-- jx-stack:start -->\n### Pull request stack\n\n",
        "- [#6](https://github.com/example-owner/example-repo/pull/6) · Root\n",
        "  - **[#7](https://github.com/example-owner/example-repo/pull/7) — this PR** · Child — *draft*\n",
        "    - [#8](https://github.com/example-owner/example-repo/pull/8) · ",
        r"\[ids\] \*stars\* \_name\_ \`code\` \<tag\> \&amp\; \\path \| \#42 — café",
        "\n\n<!-- jx-stack:end -->",
    ).to_owned();

    let preview = render_pull_request_preview_for_width(&plan, &workspace_status(), &[], 180);
    let root_link = osc8_link("https://github.com/example-owner/example-repo/pull/6", "#6");
    let child_link = osc8_link("https://github.com/example-owner/example-repo/pull/7", "#7");
    let nested_link = osc8_link("https://github.com/example-owner/example-repo/pull/8", "#8");
    assert!(
        preview.contains(&format!("  - {root_link} · Root")),
        "{preview:?}"
    );
    assert!(preview.contains(&format!("    - {BOLD_STYLE}{child_link} — this PR{RESET_STYLE} · Child — \x1b[3mdraft{RESET_STYLE}")), "{preview:?}");
    assert!(
        preview.contains(&format!(
            "      - {nested_link} · [ids] *stars* _name_ `code` <tag> &amp; \\path | #42 — café"
        )),
        "{preview:?}"
    );
    assert!(!preview.contains("jx-stack"));
    assert!(!preview.contains("]("));
    assert!(!preview.contains("**"));
    assert!(!preview.contains("*draft*"));
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
fn workspace_remove_confirmation_prompt_uses_display_root() {
    // Verifies: The prompt renderer uses the caller's operator-facing path label.
    let workspace = WorkspaceEntry {
        name: "example-fix".to_owned(),
        root: PathBuf::from("/example/home/projects/.work/example-repo/example-fix"),
        is_current: false,
    };

    assert_eq!(
        workspace_remove_confirmation_prompt(
            &workspace,
            "~/projects/.work/example-repo/example-fix"
        ),
        "Delete workspace `example-fix` at ~/projects/.work/example-repo/example-fix?"
    );
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
