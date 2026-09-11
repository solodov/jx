use super::*;

#[test]
fn generated_lists_preserve_forks_depth_and_component_selection() {
    // Verifies: sibling PRs stay at the same list depth, deep descendants stay nested, and unrelated stacks stay out.
    let mut draft = node(3, "root", "Draft branch");
    draft.draft = true;
    let metadata = StackMetadata {
        nodes: vec![
            node(6, "main", "Unrelated"),
            node(5, "branch-4", "Deep child"),
            node(4, "branch-3", "Nested child"),
            draft,
            node(2, "root", "Sibling"),
            root_node(),
        ],
        ..StackMetadata::default()
    };
    let current = current_pr(false);
    let snapshot = PullRequestStackSnapshot::from_metadata(
        &metadata,
        &[],
        std::slice::from_ref(&current),
        PullRequestStackSelection::pull_request(1),
    )
    .component_for_selection(PullRequestStackSelection::pull_request(1));
    assert_eq!(
        snapshot
            .rows()
            .iter()
            .map(|row| row.depth)
            .collect::<Vec<_>>(),
        [0, 1, 1, 2, 3]
    );

    let body = pull_request_body_with_stack_context(
        "Authored body",
        &metadata,
        &current,
        "https://github.com/owner/repo",
    );
    assert_eq!(
        body,
        concat!(
            "Authored body\n\n<!-- jx-stack:start -->\n### Pull request stack\n\n",
            "- **[#1](https://github.com/owner/repo/pull/1) — this PR** · Root\n",
            "  - [#2](https://github.com/owner/repo/pull/2) · Sibling\n",
            "  - [#3](https://github.com/owner/repo/pull/3) · Draft branch — *draft*\n",
            "    - [#4](https://github.com/owner/repo/pull/4) · Nested child\n",
            "      - [#5](https://github.com/owner/repo/pull/5) · Deep child\n",
            "\n<!-- jx-stack:end -->",
        )
    );
}

#[test]
fn current_drafts_merged_and_unpublished_nodes_have_explicit_states() {
    // Verifies: selection never hides draft state, and status does not depend on terminal glyphs.
    let mut merged = node(2, "root", "Completed");
    merged.merged = true;
    let mut unpublished = node(3, "root", "Future work");
    unpublished.pull_request = None;
    let metadata = StackMetadata {
        nodes: vec![root_node(), merged, unpublished],
        ..StackMetadata::default()
    };
    let body = pull_request_body_with_stack_context(
        "",
        &metadata,
        &current_pr(true),
        "https://github.com/owner/repo",
    );
    assert!(body
        .contains("- **[#1](https://github.com/owner/repo/pull/1) — this PR** · Root — *draft*"));
    assert!(body.contains("  - [#2](https://github.com/owner/repo/pull/2) · Completed — *merged*"));
    assert!(body.contains("  - Future work — *unpublished*"));
    assert!(!body.contains("[#3]"));
}

#[test]
fn titles_are_literal_text_and_cannot_create_markdown_or_extra_rows() {
    // Verifies: title punctuation, HTML, Unicode, and line breaks cannot change the list structure.
    let mut child = node(2, "root", "");
    child.title =
        "[ids] *stars* _name_ `code` <tag> &amp; \\path | #42 — café\n- not a child".to_owned();
    child.url = Some("https://github.com/another/repo/pull/2".to_owned());
    let metadata = StackMetadata {
        nodes: vec![root_node(), child],
        ..StackMetadata::default()
    };
    let body = pull_request_body_with_stack_context(
        "",
        &metadata,
        &current_pr(false),
        "https://github.com/owner/repo",
    );
    assert!(body.contains(concat!(
        "  - [#2](https://github.com/another/repo/pull/2) · ",
        r"\[ids\] \*stars\* \_name\_ \`code\` \<tag\> \&amp\; \\path \| \#42 — café \- not a child",
    )), "{body}");
    assert_eq!(
        body.lines()
            .filter(|line| line.trim_start().starts_with("- "))
            .count(),
        2
    );
}

#[test]
fn existing_generated_blocks_are_replaced_idempotently_without_changing_authored_text() {
    // Verifies: resync upgrades old tree markup only inside the existing managed block.
    let metadata = StackMetadata {
        nodes: vec![root_node(), node(2, "root", "Child")],
        ..StackMetadata::default()
    };
    let old = "## Authored heading\n\nKeep **this** [link](https://example.com).\n\n<!-- jx-stack:start -->\n### Pull request stack\n\n◯ Root\n&nbsp;&nbsp;└ ◌ Child — draft\n<!-- jx-stack:end -->\n\nFooter `unchanged`.";
    let updated = pull_request_body_with_stack_context(
        old,
        &metadata,
        &current_pr(false),
        "https://github.com/owner/repo",
    );
    assert!(updated
        .starts_with("## Authored heading\n\nKeep **this** [link](https://example.com).\n\n"));
    assert!(updated.ends_with("\n\nFooter `unchanged`."));
    assert!(!updated.contains("&nbsp;"));
    assert!(!updated.contains('└'));
    assert_eq!(updated.matches("<!-- jx-stack:start -->").count(), 1);
    assert_eq!(
        pull_request_body_with_stack_context(
            &updated,
            &metadata,
            &current_pr(false),
            "https://github.com/owner/repo"
        ),
        updated
    );

    let singleton = StackMetadata {
        nodes: vec![root_node()],
        ..StackMetadata::default()
    };
    assert_eq!(
        pull_request_body_with_stack_context(
            &updated,
            &singleton,
            &current_pr(false),
            "https://github.com/owner/repo"
        ),
        "## Authored heading\n\nKeep **this** [link](https://example.com).\n\nFooter `unchanged`."
    );
}

fn root_node() -> StackMetadataNode {
    StackMetadataNode {
        branch: "root".to_owned(),
        ..node(1, "main", "Root")
    }
}

fn node(number: u64, base: &str, title: &str) -> StackMetadataNode {
    StackMetadataNode {
        branch: format!("branch-{number}"),
        base_branch: base.to_owned(),
        parent_branch: (base != "main").then(|| base.to_owned()),
        parent_pull_request: None,
        pull_request: Some(number),
        title: title.to_owned(),
        url: None,
        draft: false,
        merged: false,
        work_ids: Vec::new(),
        fixes_work_ids: Vec::new(),
    }
}

fn current_pr(draft: bool) -> PullRequestRecord {
    PullRequestRecord {
        number: 1,
        title: "Root".to_owned(),
        body: None,
        head_branch: "root".to_owned(),
        base_branch: "main".to_owned(),
        html_url: None,
        draft,
        merged: false,
        reviewers: ReviewerSelection::default(),
    }
}
