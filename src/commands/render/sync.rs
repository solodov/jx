use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct GlobalFetchEntry {
    pub(in crate::commands) root: PathBuf,
    pub(in crate::commands) display_root: String,
    pub(in crate::commands) result: Result<(), String>,
}

pub(in crate::commands) fn render_fetch(
    report: &FetchReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_fetch(formatter, report)
    })
}

pub(in crate::commands) fn render_global_fetch(
    entries: &[GlobalFetchEntry],
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    let sorted_entries = sorted_global_fetch_entries(entries);
    let _ = (current_dir, color);
    Ok(render_plain_output(|formatter| {
        let fetched = sorted_entries
            .iter()
            .filter(|entry| entry.result.is_ok())
            .map(|entry| entry.display_root.as_str());
        write_fetch_path_section(formatter, "Fetched:", fetched)?;

        let errors = sorted_entries
            .iter()
            .filter_map(|entry| match &entry.result {
                Ok(()) => None,
                Err(message) => Some(GlobalFetchErrorRow {
                    root: &entry.root,
                    label: entry.display_root.as_str(),
                    detail: message.clone(),
                }),
            });
        write_fetch_error_section(formatter, "Errors:", errors)
    }))
}

struct GlobalFetchErrorRow<'a> {
    root: &'a Path,
    label: &'a str,
    detail: String,
}

fn write_fetch_path_section<'a>(
    formatter: &mut dyn Formatter,
    title: &str,
    rows: impl Iterator<Item = &'a str>,
) -> io::Result<()> {
    let rows = rows.collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(());
    }

    writeln!(formatter, "{title}")?;
    for row in rows {
        writeln!(formatter, "  {row}")?;
    }
    Ok(())
}

fn write_fetch_error_section<'a>(
    formatter: &mut dyn Formatter,
    title: &str,
    rows: impl Iterator<Item = GlobalFetchErrorRow<'a>>,
) -> io::Result<()> {
    let mut rows = rows.collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(());
    }
    rows.sort_by(|left, right| left.root.cmp(right.root));

    writeln!(formatter)?;
    writeln!(formatter, "{title}")?;
    let label_width = rows.iter().map(|row| row.label.len()).max().unwrap_or(0);
    for row in rows {
        writeln!(
            formatter,
            "  {label:<label_width$}  {detail}",
            label = row.label,
            detail = row.detail
        )?;
    }
    Ok(())
}

fn sorted_global_fetch_entries(entries: &[GlobalFetchEntry]) -> Vec<&GlobalFetchEntry> {
    let mut sorted = entries.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|entry| entry.root.clone());
    sorted
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct GlobalSyncEntry {
    pub(in crate::commands) root: PathBuf,
    pub(in crate::commands) display_root: String,
    pub(in crate::commands) outcome: GlobalSyncOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) enum GlobalSyncOutcome {
    Synced,
    SyncedWithConflicts { detail: String },
    Skipped(GlobalSyncSkipReason),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) enum GlobalSyncSkipReason {
    UpToDate,
    PullNeeded { commits: i64 },
    Diverged { pull: i64, push: i64 },
    LocalWork { changes: i64 },
    PushAccessUnavailable(String),
    SetupNeeded(String),
}

