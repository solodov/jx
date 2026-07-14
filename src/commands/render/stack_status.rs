use super::*;

const STACK_STATUS_PR_WIDTH: usize = PULL_REQUEST_STATUS_PR_WIDTH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct GlobalStackStatusEntry {
    pub(in crate::commands) key: Option<String>,
    pub(in crate::commands) root: PathBuf,
    pub(in crate::commands) display_root: String,
    pub(in crate::commands) repository: Option<GitHubRepository>,
    pub(in crate::commands) result: Result<PullRequestStackStatusReport, String>,
}

impl GlobalStackStatusEntry {
    pub(in crate::commands) fn current(
        root: PathBuf,
        report: &PullRequestStackStatusReport,
    ) -> Self {
        Self {
            key: None,
            display_root: root.display().to_string(),
            root,
            repository: Some(GitHubRepository {
                owner: report.repository.github_slug_owner().to_owned(),
                name: report.repository.github_slug_name().to_owned(),
            }),
            result: Ok(report.clone()),
        }
    }
}

pub(in crate::commands) fn render_stack_status(
    report: &PullRequestStackStatusReport,
    current_dir: &Path,
    color: bool,
    terminal_width: Option<usize>,
    display_names: &BTreeMap<String, String>,
) -> Result<String, JjError> {
    Ok(render_plain_output(|formatter| {
        writeln!(
            formatter,
            "{}",
            stack_status_repository_header(
                &report.repository.github_slug,
                Some(&report.repository.github_url),
                &current_dir.display().to_string(),
                Some(report),
                color,
            )
        )?;
        writeln!(formatter)?;
        let _ =
            write_stack_status_report(formatter, report, color, 0, terminal_width, display_names)?;
        Ok(())
    }))
}

pub(in crate::commands) fn render_global_stack_status(
    entries: &[GlobalStackStatusEntry],
    total_repositories: usize,
    current_dir: &Path,
    color: bool,
    terminal_width: Option<usize>,
    display_names: &BTreeMap<String, String>,
) -> Result<String, JjError> {
    let _ = current_dir;
    Ok(render_plain_output(|formatter| {
        let _ = total_repositories;

        for (index, entry) in entries.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            let (label, url) = entry
                .repository
                .as_ref()
                .map(|repository| (repository.slug(), Some(repository.https_url())))
                .unwrap_or_else(|| {
                    (
                        entry.key.clone().unwrap_or_else(|| "repository".to_owned()),
                        None,
                    )
                });
            let report = entry.result.as_ref().ok();
            writeln!(
                formatter,
                "{}",
                stack_status_repository_header(
                    &label,
                    url.as_deref(),
                    &entry.display_root,
                    report,
                    color
                )
            )?;
            match &entry.result {
                Ok(report) => {
                    writeln!(formatter)?;
                    let _ = write_stack_status_report(
                        formatter,
                        report,
                        color,
                        2,
                        terminal_width,
                        display_names,
                    )?;
                }
                Err(error) => writeln!(formatter, "  error: {error}")?,
            }
        }

        Ok(())
    }))
}

pub(in crate::commands) fn render_stack_status_json(entries: &[GlobalStackStatusEntry]) -> String {
    let output = StackStatusJson {
        command: "stack-status",
        version: 1,
        repositories: entries
            .iter()
            .map(StackStatusRepositoryJson::from)
            .collect(),
    };
    let mut rendered = serde_json::to_string_pretty(&output)
        .expect("stack status JSON contains only serializable values");
    rendered.push('\n');
    rendered
}

fn write_stack_status_report(
    formatter: &mut dyn Formatter,
    report: &PullRequestStackStatusReport,
    color: bool,
    indent: usize,
    terminal_width: Option<usize>,
    display_names: &BTreeMap<String, String>,
) -> io::Result<bool> {
    let indent = " ".repeat(indent);
    let visible_snapshot = visible_stack_status_snapshot(report);
    if visible_snapshot.nodes.is_empty() {
        writeln!(formatter, "{indent}No stack state")?;
        return Ok(false);
    }

    let rows = stack_status_table_rows(report, &visible_snapshot, color, display_names);
    let pr_width = rows
        .iter()
        .map(|row| row.pr_visible_width)
        .max()
        .unwrap_or(0)
        .max(STACK_STATUS_PR_WIDTH);
    writeln!(
        formatter,
        "{indent}{:<pr_width$}  Chk  Rev  {:<lag_width$}  Title",
        "PR",
        "Lag",
        pr_width = pr_width,
        lag_width = REVIEW_LAG_WIDTH,
    )?;
    for row in rows {
        let pr_padding = " ".repeat(pr_width.saturating_sub(row.pr_visible_width));
        let review_lag = render_review_lag_cell(
            &row.review_lag,
            color && !row.closed,
            row.style,
            row.merged,
            row.draft,
        );
        let line = format!(
            "{indent}{}{}  {}    {}    {}  {}",
            row.pr_cell, pr_padding, row.check_symbol, row.review_symbol, review_lag, row.title,
        );
        let line = ellipsize_rendered_line(&line, terminal_width);
        writeln!(formatter, "{}", style_stack_status_row(line, row.style))?;
    }
    Ok(true)
}

