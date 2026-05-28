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
    ReadOnlyOrigin,
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
        write_global_sync_path_section(
            formatter,
            &mut wrote_any,
            "Skipped: read-only origin",
            sorted_entries
                .iter()
                .filter_map(|entry| match &entry.outcome {
                    GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::ReadOnlyOrigin) => {
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

pub(in crate::commands) fn render_rebase_on_trunk(
    report: &RebaseOnTrunkReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_rebase_on_trunk(formatter, report)
    })
}

pub(in crate::commands) fn write_rebase_on_trunk(
    formatter: &mut dyn Formatter,
    report: &RebaseOnTrunkReport,
) -> io::Result<()> {
    write!(
        formatter,
        "Rebased: {} onto {}/{} (",
        rebase_on_trunk_source_label(&report.outcome),
        report.repository.origin_name,
        report.outcome.branch
    )?;
    write_osc8_link(
        formatter,
        &branch_url(&report.repository.github_url, &report.outcome.branch),
        &report.repository.origin_url,
    )?;
    writeln!(
        formatter,
        "), {}",
        render_rebase_on_trunk_outcome(&report.outcome)
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
    let deleted_bookmarks = report
        .push
        .bookmarks
        .iter()
        .filter(|bookmark| bookmark.new_short_commit_id.is_none())
        .collect::<Vec<_>>();
    let pull_requests = report
        .pull_requests
        .iter()
        .map(|pull_request| (pull_request.head_branch.as_str(), pull_request))
        .collect::<BTreeMap<_, _>>();

    write_rebased_section(
        formatter,
        report.repository.origin_name,
        &report.fetch.branch,
        rebased_commits,
    )?;

    if !pushed_bookmarks.is_empty() {
        write_sync_section_separator(formatter)?;
        writeln!(formatter, "Pushed commits:")?;
        let mut rows = sync_workspace_rows(pushed_bookmarks, |bookmark| {
            &bookmark.new_workspace_visibility
        });
        let workspace_width = sync_workspace_row_label_width(&rows);
        let pushed_bookmark_width = rows
            .iter()
            .map(|row| row.item.branch.chars().count())
            .max()
            .unwrap_or(0);
        let mut annotated_bookmarks = BTreeSet::new();
        for row in &mut rows {
            let pull_request = annotated_bookmarks
                .insert(row.item.branch.as_str())
                .then(|| pull_requests.get(row.item.branch.as_str()).copied())
                .flatten();
            write_pushed_bookmark_commit(
                formatter,
                row,
                pushed_bookmark_width,
                workspace_width,
                &report.repository.github_url,
                pull_request,
            )?;
        }
    }

    if !deleted_bookmarks.is_empty() {
        write_sync_section_separator(formatter)?;
        writeln!(formatter, "Deleted bookmarks:")?;
        for bookmark in deleted_bookmarks {
            write_deleted_bookmark(
                formatter,
                bookmark,
                &report.repository.github_url,
                pull_requests.get(bookmark.branch.as_str()).copied(),
            )?;
        }
    }

    if !report.skipped_conflicted_bookmarks.is_empty() {
        write_sync_section_separator(formatter)?;
        write_skipped_conflicted_bookmarks(
            formatter,
            &report.skipped_conflicted_bookmarks,
            &report.repository.github_url,
        )?;
    }

    Ok(())
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
    write_commit_id(formatter, &commit.old_short_commit_id)?;
    write!(formatter, " -> ")?;
    write_commit_id(formatter, &commit.new_short_commit_id)?;
    write!(formatter, "  ")?;
    write_description(formatter, &commit.description)?;
    if commit.has_conflict {
        write_labeled_text(formatter, &["conflict"], " (conflicted)")?;
    }
    writeln!(formatter)
}

pub(in crate::commands) fn write_skipped_conflicted_bookmarks(
    formatter: &mut dyn Formatter,
    bookmarks: &[crate::jj::SkippedPushBookmarkSummary],
    repository_url: &str,
) -> io::Result<()> {
    writeln!(formatter, "Skipped bookmarks with conflicts:")?;
    let bookmark_width = bookmarks
        .iter()
        .map(|bookmark| bookmark.branch.chars().count())
        .max()
        .unwrap_or(0);

    for bookmark in bookmarks {
        for (index, commit) in bookmark.conflicted_commits.iter().enumerate() {
            write!(formatter, "  ")?;
            if index == 0 {
                write_bookmark_target(formatter, repository_url, &bookmark.branch, bookmark_width)?;
            } else {
                write!(formatter, "{:bookmark_width$}", "")?;
            }
            write!(formatter, "  ")?;
            write_commit_id(formatter, &commit.short_commit_id)?;
            write!(formatter, "  ")?;
            write_description(formatter, &commit.description)?;
            write_labeled_text(formatter, &["conflict"], " (conflicted)")?;
            writeln!(formatter)?;
        }
    }

    Ok(())
}

pub(in crate::commands) fn write_pushed_bookmark_commit(
    formatter: &mut dyn Formatter,
    row: &SyncWorkspaceRow<'_, crate::jj::PushedBookmarkSummary>,
    bookmark_width: usize,
    workspace_width: usize,
    repository_url: &str,
    pull_request: Option<&PullRequestRecord>,
) -> io::Result<()> {
    let bookmark = row.item;
    let commit = bookmark
        .new_short_commit_id
        .as_deref()
        .expect("caller filters pushed bookmarks");
    let description = bookmark
        .new_description
        .as_deref()
        .unwrap_or("(no description)");
    write_workspace_prefix(formatter, row.workspace, workspace_width)?;
    write_commit_id(formatter, commit)?;
    write!(formatter, " -> ")?;
    write_bookmark_target(formatter, repository_url, &bookmark.branch, bookmark_width)?;
    write!(formatter, "  ")?;
    write_description(formatter, description)?;
    writeln!(formatter)?;

    if let Some(pull_request) = pull_request {
        write_pull_request_annotation(
            formatter,
            pushed_pull_request_annotation_indent(row.workspace, workspace_width, commit),
            repository_url,
            pull_request,
        )?;
    }

    Ok(())
}

pub(in crate::commands) fn write_deleted_bookmark(
    formatter: &mut dyn Formatter,
    bookmark: &crate::jj::PushedBookmarkSummary,
    repository_url: &str,
    pull_request: Option<&PullRequestRecord>,
) -> io::Result<()> {
    let commit = bookmark
        .old_short_commit_id
        .as_deref()
        .expect("deleted tracked bookmarks have an old commit");
    let description = bookmark
        .old_description
        .as_deref()
        .unwrap_or("(no description)");
    write!(formatter, "  ")?;
    write_bookmark_commit_tail(
        formatter,
        repository_url,
        &bookmark.branch,
        commit,
        description,
    )?;

    if let Some(pull_request) = pull_request {
        write_pull_request_annotation(formatter, 2, repository_url, pull_request)?;
    }

    Ok(())
}

pub(in crate::commands) fn pushed_pull_request_annotation_indent(
    workspace: Option<&str>,
    workspace_width: usize,
    commit: &str,
) -> usize {
    let workspace_prefix_width = match workspace {
        Some(_) => workspace_width + 2,
        None if workspace_width > 0 => workspace_width + 2,
        None => 0,
    };
    2 + workspace_prefix_width + commit.chars().count() + " -> ".chars().count()
}

pub(in crate::commands) fn write_pull_request_annotation(
    formatter: &mut dyn Formatter,
    indent: usize,
    repository_url: &str,
    pull_request: &PullRequestRecord,
) -> io::Result<()> {
    write!(formatter, "{:indent$}↳ ", "")?;
    if pull_request.draft {
        write!(formatter, "draft ")?;
    }
    write!(formatter, "PR ")?;
    let label = format!("#{}", pull_request.number);
    write_osc8_link(
        formatter,
        &pull_request_url(repository_url, pull_request),
        &label,
    )?;
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

pub(in crate::commands) fn write_bookmark_commit_tail(
    formatter: &mut dyn Formatter,
    repository_url: &str,
    branch: &str,
    commit: &str,
    description: &str,
) -> io::Result<()> {
    write_bookmark(formatter, repository_url, branch)?;
    write!(formatter, ": ")?;
    write_commit_id(formatter, commit)?;
    write!(formatter, " ")?;
    write_description(formatter, description)?;
    writeln!(formatter)
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

pub(in crate::commands) fn write_bookmark_target(
    formatter: &mut dyn Formatter,
    repository_url: &str,
    branch: &str,
    bookmark_width: usize,
) -> io::Result<()> {
    write_bookmark(formatter, repository_url, branch)?;
    write!(
        formatter,
        "{:padding$}",
        "",
        padding = bookmark_width.saturating_sub(branch.chars().count())
    )
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