pub(in crate::commands) fn render_global_sync(
    entries: &[GlobalSyncEntry],
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    let sorted_entries = sorted_global_sync_entries(entries);
    let _ = (current_dir, color);
    Ok(render_plain_output(|formatter| {
        let mut wrote_any = false;
        write_global_sync_path_section(
            formatter,
            &mut wrote_any,
            "Synced:",
            sorted_entries.iter().filter_map(|entry| {
                (entry.outcome == GlobalSyncOutcome::Synced).then_some(GlobalSyncPathRow {
                    root: &entry.root,
                    label: entry.display_root.as_str(),
                })
            }),
        )?;

        write_global_sync_section(
            formatter,
            &mut wrote_any,
            "Synced with conflicts:",
            sorted_entries
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    GlobalSyncOutcome::SyncedWithConflicts { detail } => {
                        Some(GlobalSyncSectionRow {
                            root: &entry.root,
                            label: entry.display_root.as_str(),
                            detail: detail.clone(),
                        })
                    }
                    _ => None,
                }),
        )?;

        write_global_sync_path_section(
            formatter,
            &mut wrote_any,
            "Skipped: up to date",
            sorted_entries
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::UpToDate) => {
                        Some(GlobalSyncPathRow {
                            root: &entry.root,
                            label: entry.display_root.as_str(),
                        })
                    }
                    _ => None,
                }),
        )?;

        write_global_sync_section(
            formatter,
            &mut wrote_any,
            "Skipped: pull needed",
            sorted_entries
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::PullNeeded { commits }) => {
                        Some(GlobalSyncSectionRow {
                            root: &entry.root,
                            label: entry.display_root.as_str(),
                            detail: format!("GitHub has {}", new_commit_count(*commits)),
                        })
                    }
                    _ => None,
                }),
        )?;
        write_global_sync_section(
            formatter,
            &mut wrote_any,
            "Skipped: diverged",
            sorted_entries
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::Diverged { pull, push }) => {
                        Some(GlobalSyncSectionRow {
                            root: &entry.root,
                            label: entry.display_root.as_str(),
                            detail: format!(
                                "pull {}, push {}",
                                commit_count_i64(*pull),
                                commit_count_i64(*push)
                            ),
                        })
                    }
                    _ => None,
                }),
        )?;
        write_global_sync_section(
            formatter,
            &mut wrote_any,
            "Skipped: local work",
            sorted_entries
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::LocalWork { changes }) => {
                        Some(GlobalSyncSectionRow {
                            root: &entry.root,
                            label: entry.display_root.as_str(),
                            detail: format!(
                                "working copy has {}",
                                local_change_count_i64(*changes)
                            ),
                        })
                    }
                    _ => None,
                }),
        )?;
        write_global_sync_section(
            formatter,
            &mut wrote_any,
            "Skipped: push access unavailable",
            sorted_entries
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::PushAccessUnavailable(
                        message,
                    )) => Some(GlobalSyncSectionRow {
                        root: &entry.root,
                        label: entry.display_root.as_str(),
                        detail: message.clone(),
                    }),
                    _ => None,
                }),
        )?;
        write_global_sync_section(
            formatter,
            &mut wrote_any,
            "Setup needed:",
            sorted_entries
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::SetupNeeded(message)) => {
                        Some(GlobalSyncSectionRow {
                            root: &entry.root,
                            label: entry.display_root.as_str(),
                            detail: message.clone(),
                        })
                    }
                    _ => None,
                }),
        )?;
        write_global_sync_section(
            formatter,
            &mut wrote_any,
            "Errors",
            sorted_entries
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    GlobalSyncOutcome::Error(message) => Some(GlobalSyncSectionRow {
                        root: &entry.root,
                        label: entry.display_root.as_str(),
                        detail: message.clone(),
                    }),
                    _ => None,
                }),
        )
    }))
}

fn sorted_global_sync_entries(entries: &[GlobalSyncEntry]) -> Vec<&GlobalSyncEntry> {
    let mut sorted = entries.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|entry| entry.root.clone());
    sorted
}

struct GlobalSyncPathRow<'a> {
    root: &'a Path,
    label: &'a str,
}

struct GlobalSyncSectionRow<'a> {
    root: &'a Path,
    label: &'a str,
    detail: String,
}

fn write_global_sync_path_section<'a>(
    formatter: &mut dyn Formatter,
    wrote_any: &mut bool,
    title: &str,
    rows: impl Iterator<Item = GlobalSyncPathRow<'a>>,
) -> io::Result<()> {
    let mut rows = rows.collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(());
    }
    rows.sort_by(|left, right| left.root.cmp(right.root));

    if *wrote_any {
        writeln!(formatter)?;
    }
    writeln!(formatter, "{title}")?;
    *wrote_any = true;
    for row in rows {
        writeln!(formatter, "  {}", row.label)?;
    }
    Ok(())
}

fn write_global_sync_section<'a>(
    formatter: &mut dyn Formatter,
    wrote_any: &mut bool,
    title: &str,
    rows: impl Iterator<Item = GlobalSyncSectionRow<'a>>,
) -> io::Result<()> {
    let mut rows = rows.collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(());
    }
    rows.sort_by(|left, right| left.root.cmp(right.root));

    let label_width = rows.iter().map(|row| row.label.len()).max().unwrap_or(0);
    if *wrote_any {
        writeln!(formatter)?;
    }
    writeln!(formatter, "{title}")?;
    *wrote_any = true;
    for row in rows {
        writeln!(
            formatter,
            "  {label:<label_width$}  {detail}",
            label = row.label,
            detail = row.detail
        )?;
    }
    Ok(())
}

