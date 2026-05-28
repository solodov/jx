use super::*;

pub(in crate::commands) fn render_check(
    report: &CheckReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_check(formatter, report)
    })
}

pub(in crate::commands) fn write_check(
    formatter: &mut dyn Formatter,
    report: &CheckReport,
) -> io::Result<()> {
    let current_state = if report.workspace.current_is_empty {
        "empty"
    } else {
        "non-empty"
    };
    let can_push = if report.github.can_push {
        "can push"
    } else {
        "cannot push"
    };

    writeln!(formatter, "ready to publish")?;
    writeln!(formatter, "repo: {}", report.repository.github_slug)?;
    writeln!(
        formatter,
        "change: {}, {current_state}",
        report.workspace.current_short_commit_id
    )?;
    write!(formatter, "bookmark: ")?;
    write_bookmark(
        formatter,
        &report.repository.github_url,
        &report.bookmark.branch,
    )?;
    writeln!(
        formatter,
        ", {}",
        bookmark_action_summary(report.bookmark.action)
    )?;
    writeln!(formatter, "github: {}, {can_push}", report.github.login)?;
    writeln!(
        formatter,
        "reviewers: {}",
        report.repository.default_reviewers
    )
}