struct StackStatusTableRow {
    pr_cell: String,
    pr_visible_width: usize,
    check_symbol: String,
    review_symbol: String,
    review_lag: ReviewLagCell,
    title: String,
    draft: bool,
    merged: bool,
    closed: bool,
    style: &'static str,
}

fn stack_status_table_rows(
    report: &PullRequestStackStatusReport,
    snapshot: &PullRequestStackSnapshot,
    color: bool,
    display_names: &BTreeMap<String, String>,
) -> Vec<StackStatusTableRow> {
    snapshot
        .rows()
        .into_iter()
        .map(|row| {
            let pull_request_number = row.node.pull_request_number();
            let status = pull_request_number.and_then(|number| report.statuses.get(&number));
            let merged = stack_status_row_is_merged(row.node, status);
            let closed = stack_status_row_is_closed(status, merged);
            let draft = stack_status_row_is_draft(row.node, status, merged, closed);
            let conflict = status.is_some_and(pull_request_has_merge_conflict);
            let style = stack_status_row_style(conflict, closed, draft, color);
            let active_cell_color = color && !draft && !closed;
            let pr_cell = stack_status_pr_cell(report, &row, status, merged, color);
            let review_lag =
                pull_request_stack_review_lag(status, report.review_wait_threshold_seconds);
            StackStatusTableRow {
                pr_visible_width: pr_cell.visible_width,
                pr_cell: pr_cell.rendered,
                check_symbol: pull_request_check_symbol_with_restore(
                    status,
                    merged,
                    active_cell_color,
                    style,
                ),
                review_symbol: pull_request_review_symbol_with_restore(
                    status,
                    merged,
                    active_cell_color,
                    review_lag.over_threshold,
                    style,
                ),
                review_lag,
                title: stack_status_title(
                    &row,
                    status,
                    draft,
                    merged,
                    closed,
                    color,
                    display_names,
                ),
                draft,
                merged,
                closed,
                style,
            }
        })
        .collect()
}

struct StackStatusPrCell {
    rendered: String,
    visible_width: usize,
}

fn stack_status_pr_cell(
    report: &PullRequestStackStatusReport,
    row: &PullRequestStackRow<'_>,
    status: Option<&PullRequestStatusRecord>,
    merged: bool,
    color: bool,
) -> StackStatusPrCell {
    let target = row
        .node
        .pull_request_number()
        .map(|number| format!("#{number}"))
        .unwrap_or_else(|| row.node.branch.clone());
    let styled_target = if merged && color {
        format!("{GREEN_STYLE}{target}{RESET_STYLE}")
    } else {
        target.clone()
    };
    let rendered_target = row.node.pull_request_number().map_or_else(
        || styled_target.clone(),
        |number| {
            osc8_link(
                &stack_status_pull_request_url(report, row.node, status, number),
                &styled_target,
            )
        },
    );
    StackStatusPrCell {
        rendered: rendered_target,
        visible_width: target.chars().count(),
    }
}

fn compact_stack_prefix(prefix: &str) -> String {
    prefix
        .replace("│  ", "│ ")
        .replace("   ", "  ")
        .replace("├─ ", "├ ")
        .replace("└─ ", "└ ")
}

fn stack_status_pull_request_url(
    report: &PullRequestStackStatusReport,
    node: &PullRequestStackNode,
    status: Option<&PullRequestStatusRecord>,
    number: u64,
) -> String {
    status
        .and_then(|status| status.url.clone())
        .or_else(|| {
            node.pull_request
                .as_ref()
                .and_then(|pull_request| pull_request.url.clone())
        })
        .unwrap_or_else(|| format!("{}/pull/{number}", report.repository.github_url))
}