pub(in crate::commands) fn write_fetch(
    formatter: &mut dyn Formatter,
    report: &FetchReport,
) -> io::Result<()> {
    write_fetch_prefix(formatter, report)?;
    writeln!(formatter, ")")?;
    write_rebased_section(
        formatter,
        report.repository.origin_name,
        &report.outcome.branch,
        visible_rebased_commits(&report.outcome),
    )
}

fn write_fetch_prefix(formatter: &mut dyn Formatter, report: &FetchReport) -> io::Result<()> {
    write!(
        formatter,
        "Fetched: {}/{} (",
        report.repository.origin_name, report.outcome.branch
    )?;
    write_osc8_link(
        formatter,
        &branch_url(&report.repository.github_url, &report.outcome.branch),
        &report.repository.origin_url,
    )
}

pub(in crate::commands) fn render_push(
    report: &PushReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_push(formatter, report)
    })
}

pub(in crate::commands) fn write_push(
    formatter: &mut dyn Formatter,
    report: &PushReport,
) -> io::Result<()> {
    write!(formatter, "Pushed: ")?;
    write_bookmark(
        formatter,
        &report.repository.github_url,
        &report.plan.bookmark.branch,
    )?;
    if report.push.pushed_refs == 0 {
        return writeln!(formatter, ", up to date");
    }

    let created = if report.bookmark_update.created {
        " (created bookmark)"
    } else {
        ""
    };
    writeln!(
        formatter,
        " -> {}{}",
        report.plan.target_short_commit_id, created
    )
}

pub(in crate::commands) fn render_tracked_push(
    report: &TrackedPushReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_tracked_push(formatter, report)
    })
}

pub(in crate::commands) fn write_tracked_push(
    formatter: &mut dyn Formatter,
    report: &TrackedPushReport,
) -> io::Result<()> {
    if report.outcome.bookmarks.is_empty() {
        return writeln!(formatter, "Pushed tracked bookmarks: nothing changed");
    }

    writeln!(formatter, "Pushed tracked bookmarks:")?;
    for bookmark in &report.outcome.bookmarks {
        write_pushed_bookmark(formatter, bookmark, &report.repository.github_url)?;
    }
    Ok(())
}

pub(in crate::commands) fn render_sync(
    report: &SyncReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_sync(report, formatter)
    })
}

pub(in crate::commands) fn render_repository_bootstrap(
    report: &RepositoryBootstrapReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_repository_bootstrap(report, formatter)
    })
}

pub(in crate::commands) fn write_repository_bootstrap(
    report: &RepositoryBootstrapReport,
    formatter: &mut dyn Formatter,
) -> io::Result<()> {
    write!(formatter, "Created private ")?;
    write_osc8_link(formatter, &report.repository_url, &report.remote_url)?;
    writeln!(formatter, " repo")?;
    write!(formatter, "Pushed ")?;
    write_commit_id(formatter, &report.push.short_commit_id)?;
    write!(formatter, " to ")?;
    write_branch(formatter, &report.repository_url, &report.push.branch)?;
    writeln!(formatter)?;
    if let Some(working_copy) = &report.push.working_copy_short_commit_id {
        write!(formatter, "Working copy now at ")?;
        write_commit_id(formatter, working_copy)?;
        writeln!(formatter, " (empty)")?;
    }
    Ok(())
}

const SYNC_COMMIT_WIDTH: usize = 8;
const SYNC_FALLBACK_WORKSPACE: &str = "default";

