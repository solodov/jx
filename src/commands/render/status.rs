use super::*;

/// Renders the shared current commit status block shown by `jx status` and stack publish previews.
pub(in crate::commands) fn render_workspace_status(status: &WorkspaceStatus) -> String {
    render_workspace_status_with_width(status, termimad::terminal_size().0.into())
}

pub(in crate::commands) fn render_workspace_status_with_width(
    status: &WorkspaceStatus,
    width: usize,
) -> String {
    let mut lines = Vec::new();
    lines.extend(status.commit_lines.iter().cloned());

    let description = status.description.trim_end();
    if !description.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        let rendered_description = render_status_description(description, width);
        lines.extend(rendered_description.trim_end().lines().map(str::to_owned));
    }

    if !status.change_lines.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(status.change_lines.iter().cloned());
    }

    if !status.extra_lines.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(status.extra_lines.iter().cloned());
    }

    lines.push(String::new());
    lines.join("\n")
}

pub(in crate::commands) fn render_status_description(description: &str, width: usize) -> String {
    let description = domain::pull_request_description_without_stack_context_markers(description);
    MadSkin::default_light()
        .text(&description, Some(width.max(20)))
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct GlobalStatusEntry {
    pub(in crate::commands) key: Option<String>,
    pub(in crate::commands) root: PathBuf,
    pub(in crate::commands) display_root: String,
    pub(in crate::commands) repository: Option<GitHubRepository>,
    pub(in crate::commands) result: Result<StatusReport, String>,
}

pub(in crate::commands) fn render_status(
    report: &StatusReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_status(formatter, report)
    })
}

pub(in crate::commands) fn render_status_json(entries: &[GlobalStatusEntry]) -> String {
    let output = RemoteStatusJson {
        command: "remote-status",
        version: 1,
        repositories: sorted_global_status_entries(entries)
            .into_iter()
            .map(RemoteStatusRepositoryJson::from)
            .collect(),
    };
    let mut rendered = serde_json::to_string_pretty(&output)
        .expect("remote-status JSON contains only serializable values");
    rendered.push('\n');
    rendered
}