fn stack_status_row_is_merged(
    node: &PullRequestStackNode,
    status: Option<&PullRequestStatusRecord>,
) -> bool {
    status.map_or(node.merged, |status| status.merged)
}

fn stack_status_row_is_closed(status: Option<&PullRequestStatusRecord>, merged: bool) -> bool {
    !merged && status.is_some_and(|status| status.closed)
}

fn stack_status_row_is_draft(
    node: &PullRequestStackNode,
    status: Option<&PullRequestStatusRecord>,
    merged: bool,
    closed: bool,
) -> bool {
    !merged && !closed && status.map_or(node.draft, |status| status.draft)
}

fn stack_status_title(
    row: &PullRequestStackRow<'_>,
    status: Option<&PullRequestStatusRecord>,
    draft: bool,
    merged: bool,
    closed: bool,
    color: bool,
    display_names: &BTreeMap<String, String>,
) -> String {
    let title = ellipsize_pull_request_title(
        status
            .map(|status| status.title.trim())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| row.node.display_title()),
    );
    if merged || closed {
        let label_chips = if merged {
            status
                .map(|status| muted_pull_request_label_chips(&status.labels, color))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut title = if merged {
            if color {
                format!("{GREEN_STYLE}● {title}{RESET_STYLE}")
            } else {
                format!("● {title}")
            }
        } else {
            format!("⊖ {title}")
        };
        if !label_chips.is_empty() {
            title.push(' ');
            title.push_str(&label_chips.join(pull_request_label_separator(color)));
        }
        if merged {
            let reviewer_tokens = status
                .map(|status| pull_request_completed_reviewer_tokens(status, color, display_names))
                .unwrap_or_default();
            if !reviewer_tokens.is_empty() {
                title.push(' ');
                title.push_str(&reviewer_tokens.join(", "));
            }
        }
        let prefix = compact_stack_prefix(&row.prefix);
        return format!("{prefix}{title}");
    }

    let label_chips = status
        .map(|status| pull_request_label_chips(&status.labels, color, draft))
        .unwrap_or_default();
    let reviewer_tokens = status
        .map(|status| pull_request_reviewer_tokens(status, color && !draft, display_names))
        .unwrap_or_default();
    let prefix = compact_stack_prefix(&row.prefix);
    let symbol = pull_request_node_symbol(status, draft);
    let mut parts = vec![format!("{prefix}{symbol} {title}")];
    if !label_chips.is_empty() {
        parts.push(label_chips.join(pull_request_label_separator(color)));
    }
    if !reviewer_tokens.is_empty() {
        parts.push(reviewer_tokens.join(", "));
    }
    parts.join(" ")
}

fn stack_status_repository_header(
    label: &str,
    url: Option<&str>,
    display_root: &str,
    report: Option<&PullRequestStackStatusReport>,
    color: bool,
) -> String {
    let label = url.map_or_else(|| label.to_owned(), |url| osc8_link(url, label));
    let trunk = report
        .and_then(|report| report.trunk.as_ref())
        .map(|trunk| format!("  ({})", stack_status_trunk_summary(trunk)))
        .unwrap_or_default();
    if color {
        format!("{BOLD_STYLE}{label}{RESET_STYLE}{DIM_STYLE}  {display_root}{trunk}{RESET_STYLE}")
    } else {
        format!("{label}  {display_root}{trunk}")
    }
}

fn stack_status_trunk_summary(trunk: &RemoteStatusReport) -> String {
    format!(
        "{}/{} {}",
        trunk.name,
        trunk.branch,
        stack_status_trunk_delta(trunk)
    )
}

fn stack_status_trunk_delta(trunk: &RemoteStatusReport) -> String {
    let local_ahead = trunk.comparison.github_behind_by + trunk.local_ahead_by;
    if !trunk.comparison.counts_exact {
        return match (trunk.comparison.state, local_ahead > 0) {
            (StatusState::GithubAhead | StatusState::Diverged, false) => "behind".to_owned(),
            (StatusState::GithubAhead | StatusState::Diverged, true) => {
                format!("behind, {} ahead", commit_count_i64(local_ahead))
            }
            (StatusState::UpToDate | StatusState::LocalAhead, false) => "up to date".to_owned(),
            (StatusState::UpToDate | StatusState::LocalAhead, true) => {
                format!("{} ahead", commit_count_i64(local_ahead))
            }
        };
    }

    let github_ahead = trunk.comparison.github_ahead_by;
    match (github_ahead > 0, local_ahead > 0) {
        (true, false) => format!("{} behind", commit_count_i64(github_ahead)),
        (false, true) => format!("{} ahead", commit_count_i64(local_ahead)),
        (true, true) => format!(
            "diverged: {} behind, {} ahead",
            commit_count_i64(github_ahead),
            commit_count_i64(local_ahead),
        ),
        (false, false) => "up to date".to_owned(),
    }
}