pub(in crate::commands) fn write_sync(
    report: &SyncReport,
    formatter: &mut dyn Formatter,
) -> io::Result<()> {
    write!(
        formatter,
        "Synced: {}/{} (",
        report.repository.origin_name, report.fetch.branch
    )?;
    write_osc8_link(
        formatter,
        &branch_url(&report.repository.github_url, &report.fetch.branch),
        &report.repository.origin_url,
    )?;
    writeln!(formatter, ")")?;
    if let Some(trunk) = &report.trunk {
        write_trunk_state(formatter, trunk)?;
    }

    let rebased_commits = visible_rebased_commits(&report.fetch);
    let pushed_bookmarks = report
        .push
        .bookmarks
        .iter()
        .filter(|bookmark| {
            bookmark.new_short_commit_id.is_some()
                && bookmark.old_short_commit_id != bookmark.new_short_commit_id
        })
        .collect::<Vec<_>>();
    let pull_requests = report
        .pull_requests
        .iter()
        .map(|pull_request| (pull_request.head_branch.as_str(), pull_request))
        .collect::<BTreeMap<_, _>>();

    let sections = sync_change_sections(
        &rebased_commits,
        &pushed_bookmarks,
        &report.skipped_conflicted_bookmarks,
        &pull_requests,
    );
    write_sync_change_section(
        formatter,
        "Conflicts blocking push:",
        &sections.conflicts,
        &report.repository.github_url,
    )?;
    write_sync_change_section(
        formatter,
        "Rebased and pushed:",
        &sections.rebased_and_pushed,
        &report.repository.github_url,
    )?;
    write_sync_change_section(
        formatter,
        "Pushed:",
        &sections.pushed,
        &report.repository.github_url,
    )?;
    write_sync_change_section(
        formatter,
        "Rebased locally:",
        &sections.rebased,
        &report.repository.github_url,
    )?;

    Ok(())
}

fn write_trunk_state(
    formatter: &mut dyn Formatter,
    trunk: &crate::jj::TrunkStateSummary,
) -> io::Result<()> {
    write!(formatter, "Trunk:  ")?;
    write_commit_id(formatter, &trunk.short_change_id)?;
    write!(
        formatter,
        "  {}  ",
        format_relative_age(trunk.committed_at_unix_ms)
    )?;
    write_description(formatter, &trunk.description)?;
    writeln!(formatter)
}

fn format_relative_age(committed_at_unix_ms: i64) -> String {
    let Some(committed_at) = chrono::DateTime::from_timestamp_millis(committed_at_unix_ms) else {
        return "unknown age".to_owned();
    };

    format_relative_age_at(committed_at, chrono::Utc::now())
}

fn format_relative_age_at(
    committed_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let seconds = now.signed_duration_since(committed_at).num_seconds().max(0);
    match seconds {
        0..=59 => "just now".to_owned(),
        60..=3_599 => relative_unit(seconds / 60, "minute"),
        3_600..=86_399 => relative_unit(seconds / 3_600, "hour"),
        86_400..=604_799 => relative_unit(seconds / 86_400, "day"),
        604_800..=31_535_999 => relative_unit(seconds / 604_800, "week"),
        _ => relative_unit(seconds / 31_536_000, "year"),
    }
}

fn relative_unit(value: i64, unit: &str) -> String {
    if value == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{value} {unit}s ago")
    }
}

pub(in crate::commands) fn write_rebased_section(
    formatter: &mut dyn Formatter,
    origin_name: &str,
    branch: &str,
    rebased_commits: Vec<&crate::jj::RebasedCommitSummary>,
) -> io::Result<()> {
    if rebased_commits.is_empty() {
        return Ok(());
    }

    write_sync_section_separator(formatter)?;
    writeln!(formatter, "Rebased on {origin_name}/{branch}:")?;
    let mut rows = sync_workspace_rows(rebased_commits, |commit| &commit.workspace_visibility);
    let workspace_width = sync_workspace_row_label_width(&rows);
    for row in &mut rows {
        write_rebased_commit(formatter, row, workspace_width)?;
    }
    Ok(())
}