#[derive(serde::Serialize)]
struct RemoteStatusJson {
    command: &'static str,
    version: u8,
    repositories: Vec<RemoteStatusRepositoryJson>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteStatusRepositoryJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    remotes: Vec<RemoteStatusRemoteJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fork: Option<RemoteStatusForkJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl From<&GlobalStatusEntry> for RemoteStatusRepositoryJson {
    fn from(entry: &GlobalStatusEntry) -> Self {
        let remotes = entry
            .result
            .as_ref()
            .map(|report| {
                sorted_remote_statuses(report)
                    .into_iter()
                    .map(RemoteStatusRemoteJson::from)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            key: entry.key.clone(),
            root: entry.root.display().to_string(),
            repository: entry.repository.as_ref().map(GitHubRepository::slug),
            url: entry.repository.as_ref().map(GitHubRepository::https_url),
            remotes,
            fork: entry
                .result
                .as_ref()
                .ok()
                .and_then(|report| report.fork.as_ref())
                .map(RemoteStatusForkJson::from),
            error: entry.result.as_ref().err().cloned(),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteStatusForkJson {
    source_repository: String,
    source_url: String,
    source_branch: String,
    fork_repository: String,
    fork_url: String,
    fork_branch: String,
    state: &'static str,
    source_ahead_by: i64,
    fork_ahead_by: i64,
}

impl From<&ForkStatusReport> for RemoteStatusForkJson {
    fn from(fork: &ForkStatusReport) -> Self {
        Self {
            source_repository: fork.source.slug(),
            source_url: fork.source.https_url(),
            source_branch: fork.source_branch.clone(),
            fork_repository: fork.fork.slug(),
            fork_url: fork.fork.https_url(),
            fork_branch: fork.fork_branch.clone(),
            state: fork.comparison.label(),
            source_ahead_by: fork.comparison.source_ahead_by,
            fork_ahead_by: fork.comparison.fork_ahead_by,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteStatusRemoteJson {
    name: String,
    url: String,
    github_url: String,
    branch: String,
    local_trunk_sha: String,
    local_trunk_short_sha: String,
    local_ahead_by: i64,
    state: &'static str,
    github_ahead_by: i64,
    github_behind_by: i64,
    counts_exact: bool,
}

impl From<&RemoteStatusReport> for RemoteStatusRemoteJson {
    fn from(remote: &RemoteStatusReport) -> Self {
        Self {
            name: remote.name.clone(),
            url: remote.url.clone(),
            github_url: remote.github_url.clone(),
            branch: remote.branch.clone(),
            local_trunk_sha: remote.local_trunk_sha.clone(),
            local_trunk_short_sha: remote.local_trunk_short_sha.clone(),
            local_ahead_by: remote.local_ahead_by,
            state: remote.comparison.label(),
            github_ahead_by: remote.comparison.github_ahead_by,
            github_behind_by: remote.comparison.github_behind_by,
            counts_exact: remote.comparison.counts_exact,
        }
    }
}

pub(in crate::commands) fn render_global_status(
    entries: &[GlobalStatusEntry],
    total_repositories: usize,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    let groups = global_status_groups(entries, total_repositories);
    let _ = (current_dir, color);
    Ok(render_plain_output(|formatter| {
        writeln!(
            formatter,
            "Remote status: {} checked, {}",
            repository_count(total_repositories),
            attention_count(groups.attention_repositories)
        )?;

        let label_width = groups.label_width();
        write_global_status_section(
            formatter,
            "Pull needed: GitHub has new commits",
            &groups.pull_needed,
            label_width,
        )?;
        write_global_status_section(
            formatter,
            "Push needed: local commits are unpublished",
            &groups.push_needed,
            label_width,
        )?;
        write_global_status_section(
            formatter,
            "Diverged: pull and push needed",
            &groups.diverged,
            label_width,
        )?;
        write_global_status_section(
            formatter,
            "Fork behind source:",
            &groups.fork_behind,
            label_width,
        )?;
        write_global_status_section(
            formatter,
            "Fork ahead of source:",
            &groups.fork_ahead,
            label_width,
        )?;
        write_global_status_section(
            formatter,
            "Fork diverged from source:",
            &groups.fork_diverged,
            label_width,
        )?;
        write_global_status_section(
            formatter,
            "Setup needed:",
            &groups.setup_needed,
            label_width,
        )?;

        if groups.synced_repositories > 0 {
            writeln!(formatter)?;
            writeln!(
                formatter,
                "Synced: {}",
                repository_count(groups.synced_repositories)
            )?;
        }
        Ok(())
    }))
}

#[derive(Debug, Default)]
struct GlobalStatusGroups {
    pull_needed: Vec<GlobalStatusRow>,
    push_needed: Vec<GlobalStatusRow>,
    diverged: Vec<GlobalStatusRow>,
    fork_behind: Vec<GlobalStatusRow>,
    fork_ahead: Vec<GlobalStatusRow>,
    fork_diverged: Vec<GlobalStatusRow>,
    setup_needed: Vec<GlobalStatusRow>,
    attention_repositories: usize,
    synced_repositories: usize,
}

impl GlobalStatusGroups {
    fn label_width(&self) -> usize {
        self.pull_needed
            .iter()
            .chain(&self.push_needed)
            .chain(&self.diverged)
            .chain(&self.fork_behind)
            .chain(&self.fork_ahead)
            .chain(&self.fork_diverged)
            .chain(&self.setup_needed)
            .map(|row| row.label.len())
            .max()
            .unwrap_or(0)
    }
}

#[derive(Debug)]
struct GlobalStatusRow {
    label: String,
    detail: String,
}

fn global_status_groups(
    entries: &[GlobalStatusEntry],
    total_repositories: usize,
) -> GlobalStatusGroups {
    let mut groups = GlobalStatusGroups::default();

    for entry in sorted_global_status_entries(entries) {
        match &entry.result {
            Ok(report) => {
                let before = groups.changed_row_count();
                push_global_status_rows(&mut groups, entry, report);
                if groups.changed_row_count() > before {
                    groups.attention_repositories += 1;
                }
            }
            Err(message) => {
                groups.attention_repositories += 1;
                groups.setup_needed.push(GlobalStatusRow {
                    label: entry.display_root.clone(),
                    detail: message.clone(),
                });
            }
        }
    }

    groups.synced_repositories = total_repositories.saturating_sub(groups.attention_repositories);
    groups
}

impl GlobalStatusGroups {
    fn changed_row_count(&self) -> usize {
        self.pull_needed.len()
            + self.push_needed.len()
            + self.diverged.len()
            + self.fork_behind.len()
            + self.fork_ahead.len()
            + self.fork_diverged.len()
    }
}

fn push_global_status_rows(
    groups: &mut GlobalStatusGroups,
    entry: &GlobalStatusEntry,
    report: &StatusReport,
) {
    let remotes = sorted_remote_statuses(report);
    let show_remote_name = remotes.len() > 1;
    for remote in remotes {
        let counts = remote_status_counts(remote);
        let label = global_status_remote_label(entry, remote, show_remote_name);
        match (counts.pull > 0, counts.push > 0) {
            (true, false) => groups.pull_needed.push(GlobalStatusRow {
                label,
                detail: format!("{} to pull", commit_count_i64(counts.pull)),
            }),
            (false, true) => groups.push_needed.push(GlobalStatusRow {
                label,
                detail: format!("{} to push", commit_count_i64(counts.push)),
            }),
            (true, true) => groups.diverged.push(GlobalStatusRow {
                label,
                detail: format!(
                    "pull {}, push {}",
                    commit_count_i64(counts.pull),
                    commit_count_i64(counts.push)
                ),
            }),
            (false, false) => {}
        }
    }

    if let Some(fork) = &report.fork {
        push_global_fork_status_row(groups, entry, fork);
    }
}

fn push_global_fork_status_row(
    groups: &mut GlobalStatusGroups,
    entry: &GlobalStatusEntry,
    fork: &ForkStatusReport,
) {
    let label = entry.display_root.clone();
    match fork.comparison.state {
        ForkStatusState::SourceAhead => groups.fork_behind.push(GlobalStatusRow {
            label,
            detail: format!(
                "{} has {}",
                source_branch_label(fork),
                new_commit_count(fork.comparison.source_ahead_by)
            ),
        }),
        ForkStatusState::ForkAhead => groups.fork_ahead.push(GlobalStatusRow {
            label,
            detail: format!(
                "fork has {} not in {}",
                commit_count_i64(fork.comparison.fork_ahead_by),
                source_branch_label(fork)
            ),
        }),
        ForkStatusState::Diverged => groups.fork_diverged.push(GlobalStatusRow {
            label,
            detail: format!(
                "{} has {}, fork has {}",
                source_branch_label(fork),
                new_commit_count(fork.comparison.source_ahead_by),
                commit_count_i64(fork.comparison.fork_ahead_by)
            ),
        }),
        ForkStatusState::Synced => {}
    }
}

fn global_status_remote_label(
    entry: &GlobalStatusEntry,
    remote: &RemoteStatusReport,
    show_remote_name: bool,
) -> String {
    if show_remote_name || remote.name != "origin" {
        format!("{} ({})", entry.display_root, remote.name)
    } else {
        entry.display_root.clone()
    }
}

fn write_global_status_section(
    formatter: &mut dyn Formatter,
    title: &str,
    rows: &[GlobalStatusRow],
    label_width: usize,
) -> io::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    writeln!(formatter)?;
    writeln!(formatter, "{title}")?;
    for row in rows {
        writeln!(
            formatter,
            "  {label:<label_width$}  {detail}",
            label = row.label.as_str(),
            detail = row.detail.as_str()
        )?;
    }
    Ok(())
}

fn repository_count(count: usize) -> String {
    let noun = if count == 1 {
        "repository"
    } else {
        "repositories"
    };
    format!("{count} {noun}")
}

fn attention_count(count: usize) -> String {
    if count == 1 {
        "1 needs attention".to_owned()
    } else {
        format!("{count} need attention")
    }
}

fn sorted_global_status_entries(entries: &[GlobalStatusEntry]) -> Vec<&GlobalStatusEntry> {
    let mut sorted = entries.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|entry| entry.root.clone());
    sorted
}

fn sorted_remote_statuses(report: &StatusReport) -> Vec<&RemoteStatusReport> {
    let mut remotes = report.remotes.iter().collect::<Vec<_>>();
    remotes.sort_by(|left, right| {
        (&left.name, &left.branch, &left.github_url, &left.url).cmp(&(
            &right.name,
            &right.branch,
            &right.github_url,
            &right.url,
        ))
    });
    remotes
}

pub(in crate::commands) fn write_status(
    formatter: &mut dyn Formatter,
    report: &StatusReport,
) -> io::Result<()> {
    for remote in sorted_remote_statuses(report) {
        write_remote_status(formatter, remote)?;
    }
    if let Some(fork) = &report.fork {
        write_fork_status(formatter, fork)?;
    }
    Ok(())
}

pub(in crate::commands) fn bookmark_action_summary(action: BookmarkAction) -> &'static str {
    match action {
        BookmarkAction::Create => "will create",
        BookmarkAction::Reuse => "exists",
    }
}

pub(in crate::commands) fn pull_request_action(action: PullRequestAction) -> &'static str {
    match action {
        PullRequestAction::Created => "Created",
        PullRequestAction::Updated => "Updated",
    }
}

pub(in crate::commands) fn rebase_on_trunk_source_label(outcome: &RebaseOnTrunkOutcome) -> String {
    match outcome.source_short_commit_ids.as_slice() {
        [source] => source.clone(),
        sources => format!("{} sources", sources.len()),
    }
}

pub(in crate::commands) fn render_rebase_on_trunk_outcome(
    outcome: &RebaseOnTrunkOutcome,
) -> String {
    if outcome.rebased_commits == 0 {
        "up to date".to_owned()
    } else {
        format!("rebased {}", commit_count(outcome.rebased_commits))
    }
}

pub(in crate::commands) fn commit_count(count: usize) -> String {
    let noun = if count == 1 { "commit" } else { "commits" };
    format!("{count} {noun}")
}

pub(in crate::commands) fn write_remote_status(
    formatter: &mut dyn Formatter,
    remote: &domain::RemoteStatusReport,
) -> io::Result<()> {
    write!(formatter, "remote: {} (", remote.name)?;
    write_osc8_link(
        formatter,
        &branch_url(&remote.github_url, &remote.branch),
        &remote.url,
    )?;
    writeln!(formatter, "), {}", render_status_delta(remote))
}

pub(in crate::commands) fn render_status_delta(remote: &domain::RemoteStatusReport) -> String {
    let counts = remote_status_counts(remote);
    match (counts.pull > 0, counts.push > 0) {
        (true, false) => format!("pull needed: GitHub has {}", new_commit_count(counts.pull)),
        (false, true) => format!(
            "push needed: local has {}",
            unpublished_commit_count(counts.push)
        ),
        (true, true) => format!(
            "diverged: pull {}, push {}",
            commit_count_i64(counts.pull),
            commit_count_i64(counts.push)
        ),
        (false, false) => "synced".to_owned(),
    }
}

pub(in crate::commands) fn write_fork_status(
    formatter: &mut dyn Formatter,
    fork: &domain::ForkStatusReport,
) -> io::Result<()> {
    write!(formatter, "fork: ")?;
    write_osc8_link(
        formatter,
        &branch_url(&fork.fork.https_url(), &fork.fork_branch),
        &fork_branch_label(fork),
    )?;
    write!(formatter, " vs source ")?;
    write_osc8_link(
        formatter,
        &branch_url(&fork.source.https_url(), &fork.source_branch),
        &source_branch_label(fork),
    )?;
    writeln!(formatter, ", {}", render_fork_status_delta(fork))
}

pub(in crate::commands) fn render_fork_status_delta(fork: &domain::ForkStatusReport) -> String {
    match fork.comparison.state {
        ForkStatusState::SourceAhead => format!(
            "source has {}",
            new_commit_count(fork.comparison.source_ahead_by)
        ),
        ForkStatusState::ForkAhead => format!(
            "fork has {} not in source",
            commit_count_i64(fork.comparison.fork_ahead_by)
        ),
        ForkStatusState::Diverged => format!(
            "diverged: source has {}, fork has {}",
            new_commit_count(fork.comparison.source_ahead_by),
            commit_count_i64(fork.comparison.fork_ahead_by)
        ),
        ForkStatusState::Synced => "synced with source".to_owned(),
    }
}

fn fork_branch_label(fork: &domain::ForkStatusReport) -> String {
    format!("{}/{}", fork.fork.slug(), fork.fork_branch)
}

fn source_branch_label(fork: &domain::ForkStatusReport) -> String {
    format!("{}/{}", fork.source.slug(), fork.source_branch)
}

#[derive(Debug, Clone, Copy)]
struct RemoteStatusCounts {
    pull: i64,
    push: i64,
}

fn remote_status_counts(remote: &domain::RemoteStatusReport) -> RemoteStatusCounts {
    RemoteStatusCounts {
        pull: remote.comparison.github_ahead_by,
        push: remote.comparison.github_behind_by + remote.local_ahead_by,
    }
}

pub(in crate::commands) fn commit_count_i64(count: i64) -> String {
    let noun = if count == 1 { "commit" } else { "commits" };
    format!("{count} {noun}")
}

pub(in crate::commands) fn local_change_count_i64(count: i64) -> String {
    let noun = if count == 1 {
        "local change"
    } else {
        "local changes"
    };
    format!("{count} {noun}")
}

pub(in crate::commands) fn new_commit_count(count: i64) -> String {
    let noun = if count == 1 {
        "new commit"
    } else {
        "new commits"
    };
    format!("{count} {noun}")
}

pub(in crate::commands) fn unpublished_commit_count(count: i64) -> String {
    let noun = if count == 1 {
        "unpublished commit"
    } else {
        "unpublished commits"
    };
    format!("{count} {noun}")
}

pub(in crate::commands) fn branch_url(repository_url: &str, branch: &str) -> String {
    format!("{repository_url}/tree/{branch}")
}

pub(in crate::commands) fn bookmark_pull_request_url(
    repository_url: &str,
    bookmark: &str,
) -> String {
    let query = url_query_encode(&format!("is:pr head:{bookmark}"));
    format!("{repository_url}/pulls?q={query}")
}

pub(in crate::commands) fn linked_bookmark_text(repository_url: &str, bookmark: &str) -> String {
    osc8_link(
        &bookmark_pull_request_url(repository_url, bookmark),
        bookmark,
    )
}

pub(in crate::commands) fn linked_pull_request_text(
    repository_url: &str,
    pull_request: &PullRequestRecord,
) -> String {
    osc8_link(
        &pull_request_url(repository_url, pull_request),
        &format!("#{}", pull_request.number),
    )
}

pub(in crate::commands) fn url_query_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

pub(in crate::commands) fn osc8_link(uri: &str, label: &str) -> String {
    format!("\x1b]8;;{uri}\x1b\\{label}\x1b]8;;\x1b\\")
}