fn visible_stack_status_snapshot(
    report: &PullRequestStackStatusReport,
) -> PullRequestStackSnapshot {
    let visible_branches = report
        .snapshot
        .nodes
        .iter()
        .map(|node| node.branch.clone())
        .collect::<BTreeSet<_>>();
    let nodes = report
        .snapshot
        .nodes
        .iter()
        .filter(|node| visible_branches.contains(node.branch.as_str()))
        .cloned()
        .map(|mut node| {
            if node
                .parent_branch
                .as_ref()
                .is_some_and(|parent| !visible_branches.contains(parent))
            {
                node.parent_branch = None;
                node.parent_pull_request = None;
            }
            node
        })
        .collect::<Vec<_>>();
    let current_branch = report
        .snapshot
        .current_branch
        .as_ref()
        .filter(|branch| visible_branches.contains(*branch))
        .cloned();
    let current_pull_request = report.snapshot.current_pull_request.filter(|number| {
        nodes
            .iter()
            .any(|node| node.pull_request_number() == Some(*number))
    });

    PullRequestStackSnapshot {
        nodes,
        current_branch,
        current_pull_request,
    }
}

fn lifecycle_label(
    node: &PullRequestStackNode,
    status: Option<&PullRequestStatusRecord>,
) -> &'static str {
    match status {
        Some(status) if status.merged => "merged",
        Some(status) if status.closed => "closed",
        Some(status) if status.draft => "draft",
        Some(_) => "ready",
        None if node.merged => "merged",
        None if node.draft => "draft",
        None if node.pull_request_number().is_some() => "unknown",
        None => "no-pr",
    }
}

fn stack_status_row_style(conflict: bool, closed: bool, draft: bool, color: bool) -> &'static str {
    if !color {
        ""
    } else if conflict {
        CONFLICT_STYLE
    } else if closed {
        PASTEL_BLUE_STYLE
    } else if draft {
        DRAFT_ROW_STYLE
    } else {
        ""
    }
}

fn style_stack_status_row(line: String, style: &str) -> String {
    if style.is_empty() {
        line
    } else {
        format!("{style}{line}{RESET_STYLE}")
    }
}