fn sync_change_sections<'a>(
    rebased_commits: &[&'a crate::jj::RebasedCommitSummary],
    pushed_bookmarks: &[&'a crate::jj::PushedBookmarkSummary],
    skipped_conflicted_bookmarks: &'a [crate::jj::SkippedPushBookmarkSummary],
    pull_requests: &BTreeMap<&'a str, &'a PullRequestRecord>,
) -> SyncChangeSections<'a> {
    let mut sections = SyncChangeSections::default();
    let skipped_conflict_descriptions = skipped_conflicted_bookmarks
        .iter()
        .flat_map(|bookmark| bookmark.conflicted_commits.iter())
        .map(|commit| commit.description.as_str())
        .collect::<BTreeSet<_>>();

    for bookmark in skipped_conflicted_bookmarks {
        let pull_request = pull_requests.get(bookmark.branch.as_str()).copied();
        sections
            .conflicts
            .extend(
                bookmark
                    .conflicted_commits
                    .iter()
                    .map(|commit| SyncChangeEntry {
                        commit: commit.short_commit_id.as_str(),
                        description: commit.description.as_str(),
                        visibility: sync_display_visibility(&commit.workspace_visibility),
                        pull_request,
                        conflicted: true,
                    }),
            );
    }

    let mut used_pushed_indexes = BTreeSet::new();
    for rebased in rebased_commits {
        if rebased.has_conflict {
            if !skipped_conflict_descriptions.contains(rebased.description.as_str()) {
                sections.conflicts.push(sync_rebased_change_entry(rebased));
            }
            continue;
        }

        if let Some((index, pushed)) =
            pushed_bookmarks
                .iter()
                .enumerate()
                .find(|(index, bookmark)| {
                    !used_pushed_indexes.contains(index)
                        && bookmark.new_short_change_id.as_deref()
                            == Some(rebased.short_change_id.as_str())
                })
        {
            used_pushed_indexes.insert(index);
            let pull_request = pull_requests.get(pushed.branch.as_str()).copied();
            if let Some(entry) = sync_pushed_change_entry(
                pushed,
                pull_request,
                sync_rebased_pushed_visibility(rebased, pushed),
                Some(rebased.description.as_str()),
            ) {
                sections.rebased_and_pushed.push(entry);
            }
        } else {
            sections.rebased.push(sync_rebased_change_entry(rebased));
        }
    }

    for (index, pushed) in pushed_bookmarks.iter().enumerate() {
        if used_pushed_indexes.contains(&index) {
            continue;
        }

        let pull_request = pull_requests.get(pushed.branch.as_str()).copied();
        if let Some(entry) = sync_pushed_change_entry(
            pushed,
            pull_request,
            pushed.new_workspace_visibility.clone(),
            None,
        ) {
            sections.pushed.push(entry);
        }
    }

    sections
}

#[derive(Debug, Default)]
struct SyncChangeSections<'a> {
    conflicts: Vec<SyncChangeEntry<'a>>,
    rebased_and_pushed: Vec<SyncChangeEntry<'a>>,
    pushed: Vec<SyncChangeEntry<'a>>,
    rebased: Vec<SyncChangeEntry<'a>>,
}

#[derive(Debug)]
struct SyncChangeEntry<'a> {
    commit: &'a str,
    description: &'a str,
    visibility: crate::jj::WorkspaceVisibility,
    pull_request: Option<&'a PullRequestRecord>,
    conflicted: bool,
}

fn sync_display_visibility(
    visibility: &crate::jj::WorkspaceVisibility,
) -> crate::jj::WorkspaceVisibility {
    if visibility.names.is_empty() {
        crate::jj::WorkspaceVisibility {
            names: vec![SYNC_FALLBACK_WORKSPACE.to_owned()],
            includes_current: true,
        }
    } else {
        visibility.clone()
    }
}

fn sync_rebased_change_entry(commit: &crate::jj::RebasedCommitSummary) -> SyncChangeEntry<'_> {
    SyncChangeEntry {
        commit: commit.short_change_id.as_str(),
        description: commit.description.as_str(),
        visibility: sync_display_visibility(&commit.workspace_visibility),
        pull_request: None,
        conflicted: commit.has_conflict,
    }
}

fn sync_pushed_change_entry<'a>(
    bookmark: &'a crate::jj::PushedBookmarkSummary,
    pull_request: Option<&'a PullRequestRecord>,
    visibility: crate::jj::WorkspaceVisibility,
    fallback_description: Option<&'a str>,
) -> Option<SyncChangeEntry<'a>> {
    Some(SyncChangeEntry {
        commit: bookmark.new_short_change_id.as_deref()?,
        description: pushed_bookmark_description(bookmark, pull_request, fallback_description),
        visibility: sync_display_visibility(&visibility),
        pull_request,
        conflicted: false,
    })
}

