use super::*;

#[test]
fn stack_status_alias_column_is_left_aligned_capped_and_matches_pr_styling() {
    let report = alias_report();
    let aliases = BTreeMap::from([
        ("topic/101".to_owned(), "q".to_owned()),
        ("topic/102".to_owned(), "kn".to_owned()),
        ("topic/103".to_owned(), "pty".to_owned()),
        ("topic/104".to_owned(), "longer".to_owned()),
    ]);
    for color in [false, true] {
        for layout in [
            PullRequestTableLayout::Flow,
            PullRequestTableLayout::FitTerminal,
        ] {
            let local = render_stack_status(
                &report,
                Path::new("/repo"),
                &aliases,
                color,
                Some(100),
                layout,
                &BTreeMap::new(),
            );
            let mut entry = GlobalStackStatusEntry::current(PathBuf::from("/repo"), &report);
            entry.local_aliases = aliases.clone();
            let global = render_global_stack_status(
                &[entry],
                1,
                Path::new("/caller"),
                color,
                Some(100),
                layout,
                &BTreeMap::new(),
            );
            for (frame, indent) in [(local, ""), (global, "  ")] {
                assert_eq!(frame.rows.len(), 5);
                for row in &frame.rows {
                    let number = row.context.pr_number;
                    let text = frame.text.lines().nth(row.line).unwrap();
                    let label = match number {
                        101 => "q  ",
                        102 => "kn ",
                        103 => "pty",
                        104 => "…  ",
                        105 => "   ",
                        _ => unreachable!(),
                    };
                    let number_label = number.to_string();
                    let (alias, pr) = if color && number == 103 {
                        (
                            format!("{GREEN_STYLE}{label}{RESET_STYLE}"),
                            format!("{GREEN_STYLE}{number_label}{RESET_STYLE}"),
                        )
                    } else {
                        (label.to_owned(), number_label)
                    };
                    let row_style = if color && number == 102 {
                        DRAFT_ROW_STYLE
                    } else {
                        ""
                    };
                    assert!(
                        text.starts_with(&format!(
                            "{row_style}{indent}{alias} {}",
                            osc8_link(&row.context.pr_url, &pr),
                        )),
                        "{text:?}"
                    );
                    assert!(!text.contains("longer"), "{text:?}");
                    assert!(!text.contains("\x1b[48;"), "{text:?}");
                    assert!(rendered_visible_width(text) <= 100, "{text:?}");
                }
                if !color {
                    assert!(frame.text.contains(&format!("{indent}JJ  PR")));
                }
            }
        }
    }
}

#[test]
fn aliases_do_not_change_stack_status_json() {
    let report = alias_report();
    let original = GlobalStackStatusEntry::current(PathBuf::from("/repo"), &report);
    let mut annotated = original.clone();
    annotated
        .local_aliases
        .insert("topic/101".to_owned(), "qq".to_owned());

    assert_eq!(
        render_stack_status_json(&[original]),
        render_stack_status_json(&[annotated]),
    );
}

#[test]
fn current_stack_status_loads_local_aliases_only_for_human_output() {
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        "[remote \"origin\"]\n    url = https://github.com/example-owner/example-repo.git\n",
    );
    write_stack_metadata(
        &workspace.path(),
        &StackMetadata {
            nodes: vec![alias_node(101, false, false)],
            ..StackMetadata::default()
        },
    )
    .unwrap();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices {
        local_bookmark_aliases_by_root: BTreeMap::from([(
            workspace.path(),
            BTreeMap::from([("topic/101".to_owned(), "qq".to_owned())]),
        )]),
        ..FakeServices::default()
    };

    let human =
        run_with_args_and_services(["jx", "stack", "status"], &environment, &services).unwrap();
    assert!(human.stdout.lines().any(|line| line.starts_with("qq  ")));
    assert_eq!(
        services.local_bookmark_alias_requests.borrow().as_slice(),
        [(workspace.path(), vec!["topic/101".to_owned()])],
    );
    services.local_bookmark_alias_requests.borrow_mut().clear();

    let json = run_with_args_and_services(
        ["jx", "stack", "status", "--format", "json"],
        &environment,
        &services,
    )
    .unwrap();
    assert!(services.local_bookmark_alias_requests.borrow().is_empty());
    assert!(!json.stdout.contains("qq"));
    let metadata = fs::read_to_string(workspace.path().join(".jx/stack.toml")).unwrap();
    assert!(!metadata.contains("alias"));
}

#[test]
fn all_repository_status_uses_each_repositorys_own_aliases() {
    let workspace = TestWorkspace::new();
    workspace.write_home_file(
        ".config/jx/config.toml",
        "[[layout.rules]]\nsource = 'github'\nowner = 'example-owner'\nroot = '~/projects'\npath = '{repo}'\n",
    );
    let mut services = FakeServices::default();
    let mut roots = Vec::new();
    for (name, number, alias) in [("api-alpha", 201, "q"), ("api-beta", 202, "kn")] {
        let root = workspace.create_jj_workspace(&format!("projects/{name}"));
        TestWorkspace::write_git_config_at(
            &root,
            &format!(
                "[remote \"origin\"]\n    url = https://github.com/example-owner/{name}.git\n"
            ),
        );
        services.authored_open_pull_requests_by_repository.insert(
            format!("example-owner/{name}"),
            vec![pull_request_choice_record(
                number,
                name,
                "topic/shared",
                "main",
                false,
            )],
        );
        services.local_bookmark_aliases_by_root.insert(
            root.clone(),
            BTreeMap::from([("topic/shared".to_owned(), alias.to_owned())]),
        );
        roots.push(root);
    }
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());

    let output =
        run_with_args_and_services(["jx", "stack", "status", "--all"], &environment, &services)
            .unwrap()
            .stdout;

    for (name, number, alias) in [("api-alpha", 201, "q  "), ("api-beta", 202, "kn ")] {
        let pr = osc8_link(
            &format!("https://github.com/example-owner/{name}/pull/{number}"),
            &number.to_string(),
        );
        assert!(
            output
                .lines()
                .any(|line| line.starts_with(&format!("  {alias} {pr}"))),
            "{output:?}"
        );
    }
    assert_eq!(services.local_bookmark_alias_requests.borrow().len(), 2);
    for root in roots {
        assert!(services
            .local_bookmark_alias_requests
            .borrow()
            .iter()
            .any(|(requested_root, bookmarks)| requested_root == &root
                && bookmarks == &["topic/shared".to_owned()],));
    }
}

fn alias_report() -> PullRequestStackStatusReport {
    let nodes = [
        (101, false, false),
        (102, true, false),
        (103, false, true),
        (104, false, false),
        (105, false, false),
    ]
    .into_iter()
    .map(|(number, draft, merged)| alias_node(number, draft, merged))
    .collect();
    PullRequestStackStatusReport {
        repository: preview_plan().repository,
        snapshot: PullRequestStackSnapshot::from_metadata(
            &StackMetadata {
                nodes,
                ..StackMetadata::default()
            },
            &[],
            &[],
            PullRequestStackSelection::default(),
        ),
        statuses: BTreeMap::new(),
        trunk: None,
        review_wait_threshold_seconds: None,
    }
}

fn alias_node(number: u64, draft: bool, merged: bool) -> StackMetadataNode {
    StackMetadataNode {
        branch: format!("topic/{number}"),
        base_branch: "main".to_owned(),
        parent_branch: None,
        pull_request: Some(number),
        parent_pull_request: None,
        title: "Short title".to_owned(),
        url: None,
        draft,
        merged,
        work_ids: Vec::new(),
        fixes_work_ids: Vec::new(),
    }
}
