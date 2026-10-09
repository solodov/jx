use super::*;

#[test]
fn stack_status_orders_oldest_stacks_first_in_every_table_layout() {
    let report = ordered_stack_report();
    for color in [false, true] {
        for layout in [
            PullRequestTableLayout::Flow,
            PullRequestTableLayout::FitTerminal,
        ] {
            let local = render_stack_status(
                &report,
                Path::new("/repo"),
                &BTreeMap::new(),
                color,
                Some(100),
                layout,
                &BTreeMap::new(),
            );
            let global = render_global_stack_status(
                &[GlobalStackStatusEntry::current(
                    PathBuf::from("/repo"),
                    &report,
                )],
                1,
                Path::new("/caller"),
                color,
                Some(100),
                layout,
                &BTreeMap::new(),
            );
            for frame in [local, global] {
                assert_eq!(
                    frame
                        .rows
                        .iter()
                        .map(|row| row.context.pr_number)
                        .collect::<Vec<_>>(),
                    vec![10, 100, 20, 30, 31],
                    "{}",
                    frame.text,
                );
            }
        }
    }
}

#[test]
fn stack_status_json_keeps_the_same_oldest_stack_dependency_order() {
    let report = ordered_stack_report();
    let output = render_stack_status_json(&[GlobalStackStatusEntry::current(
        PathBuf::from("/repo"),
        &report,
    )]);
    let output: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
    assert_eq!(
        output["repositories"][0]["pullRequests"]
            .as_array()
            .expect("PR rows")
            .iter()
            .map(|row| row["number"].as_u64().expect("PR number"))
            .collect::<Vec<_>>(),
        vec![10, 100, 20, 30, 31],
    );
}

#[test]
fn stack_status_help_explains_oldest_stack_order() {
    let help = help_output(["jx", "stack", "status", "--help"]);
    let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(help.contains("Stacks with the oldest PRs appear first, with parents before children"));
}

fn ordered_stack_report() -> PullRequestStackStatusReport {
    let metadata = StackMetadata {
        nodes: vec![
            stack_status_node(31, "new-child", "new", "New child", false),
            stack_status_node(20, "middle", "main", "Middle stack", false),
            stack_status_node(100, "old-child", "old", "Old stack child", false),
            stack_status_node(30, "new", "main", "New stack", false),
            stack_status_node(10, "old", "main", "Old stack", false),
        ],
        ..StackMetadata::default()
    };
    PullRequestStackStatusReport {
        repository: preview_plan().repository,
        snapshot: PullRequestStackSnapshot::from_metadata(
            &metadata,
            &[],
            &[],
            PullRequestStackSelection::default(),
        ),
        statuses: BTreeMap::new(),
        trunk: None,
        review_wait_threshold_seconds: None,
    }
}
