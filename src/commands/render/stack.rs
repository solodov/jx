use super::*;

const PULL_REQUEST_DRAFT_STYLE: &str = "\x1b[2m\x1b[38;2;190;184;176m";
const PULL_REQUEST_DRAFT_RESET_STYLE: &str = "\x1b[0m";

/// Formats a stack row for CLI output, applying the shared draft style when color is enabled.
pub(in crate::commands) fn render_stack_row_label(
    row: PullRequestStackRow<'_>,
    color: bool,
) -> String {
    let draft = row.node.draft;
    let label = row.plain_label();
    if color && draft {
        format!("{PULL_REQUEST_DRAFT_STYLE}{label}{PULL_REQUEST_DRAFT_RESET_STYLE}")
    } else {
        label
    }
}