#[derive(serde::Serialize)]
struct StackStatusJson {
    command: &'static str,
    version: u8,
    repositories: Vec<StackStatusRepositoryJson>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StackStatusRepositoryJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trunk: Option<StackStatusTrunkJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pull_requests: Vec<StackStatusPullRequestJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl From<&GlobalStackStatusEntry> for StackStatusRepositoryJson {
    fn from(entry: &GlobalStackStatusEntry) -> Self {
        Self {
            key: entry.key.clone(),
            root: entry.root.display().to_string(),
            repository: entry.repository.as_ref().map(GitHubRepository::slug),
            url: entry.repository.as_ref().map(GitHubRepository::https_url),
            trunk: entry
                .result
                .as_ref()
                .ok()
                .and_then(|report| report.trunk.as_ref())
                .map(StackStatusTrunkJson::from),
            pull_requests: entry
                .result
                .as_ref()
                .map(stack_status_pull_requests_json)
                .unwrap_or_default(),
            error: entry.result.as_ref().err().cloned(),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StackStatusTrunkJson {
    remote: String,
    branch: String,
    url: String,
    github_url: String,
    local_trunk_sha: String,
    local_trunk_short_sha: String,
    local_ahead_by: i64,
    state: &'static str,
    github_ahead_by: i64,
    github_behind_by: i64,
    counts_exact: bool,
}

impl From<&RemoteStatusReport> for StackStatusTrunkJson {
    fn from(trunk: &RemoteStatusReport) -> Self {
        Self {
            remote: trunk.name.clone(),
            branch: trunk.branch.clone(),
            url: trunk.url.clone(),
            github_url: trunk.github_url.clone(),
            local_trunk_sha: trunk.local_trunk_sha.clone(),
            local_trunk_short_sha: trunk.local_trunk_short_sha.clone(),
            local_ahead_by: trunk.local_ahead_by,
            state: trunk.comparison.label(),
            github_ahead_by: trunk.comparison.github_ahead_by,
            github_behind_by: trunk.comparison.github_behind_by,
            counts_exact: trunk.comparison.counts_exact,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StackStatusPullRequestJson {
    number: u64,
    title: String,
    branch: String,
    base_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    lifecycle: &'static str,
    check_status: &'static str,
    merge_status: &'static str,
    review_status: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labels: Vec<StackStatusLabelJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    requested_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggested_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    approved_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    changes_requested_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    commented_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    addressed_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reviewer_responses: Vec<StackStatusResponseJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dismissed_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    requested_teams: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_commit_oid: Option<String>,
    local: bool,
    current: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StackStatusLabelJson {
    name: String,
    color: String,
}

impl From<&PullRequestLabel> for StackStatusLabelJson {
    fn from(label: &PullRequestLabel) -> Self {
        Self {
            name: label.name.clone(),
            color: label.color.clone(),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StackStatusResponseJson {
    reviewer: String,
    responded_at: String,
}

impl From<&PullRequestReviewerResponse> for StackStatusResponseJson {
    fn from(response: &PullRequestReviewerResponse) -> Self {
        Self {
            reviewer: response.reviewer.clone(),
            responded_at: response.responded_at.clone(),
        }
    }
}

fn stack_status_pull_requests_json(
    report: &PullRequestStackStatusReport,
) -> Vec<StackStatusPullRequestJson> {
    visible_stack_status_snapshot(report)
        .nodes
        .iter()
        .filter_map(|node| {
            let number = node.pull_request_number()?;
            let status = report.statuses.get(&number);
            Some(StackStatusPullRequestJson {
                number,
                title: status
                    .map(|status| status.title.clone())
                    .unwrap_or_else(|| node.title.clone()),
                branch: status
                    .map(|status| status.head_branch.clone())
                    .unwrap_or_else(|| node.branch.clone()),
                base_branch: status
                    .map(|status| status.base_branch.clone())
                    .unwrap_or_else(|| node.base_branch.clone()),
                url: status.and_then(|status| status.url.clone()).or_else(|| {
                    node.pull_request
                        .as_ref()
                        .and_then(|pull_request| pull_request.url.clone())
                }),
                lifecycle: lifecycle_label(node, status),
                check_status: status
                    .map(|status| status.check_status.label())
                    .unwrap_or("unknown"),
                merge_status: status
                    .map(|status| status.merge_status.label())
                    .unwrap_or("unknown"),
                review_status: status
                    .map(|status| status.review_status.label())
                    .unwrap_or("unknown"),
                labels: status
                    .map(|status| {
                        status
                            .labels
                            .iter()
                            .map(StackStatusLabelJson::from)
                            .collect()
                    })
                    .unwrap_or_default(),
                requested_users: status
                    .map(|status| status.requested_reviewers.users.clone())
                    .unwrap_or_default(),
                suggested_users: status
                    .map(|status| status.suggested_reviewers.clone())
                    .unwrap_or_default(),
                approved_users: status
                    .map(|status| status.approved_reviewers.clone())
                    .unwrap_or_default(),
                changes_requested_users: status
                    .map(|status| status.changes_requested_reviewers.clone())
                    .unwrap_or_default(),
                commented_users: status
                    .map(|status| status.commented_reviewers.clone())
                    .unwrap_or_default(),
                addressed_users: status
                    .map(|status| status.addressed_reviewers.clone())
                    .unwrap_or_default(),
                reviewer_responses: status
                    .map(|status| {
                        status
                            .reviewer_responses
                            .iter()
                            .map(StackStatusResponseJson::from)
                            .collect()
                    })
                    .unwrap_or_default(),
                dismissed_users: status
                    .map(|status| status.dismissed_reviewers.clone())
                    .unwrap_or_default(),
                requested_teams: status
                    .map(|status| status.requested_reviewers.teams.clone())
                    .unwrap_or_default(),
                latest_commit_oid: status.and_then(|status| status.latest_commit_oid.clone()),
                local: node.is_local,
                current: node.is_current,
            })
        })
        .collect()
}

trait RepositorySummaryExt {
    fn github_slug_owner(&self) -> &str;
    fn github_slug_name(&self) -> &str;
}

impl RepositorySummaryExt for RepositorySummary {
    fn github_slug_owner(&self) -> &str {
        self.github_slug
            .split_once('/')
            .map_or("", |(owner, _)| owner)
    }

    fn github_slug_name(&self) -> &str {
        self.github_slug
            .split_once('/')
            .map_or("", |(_, name)| name)
    }
}
