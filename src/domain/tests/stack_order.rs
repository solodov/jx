use super::*;

#[test]
fn oldest_stack_rows_use_the_oldest_descendant_and_keep_dependencies() {
    let snapshot = snapshot(vec![
        node(Some(30), "newer", "main"),
        node(None, "local-child", "local"),
        node(Some(100), "old-child", "old"),
        node(Some(2), "first-child", "first"),
        node(None, "local", "main"),
        node(Some(10), "old", "main"),
        node(Some(20), "published-child", "unpublished-parent"),
        node(Some(50), "first", "main"),
        node(None, "unpublished-parent", "main"),
    ]);

    let rows = snapshot.rows_oldest_first();
    assert_eq!(
        rows.iter()
            .map(|row| (row.node.branch.as_str(), row.depth))
            .collect::<Vec<_>>(),
        vec![
            ("first", 0),
            ("first-child", 1),
            ("old", 0),
            ("old-child", 1),
            ("unpublished-parent", 0),
            ("published-child", 1),
            ("newer", 0),
            ("local", 0),
            ("local-child", 1),
        ]
    );
    for row in rows {
        assert_eq!(row.prefix, if row.depth == 0 { "" } else { "└─ " });
    }
}

#[test]
fn oldest_stack_rows_include_orphans_and_cycles_once() {
    let snapshot = snapshot(vec![
        node(Some(20), "cycle-b", "cycle-a"),
        node(Some(5), "root", "main"),
        node(Some(10), "cycle-a", "cycle-b"),
        node(Some(2), "orphan", "missing"),
    ]);
    assert_eq!(
        snapshot
            .rows_oldest_first()
            .iter()
            .map(|row| row.node.pull_request_number())
            .collect::<Vec<_>>(),
        vec![Some(2), Some(5), Some(10), Some(20)]
    );
}

fn snapshot(nodes: Vec<StackMetadataNode>) -> PullRequestStackSnapshot {
    PullRequestStackSnapshot::from_metadata(
        &StackMetadata {
            nodes,
            ..StackMetadata::default()
        },
        &[],
        &[],
        PullRequestStackSelection::default(),
    )
}

fn node(number: Option<u64>, branch: &str, parent: &str) -> StackMetadataNode {
    StackMetadataNode {
        branch: branch.to_owned(),
        base_branch: parent.to_owned(),
        parent_branch: (parent != "main").then(|| parent.to_owned()),
        pull_request: number,
        parent_pull_request: None,
        title: branch.to_owned(),
        url: None,
        draft: false,
        merged: false,
        work_ids: Vec::new(),
        fixes_work_ids: Vec::new(),
    }
}
