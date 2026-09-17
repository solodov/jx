use super::*;
use crate::repository::GitHubRepository;

#[test]
fn frame_preserves_osc8_bytes_and_records_targets_without_parsing_display_text() {
    let context = context();
    let link = crate::commands::osc8_link(&context.pr_url, "#12");
    let row = format!("  {link} Chk Rev —    Title");
    let mut frame = PullRequestTableFrame::default();
    frame.push_line("repository header");
    frame.push_pr_line(&row, Some(context.clone()));
    frame.push_line("");
    frame.push_line("  error: first line\nsecond line");
    frame.push_pr_line("branch without PR", None);
    frame.push_pr_line(
        "a clipped row with no visible PR number",
        Some(context.clone()),
    );
    assert_eq!(frame.text, format!("repository header\n{row}\n\n  error: first line\nsecond line\nbranch without PR\na clipped row with no visible PR number\n"));
    assert_eq!(frame.rows.len(), 2);
    assert_eq!(
        frame.rows[0],
        RenderedPrRow {
            line: 1,
            context: context.clone()
        }
    );
    assert_eq!(frame.rows[1], RenderedPrRow { line: 6, context });
}

fn context() -> PrActionContext {
    PrActionContext {
        repository: GitHubRepository {
            owner: "owner".to_owned(),
            name: "repo".to_owned(),
        },
        repository_root: None,
        pr_number: 12,
        pr_url: "https://github.com/owner/repo/pull/12".to_owned(),
        title: "Full original title".to_owned(),
        branch: "topic/branch".to_owned(),
        base_branch: "main".to_owned(),
        head_oid: Some("head".to_owned()),
        local_commit_id: None,
        local_change_id: None,
    }
}