fn sync_rebased_pushed_visibility(
    rebased: &crate::jj::RebasedCommitSummary,
    pushed: &crate::jj::PushedBookmarkSummary,
) -> crate::jj::WorkspaceVisibility {
    if rebased.workspace_visibility.names.is_empty() {
        pushed.new_workspace_visibility.clone()
    } else {
        rebased.workspace_visibility.clone()
    }
}

fn pushed_bookmark_description<'a>(
    bookmark: &'a crate::jj::PushedBookmarkSummary,
    pull_request: Option<&'a PullRequestRecord>,
    fallback: Option<&'a str>,
) -> &'a str {
    pull_request
        .map(|pull_request| pull_request.title.trim())
        .filter(|title| !title.is_empty())
        .or_else(|| {
            bookmark
                .new_description
                .as_deref()
                .filter(|title| !title.is_empty())
        })
        .or_else(|| fallback.filter(|title| !title.is_empty()))
        .unwrap_or("(no description)")
}

fn write_sync_change_section(
    formatter: &mut dyn Formatter,
    title: &str,
    entries: &[SyncChangeEntry<'_>],
    repository_url: &str,
) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    write_sync_section_separator(formatter)?;
    writeln!(formatter, "{title}")?;
    let rows = sync_workspace_rows(entries.iter().collect(), |entry| &entry.visibility);
    write_workspace_grouped_rows(formatter, &rows, |formatter, row| {
        write_sync_change_entry(formatter, row.item, repository_url)
    })
}

fn write_workspace_grouped_rows<T>(
    formatter: &mut dyn Formatter,
    rows: &[SyncWorkspaceRow<'_, T>],
    mut write_row: impl FnMut(&mut dyn Formatter, &SyncWorkspaceRow<'_, T>) -> io::Result<()>,
) -> io::Result<()> {
    let mut index = 0;
    let mut group_index = 0;
    while index < rows.len() {
        if group_index > 0 {
            writeln!(formatter)?;
        }

        let workspace = rows[index].workspace;
        write_sync_workspace_group_heading(formatter, workspace)?;
        while index < rows.len() && rows[index].workspace == workspace {
            write_row(formatter, &rows[index])?;
            index += 1;
        }
        group_index += 1;
    }
    Ok(())
}

fn write_sync_workspace_group_heading(
    formatter: &mut dyn Formatter,
    workspace: Option<&str>,
) -> io::Result<()> {
    write!(formatter, "  ")?;
    write_workspace_label(formatter, workspace.unwrap_or(SYNC_FALLBACK_WORKSPACE))?;
    writeln!(formatter)
}

fn write_sync_change_entry(
    formatter: &mut dyn Formatter,
    entry: &SyncChangeEntry<'_>,
    repository_url: &str,
) -> io::Result<()> {
    let pull_request_url = entry
        .pull_request
        .map(|pull_request| pull_request_url(repository_url, pull_request));
    let style = match (entry.conflicted, pull_request_url.as_deref()) {
        (true, Some(url)) => SyncCommitStyle::ConflictedPullRequest { url },
        (true, None) => SyncCommitStyle::Conflicted,
        (false, Some(url)) => SyncCommitStyle::PullRequest { url },
        (false, None) => SyncCommitStyle::Plain,
    };

    write!(formatter, "    ")?;
    write_sync_commit_cell(formatter, entry.commit, SYNC_COMMIT_WIDTH, style)?;
    write!(formatter, "  ")?;
    write_description(formatter, entry.description)?;
    writeln!(formatter)
}

#[derive(Debug, Clone, Copy)]
enum SyncCommitStyle<'a> {
    Plain,
    PullRequest { url: &'a str },
    Conflicted,
    ConflictedPullRequest { url: &'a str },
}

impl<'a> SyncCommitStyle<'a> {
    fn labels(self) -> &'static [&'static str] {
        match self {
            Self::Plain => &["commit_id"],
            Self::PullRequest { .. } => &["commit_id", "link", "pull_request_commit"],
            Self::Conflicted => &["commit_id", "conflict", "conflicted_commit"],
            Self::ConflictedPullRequest { .. } => &[
                "commit_id",
                "conflict",
                "link",
                "conflicted_pull_request_commit",
            ],
        }
    }

    fn url(self) -> Option<&'a str> {
        match self {
            Self::Plain | Self::Conflicted => None,
            Self::PullRequest { url } | Self::ConflictedPullRequest { url } => Some(url),
        }
    }
}

fn write_sync_commit_cell(
    formatter: &mut dyn Formatter,
    commit: &str,
    width: usize,
    style: SyncCommitStyle<'_>,
) -> io::Result<()> {
    if let Some(url) = style.url() {
        write_osc8_start(formatter, url)?;
    }
    write_labeled_text(formatter, style.labels(), commit)?;
    if style.url().is_some() {
        write_osc8_end(formatter)?;
    }
    write!(
        formatter,
        "{:padding$}",
        "",
        padding = width.saturating_sub(commit.chars().count())
    )
}

pub(in crate::commands) fn visible_rebased_commits(
    outcome: &FetchOutcome,
) -> Vec<&crate::jj::RebasedCommitSummary> {
    outcome
        .rebased_commits
        .iter()
        .filter(|commit| !is_uninformative_rebased_commit(commit))
        .collect()
}

pub(in crate::commands) fn write_sync_section_separator(
    formatter: &mut dyn Formatter,
) -> io::Result<()> {
    writeln!(formatter)
}

pub(in crate::commands) fn write_osc8_link(
    formatter: &mut dyn Formatter,
    uri: &str,
    label: &str,
) -> io::Result<()> {
    write_osc8_start(formatter, uri)?;
    write_labeled_text(formatter, &["link"], label)?;
    write_osc8_end(formatter)
}

pub(in crate::commands) fn write_osc8_start(
    formatter: &mut dyn Formatter,
    uri: &str,
) -> io::Result<()> {
    write!(formatter.raw()?, "\x1b]8;;{uri}\x1b\\")
}

pub(in crate::commands) fn write_osc8_end(formatter: &mut dyn Formatter) -> io::Result<()> {
    write!(formatter.raw()?, "\x1b]8;;\x1b\\")
}

pub(in crate::commands) fn is_uninformative_rebased_commit(
    commit: &crate::jj::RebasedCommitSummary,
) -> bool {
    commit.is_empty && commit.description == "(no description)" && !commit.has_conflict
}

pub(in crate::commands) struct SyncWorkspaceRow<'a, T> {
    item: &'a T,
    workspace: Option<&'a str>,
    is_current_workspace: bool,
    original_index: usize,
}

