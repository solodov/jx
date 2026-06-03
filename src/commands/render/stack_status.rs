use super::*;

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
) -> Result<String, JjError> {
    let _ = (current_dir, color);
    Ok(render_plain_output(|formatter| {
        write_stack_status_report(formatter, report, color, 0)
    }))
}

pub(in crate::commands) fn render_global_stack_status(
    entries: &[GlobalStackStatusEntry],
    total_repositories: usize,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    let _ = (current_dir, color);
    Ok(render_plain_output(|formatter| {
        let stack_repositories = entries
            .iter()
            .filter(|entry| {
                entry
                    .result
                    .as_ref()
                    .is_ok_and(|report| visible_stack_status_node_count(report) > 0)
            })
            .count();
        let pull_requests = entries
            .iter()
            .filter_map(|entry| entry.result.as_ref().ok())
            .map(visible_stack_status_pull_request_count)
            .sum::<usize>();
        let attention = entries
            .iter()
            .filter_map(|entry| entry.result.as_ref().ok())
            .filter(|report| stack_status_report_needs_attention(report))
            .count();

        writeln!(
            formatter,
            "Stack status: {} checked, {} with stacks, {}, {}",
            repository_count(total_repositories),
            repository_count(stack_repositories),
            pull_request_count(pull_requests),
            attention_repository_count(attention),
        )?;

        for entry in entries {
            writeln!(formatter)?;
            let label = entry
                .key
                .as_deref()
                .or_else(|| {
                    entry
                        .repository
                        .as_ref()
                        .map(|repository| repository.name.as_str())
                })
                .unwrap_or("repository");
            writeln!(formatter, "{label}  {}", entry.display_root)?;
            match &entry.result {
                Ok(report) => write_stack_status_report(formatter, report, color, 2)?,
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
) -> io::Result<()> {
    let indent = " ".repeat(indent);
    let visible_snapshot = visible_stack_status_snapshot(report);
    if visible_snapshot.nodes.is_empty() {
        writeln!(formatter, "{indent}No stack state")?;
        return Ok(());
    }

    for row in visible_snapshot.rows() {
        let pull_request_number = row.node.pull_request_number();
        let status = pull_request_number.and_then(|number| report.statuses.get(&number));
        let lifecycle = lifecycle_label(row.node, status);
        let label = render_stack_row_label(row, color);
        writeln!(
            formatter,
            "{indent}{label}  {lifecycle:<6}  {:<14}  {}",
            check_label(status),
            review_label(status),
        )?;
    }
    Ok(())
}

fn stack_status_report_needs_attention(report: &PullRequestStackStatusReport) -> bool {
    visible_stack_status_snapshot(report)
        .nodes
        .iter()
        .any(|node| {
            node.pull_request_number()
                .and_then(|number| report.statuses.get(&number))
                .is_none_or(pull_request_status_needs_attention)
        })
}

fn visible_stack_status_node_count(report: &PullRequestStackStatusReport) -> usize {
    visible_stack_status_snapshot(report).nodes.len()
}

fn visible_stack_status_pull_request_count(report: &PullRequestStackStatusReport) -> usize {
    visible_stack_status_snapshot(report)
        .nodes
        .iter()
        .filter(|node| node.pull_request_number().is_some())
        .count()
}

fn visible_stack_status_snapshot(
    report: &PullRequestStackStatusReport,
) -> PullRequestStackSnapshot {
    let visible_branches = report
        .snapshot
        .nodes
        .iter()
        .filter(|node| !stack_status_node_is_done(node, report))
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

fn stack_status_node_is_done(
    node: &PullRequestStackNode,
    report: &PullRequestStackStatusReport,
) -> bool {
    node.pull_request_number()
        .and_then(|number| report.statuses.get(&number))
        .map_or(node.merged, |status| status.merged || status.closed)
}

fn pull_request_status_needs_attention(status: &PullRequestStatusRecord) -> bool {
    matches!(
        status.check_status,
        PullRequestCheckStatus::Failing | PullRequestCheckStatus::Unknown
    ) || matches!(
        status.review_status,
        PullRequestReviewStatus::ChangesRequested | PullRequestReviewStatus::Unknown
    )
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

fn check_label(status: Option<&PullRequestStatusRecord>) -> &'static str {
    match status.map(|status| status.check_status) {
        Some(PullRequestCheckStatus::Passing) => "checks passing",
        Some(PullRequestCheckStatus::Failing) => "checks failing",
        Some(PullRequestCheckStatus::Pending) => "checks pending",
        Some(PullRequestCheckStatus::Missing) => "checks missing",
        Some(PullRequestCheckStatus::Unknown) => "checks unknown",
        None => "checks unknown",
    }
}

fn review_label(status: Option<&PullRequestStatusRecord>) -> String {
    let Some(status) = status else {
        return "review unknown".to_owned();
    };
    match status.review_status {
        PullRequestReviewStatus::Approved => "approved".to_owned(),
        PullRequestReviewStatus::ChangesRequested => "changes requested".to_owned(),
        PullRequestReviewStatus::ReviewRequired => "review required".to_owned(),
        PullRequestReviewStatus::ReviewRequested => {
            requested_reviewers_label(&status.requested_reviewers)
        }
        PullRequestReviewStatus::NotReviewed => "not reviewed".to_owned(),
        PullRequestReviewStatus::Unknown => "review unknown".to_owned(),
    }
}

fn requested_reviewers_label(reviewers: &ReviewerSelection) -> String {
    let mut names = reviewers.users.clone();
    names.extend(reviewers.teams.iter().map(|team| format!("team/{team}")));
    if names.is_empty() {
        "review requested".to_owned()
    } else {
        format!("review requested: {}", names.join(", "))
    }
}

fn repository_count(count: usize) -> String {
    match count {
        1 => "1 repository".to_owned(),
        count => format!("{count} repositories"),
    }
}

fn pull_request_count(count: usize) -> String {
    match count {
        1 => "1 pull request".to_owned(),
        count => format!("{count} pull requests"),
    }
}

fn attention_repository_count(count: usize) -> String {
    match count {
        1 => "1 repository needs attention".to_owned(),
        count => format!("{count} repositories need attention"),
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
struct StackStatusPullRequestJson {
    number: u64,
    title: String,
    branch: String,
    base_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    lifecycle: &'static str,
    check_status: &'static str,
    review_status: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    requested_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    requested_teams: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_commit_oid: Option<String>,
    local: bool,
    current: bool,
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
                review_status: status
                    .map(|status| status.review_status.label())
                    .unwrap_or("unknown"),
                requested_users: status
                    .map(|status| status.requested_reviewers.users.clone())
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