pub(in crate::commands) fn sync_workspace_rows<'a, T>(
    items: Vec<&'a T>,
    visibility: impl Fn(&'a T) -> &'a crate::jj::WorkspaceVisibility,
) -> Vec<SyncWorkspaceRow<'a, T>> {
    let mut rows = Vec::new();
    for (original_index, item) in items.into_iter().enumerate() {
        let visibility = visibility(item);
        if visibility.names.is_empty() {
            rows.push(SyncWorkspaceRow {
                item,
                workspace: None,
                is_current_workspace: false,
                original_index,
            });
            continue;
        }

        for (workspace_index, workspace) in visibility.names.iter().enumerate() {
            rows.push(SyncWorkspaceRow {
                item,
                workspace: Some(workspace),
                is_current_workspace: visibility.includes_current && workspace_index == 0,
                original_index,
            });
        }
    }

    rows.sort_by(|left, right| {
        sync_workspace_row_sort_key(left).cmp(&sync_workspace_row_sort_key(right))
    });
    rows
}

pub(in crate::commands) fn sync_workspace_row_sort_key<'a, T>(
    row: &SyncWorkspaceRow<'a, T>,
) -> (u8, &'a str, usize) {
    match row.workspace {
        Some(_) if row.is_current_workspace => (0, "", row.original_index),
        Some(workspace) => (1, workspace, row.original_index),
        None => (2, "", row.original_index),
    }
}

pub(in crate::commands) fn sync_workspace_row_label_width<T>(
    rows: &[SyncWorkspaceRow<'_, T>],
) -> usize {
    rows.iter()
        .filter_map(|row| row.workspace.map(|workspace| workspace.chars().count() + 1))
        .max()
        .unwrap_or(0)
}

pub(in crate::commands) fn write_rebased_commit(
    formatter: &mut dyn Formatter,
    row: &SyncWorkspaceRow<'_, crate::jj::RebasedCommitSummary>,
    workspace_width: usize,
) -> io::Result<()> {
    let commit = row.item;
    write_workspace_prefix(formatter, row.workspace, workspace_width)?;
    write_commit_id(formatter, &commit.short_change_id)?;
    write!(formatter, "  ")?;
    write_description(formatter, &commit.description)?;
    if commit.has_conflict {
        write_labeled_text(formatter, &["conflict"], " (conflicted)")?;
    }
    writeln!(formatter)
}

pub(in crate::commands) fn pull_request_url(
    repository_url: &str,
    pull_request: &PullRequestRecord,
) -> String {
    pull_request
        .html_url
        .clone()
        .unwrap_or_else(|| format!("{repository_url}/pull/{}", pull_request.number))
}

pub(in crate::commands) fn write_workspace_prefix(
    formatter: &mut dyn Formatter,
    workspace: Option<&str>,
    workspace_width: usize,
) -> io::Result<()> {
    write!(formatter, "  ")?;
    match workspace {
        Some(workspace) => {
            write_workspace_label(formatter, workspace)?;
            write!(
                formatter,
                "{:padding$}  ",
                "",
                padding = workspace_width.saturating_sub(workspace.chars().count() + 1)
            )
        }
        None if workspace_width > 0 => write!(formatter, "{:workspace_width$}  ", ""),
        None => Ok(()),
    }
}

pub(in crate::commands) fn write_workspace_label(
    formatter: &mut dyn Formatter,
    workspace: &str,
) -> io::Result<()> {
    write_labeled_text(formatter, &["working_copies"], &format!("{workspace}@"))
}

pub(in crate::commands) fn write_bookmark(
    formatter: &mut dyn Formatter,
    repository_url: &str,
    branch: &str,
) -> io::Result<()> {
    write_osc8_start(
        formatter,
        &bookmark_pull_request_url(repository_url, branch),
    )?;
    write_labeled_text(formatter, &["bookmark", "bookmark_synced", "link"], branch)?;
    write_osc8_end(formatter)
}

pub(in crate::commands) fn write_branch(
    formatter: &mut dyn Formatter,
    repository_url: &str,
    branch: &str,
) -> io::Result<()> {
    write_osc8_start(formatter, &branch_url(repository_url, branch))?;
    write_labeled_text(formatter, &["bookmark", "bookmark_synced", "link"], branch)?;
    write_osc8_end(formatter)
}

pub(in crate::commands) fn write_commit_id(
    formatter: &mut dyn Formatter,
    commit: &str,
) -> io::Result<()> {
    write_labeled_text(formatter, &["commit_id"], commit)
}

pub(in crate::commands) fn write_description(
    formatter: &mut dyn Formatter,
    summary: &str,
) -> io::Result<()> {
    let summary = display_summary(summary);
    if is_description_placeholder(summary) {
        write_labeled_text(formatter, &["description", "placeholder"], summary)
    } else {
        write_labeled_text(formatter, &["description"], summary)
    }
}

pub(in crate::commands) fn write_labeled_text(
    formatter: &mut dyn Formatter,
    labels: &[&str],
    text: &str,
) -> io::Result<()> {
    for label in labels {
        formatter.push_label(label);
    }
    let result = write!(formatter, "{text}");
    for _ in labels {
        formatter.pop_label();
    }
    result
}

pub(in crate::commands) fn is_description_placeholder(summary: &str) -> bool {
    summary.trim().is_empty() || summary == "(no description)"
}

pub(in crate::commands) fn write_pushed_bookmark(
    formatter: &mut dyn Formatter,
    bookmark: &crate::jj::PushedBookmarkSummary,
    repository_url: &str,
) -> io::Result<()> {
    write!(formatter, "  ")?;
    write_bookmark(formatter, repository_url, &bookmark.branch)?;
    match (&bookmark.old_short_commit_id, &bookmark.new_short_commit_id) {
        (Some(old), Some(new)) => writeln!(formatter, ": {old} -> {new}"),
        (None, Some(new)) => writeln!(formatter, ": created at {new}"),
        (Some(old), None) => writeln!(formatter, ": deleted from {old}"),
        (None, None) => writeln!(formatter, ": unchanged"),
    }
}

pub(in crate::commands) fn display_summary(summary: &str) -> &str {
    if summary.trim().is_empty() {
        "(no description)"
    } else {
        summary
    }
}
