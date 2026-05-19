use super::*;

pub(super) fn render_linked_output(
    current_dir: &Path,
    color: bool,
    render: impl FnOnce(&mut dyn Formatter) -> io::Result<()>,
) -> Result<String, JjError> {
    if color {
        JjWorkspace::render_workspace_formatted_output(current_dir, render)
    } else {
        Ok(render_plain_output(render))
    }
}

fn render_plain_output(render: impl FnOnce(&mut dyn Formatter) -> io::Result<()>) -> String {
    let mut output = Vec::new();
    let mut formatter = PlainTextFormatter::new(&mut output);
    render(&mut formatter).expect("writing command output to a string cannot fail");
    String::from_utf8(output).expect("command output is UTF-8")
}

pub(super) fn render_check(
    report: &CheckReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_check(formatter, report)
    })
}

pub(super) fn write_check(formatter: &mut dyn Formatter, report: &CheckReport) -> io::Result<()> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GlobalFetchEntry {
    pub(super) root: PathBuf,
    pub(super) display_root: String,
    pub(super) result: Result<(), String>,
}

pub(super) fn render_fetch(
    report: &FetchReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_fetch(formatter, report)
    })
}

pub(super) fn render_global_fetch(
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
pub(super) struct GlobalSyncEntry {
    pub(super) root: PathBuf,
    pub(super) display_root: String,
    pub(super) outcome: GlobalSyncOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GlobalSyncOutcome {
    Synced,
    Skipped(GlobalSyncSkipReason),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GlobalSyncSkipReason {
    UpToDate,
    PullNeeded { commits: i64 },
    Diverged { pull: i64, push: i64 },
    LocalWork { changes: i64 },
    ReadOnlyOrigin,
    PushAccessUnavailable(String),
    SetupNeeded(String),
}

pub(super) fn render_global_sync(
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

pub(super) fn write_fetch(formatter: &mut dyn Formatter, report: &FetchReport) -> io::Result<()> {
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

pub(super) fn render_rebase_on_trunk(
    report: &RebaseOnTrunkReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_rebase_on_trunk(formatter, report)
    })
}

pub(super) fn write_rebase_on_trunk(
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

pub(super) fn render_pull_request(report: &PullRequestReport) -> String {
    match &report.pull_request.html_url {
        Some(url) => format!("{} {url}\n", pull_request_action(report.action)),
        None => format!(
            "{} PR #{}\n",
            pull_request_action(report.action),
            report.pull_request.number
        ),
    }
}

pub(super) fn render_clone(plan: &ClonePlan, destination: &str) -> String {
    format!("Cloned {} to {destination}\n", clone_link(plan))
}

pub(super) fn clone_link(plan: &ClonePlan) -> String {
    osc8_link(&clone_web_url(plan), &plan.remote_url)
}

fn clone_web_url(plan: &ClonePlan) -> String {
    format!(
        "https://{}/{}/{}",
        plan.identity.host, plan.identity.owner, plan.identity.repo
    )
}

pub(super) fn render_work_add(plan: &WorkAddPlan) -> String {
    format!("Added workspace: {}\n", plan.destination.display())
}

pub(super) fn render_work_list(workspaces: &[WorkspaceEntry]) -> String {
    let labels = workspaces.iter().map(workspace_label).collect::<Vec<_>>();
    render_keyed_paths(
        labels
            .into_iter()
            .zip(workspaces.iter().map(|workspace| &workspace.root)),
    )
}

pub(super) fn render_global_work_list(locations: &[WorkLocation]) -> String {
    render_keyed_paths(
        locations
            .iter()
            .map(|location| (location.key.clone(), &location.root)),
    )
}

pub(super) fn render_work_complete(locations: &[WorkLocation]) -> String {
    locations
        .iter()
        .map(|location| format!("{}\n", location.key))
        .collect()
}

pub(super) fn render_work_repository_complete(repositories: &[WorkRepository]) -> String {
    repositories
        .iter()
        .map(|repository| format!("{}\n", repository.key))
        .collect()
}

pub(super) fn render_workspace_name_complete(workspaces: &[WorkspaceEntry]) -> String {
    workspaces
        .iter()
        .map(|workspace| format!("{}\n", workspace.name))
        .collect()
}

pub(super) fn render_work_root(root: &Path) -> String {
    format!("{}\n", root.display())
}

pub(super) fn render_work_delete(workspace: &WorkspaceEntry) -> String {
    format!("Deleted workspace: {}\n", workspace.name)
}

fn render_keyed_paths<'a>(rows: impl IntoIterator<Item = (String, &'a PathBuf)>) -> String {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    let mut output = String::new();
    for (label, path) in rows {
        output.push_str(&format!(
            "{label:<width$}  {}\n",
            path.display(),
            width = width
        ));
    }
    output
}

fn workspace_label(workspace: &WorkspaceEntry) -> String {
    if workspace.is_current {
        format!("{}@", workspace.name)
    } else {
        workspace.name.clone()
    }
}

pub(super) fn render_push(
    report: &PushReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_push(formatter, report)
    })
}

pub(super) fn write_push(formatter: &mut dyn Formatter, report: &PushReport) -> io::Result<()> {
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

pub(super) fn render_tracked_push(
    report: &TrackedPushReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_tracked_push(formatter, report)
    })
}

pub(super) fn write_tracked_push(
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

pub(super) fn render_sync(
    report: &SyncReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_sync(report, formatter)
    })
}

pub(super) fn render_repository_bootstrap(
    report: &RepositoryBootstrapReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_repository_bootstrap(report, formatter)
    })
}

pub(super) fn write_repository_bootstrap(
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

pub(super) fn write_sync(report: &SyncReport, formatter: &mut dyn Formatter) -> io::Result<()> {
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
        .filter(|bookmark| bookmark.new_short_commit_id.is_some())
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

    Ok(())
}

pub(super) fn write_rebased_section(
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

pub(super) fn visible_rebased_commits(
    outcome: &FetchOutcome,
) -> Vec<&crate::jj::RebasedCommitSummary> {
    outcome
        .rebased_commits
        .iter()
        .filter(|commit| !is_uninformative_rebased_commit(commit))
        .collect()
}

pub(super) fn write_sync_section_separator(formatter: &mut dyn Formatter) -> io::Result<()> {
    writeln!(formatter)
}

pub(super) fn write_osc8_link(
    formatter: &mut dyn Formatter,
    uri: &str,
    label: &str,
) -> io::Result<()> {
    write_osc8_start(formatter, uri)?;
    write_labeled_text(formatter, &["link"], label)?;
    write_osc8_end(formatter)
}

pub(super) fn write_osc8_start(formatter: &mut dyn Formatter, uri: &str) -> io::Result<()> {
    write!(formatter.raw()?, "\x1b]8;;{uri}\x1b\\")
}

pub(super) fn write_osc8_end(formatter: &mut dyn Formatter) -> io::Result<()> {
    write!(formatter.raw()?, "\x1b]8;;\x1b\\")
}

pub(super) fn is_uninformative_rebased_commit(commit: &crate::jj::RebasedCommitSummary) -> bool {
    commit.is_empty && commit.description == "(no description)" && !commit.has_conflict
}

pub(super) struct SyncWorkspaceRow<'a, T> {
    item: &'a T,
    workspace: Option<&'a str>,
    is_current_workspace: bool,
    original_index: usize,
}

pub(super) fn sync_workspace_rows<'a, T>(
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

pub(super) fn sync_workspace_row_sort_key<'a, T>(
    row: &SyncWorkspaceRow<'a, T>,
) -> (u8, &'a str, usize) {
    match row.workspace {
        Some(_) if row.is_current_workspace => (0, "", row.original_index),
        Some(workspace) => (1, workspace, row.original_index),
        None => (2, "", row.original_index),
    }
}

pub(super) fn sync_workspace_row_label_width<T>(rows: &[SyncWorkspaceRow<'_, T>]) -> usize {
    rows.iter()
        .filter_map(|row| row.workspace.map(|workspace| workspace.chars().count() + 1))
        .max()
        .unwrap_or(0)
}

pub(super) fn write_rebased_commit(
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

pub(super) fn write_pushed_bookmark_commit(
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

pub(super) fn write_deleted_bookmark(
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

pub(super) fn pushed_pull_request_annotation_indent(
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

pub(super) fn write_pull_request_annotation(
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

pub(super) fn pull_request_url(repository_url: &str, pull_request: &PullRequestRecord) -> String {
    pull_request
        .html_url
        .clone()
        .unwrap_or_else(|| format!("{repository_url}/pull/{}", pull_request.number))
}

pub(super) fn write_bookmark_commit_tail(
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

pub(super) fn write_workspace_prefix(
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

pub(super) fn write_workspace_label(
    formatter: &mut dyn Formatter,
    workspace: &str,
) -> io::Result<()> {
    write_labeled_text(formatter, &["working_copies"], &format!("{workspace}@"))
}

pub(super) fn write_bookmark_target(
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

pub(super) fn write_bookmark(
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

pub(super) fn write_branch(
    formatter: &mut dyn Formatter,
    repository_url: &str,
    branch: &str,
) -> io::Result<()> {
    write_osc8_start(formatter, &branch_url(repository_url, branch))?;
    write_labeled_text(formatter, &["bookmark", "bookmark_synced", "link"], branch)?;
    write_osc8_end(formatter)
}

pub(super) fn write_commit_id(formatter: &mut dyn Formatter, commit: &str) -> io::Result<()> {
    write_labeled_text(formatter, &["commit_id"], commit)
}

pub(super) fn write_description(formatter: &mut dyn Formatter, summary: &str) -> io::Result<()> {
    let summary = display_summary(summary);
    if is_description_placeholder(summary) {
        write_labeled_text(formatter, &["description", "placeholder"], summary)
    } else {
        write_labeled_text(formatter, &["description"], summary)
    }
}

pub(super) fn write_labeled_text(
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

pub(super) fn is_description_placeholder(summary: &str) -> bool {
    summary.trim().is_empty() || summary == "(no description)"
}

pub(super) fn write_pushed_bookmark(
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

pub(super) fn display_summary(summary: &str) -> &str {
    if summary.trim().is_empty() {
        "(no description)"
    } else {
        summary
    }
}

/// Renders the shared current commit status block shown by `jx status` and `jx pr`.
pub(super) fn render_workspace_status(status: &WorkspaceStatus) -> String {
    render_workspace_status_with_width(status, termimad::terminal_size().0.into())
}

pub(super) fn render_workspace_status_with_width(status: &WorkspaceStatus, width: usize) -> String {
    let mut lines = Vec::new();
    lines.extend(status.commit_lines.iter().cloned());

    let description = status.description.trim_end();
    if !description.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(
            render_status_description(description, width)
                .lines()
                .map(str::to_owned),
        );
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

pub(super) fn render_status_description(description: &str, width: usize) -> String {
    MadSkin::default()
        .text(description, Some(width.max(20)))
        .to_string()
}

/// Renders the shared status block plus PR-only metadata before any publishing mutation.
pub(super) fn render_pull_request_preview(
    plan: &PullRequestPlan,
    status: &WorkspaceStatus,
) -> String {
    let mut preview = render_workspace_status(status);

    if !plan.labels.is_empty() {
        preview.push('\n');
        preview.push_str(&format!("Labels: {}\n", plan.labels.join(", ")));
    }

    preview
}

/// Builds the final confirmation prompt from planned create/update and draft state.
pub(super) fn pull_request_confirmation_prompt(plan: &PullRequestPlan) -> String {
    let (verb, draft) = match &plan.existing_pull_request {
        Some(existing) => ("Update", existing.draft),
        None => ("Create", plan.draft),
    };
    if draft {
        format!("{verb} draft pull request?")
    } else {
        format!("{verb} pull request?")
    }
}

/// Builds the confirmation prompt for creating an otherwise missing push bookmark.
pub(super) fn push_confirmation_prompt(plan: &PushPlan) -> String {
    format!(
        "Create bookmark `{}` at {} and push?",
        plan.bookmark.branch, plan.target_short_commit_id
    )
}

/// Builds the confirmation prompt before forgetting and deleting a managed workspace.
pub(super) fn workspace_remove_confirmation_prompt(workspace: &WorkspaceEntry) -> String {
    format!(
        "Delete workspace `{}` at {}?",
        workspace.name,
        workspace.root.display()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GlobalStatusEntry {
    pub(super) key: Option<String>,
    pub(super) root: PathBuf,
    pub(super) display_root: String,
    pub(super) repository: Option<GitHubRepository>,
    pub(super) result: Result<StatusReport, String>,
}

pub(super) fn render_status(
    report: &StatusReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_status(formatter, report)
    })
}

pub(super) fn render_status_json(entries: &[GlobalStatusEntry]) -> String {
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
        }
    }
}

pub(super) fn render_global_status(
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

pub(super) fn write_status(formatter: &mut dyn Formatter, report: &StatusReport) -> io::Result<()> {
    for remote in sorted_remote_statuses(report) {
        write_remote_status(formatter, remote)?;
    }
    if let Some(fork) = &report.fork {
        write_fork_status(formatter, fork)?;
    }
    Ok(())
}

pub(super) fn bookmark_action_summary(action: BookmarkAction) -> &'static str {
    match action {
        BookmarkAction::Create => "will create",
        BookmarkAction::Reuse => "exists",
    }
}

pub(super) fn pull_request_action(action: PullRequestAction) -> &'static str {
    match action {
        PullRequestAction::Created => "Created",
        PullRequestAction::Updated => "Updated",
    }
}

pub(super) fn rebase_on_trunk_source_label(outcome: &RebaseOnTrunkOutcome) -> String {
    match outcome.source_short_commit_ids.as_slice() {
        [source] => source.clone(),
        sources => format!("{} sources", sources.len()),
    }
}

pub(super) fn render_rebase_on_trunk_outcome(outcome: &RebaseOnTrunkOutcome) -> String {
    if outcome.rebased_commits == 0 {
        "up to date".to_owned()
    } else {
        format!("rebased {}", commit_count(outcome.rebased_commits))
    }
}

pub(super) fn commit_count(count: usize) -> String {
    let noun = if count == 1 { "commit" } else { "commits" };
    format!("{count} {noun}")
}

pub(super) fn write_remote_status(
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

pub(super) fn render_status_delta(remote: &domain::RemoteStatusReport) -> String {
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

pub(super) fn write_fork_status(
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

pub(super) fn render_fork_status_delta(fork: &domain::ForkStatusReport) -> String {
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

fn commit_count_i64(count: i64) -> String {
    let noun = if count == 1 { "commit" } else { "commits" };
    format!("{count} {noun}")
}

fn local_change_count_i64(count: i64) -> String {
    let noun = if count == 1 {
        "local change"
    } else {
        "local changes"
    };
    format!("{count} {noun}")
}

fn new_commit_count(count: i64) -> String {
    let noun = if count == 1 {
        "new commit"
    } else {
        "new commits"
    };
    format!("{count} {noun}")
}

fn unpublished_commit_count(count: i64) -> String {
    let noun = if count == 1 {
        "unpublished commit"
    } else {
        "unpublished commits"
    };
    format!("{count} {noun}")
}

pub(super) fn branch_url(repository_url: &str, branch: &str) -> String {
    format!("{repository_url}/tree/{branch}")
}

pub(super) fn bookmark_pull_request_url(repository_url: &str, bookmark: &str) -> String {
    let query = url_query_encode(&format!("is:pr head:{bookmark}"));
    format!("{repository_url}/pulls?q={query}")
}

#[cfg(test)]
pub(super) fn linked_bookmark_text(repository_url: &str, bookmark: &str) -> String {
    osc8_link(
        &bookmark_pull_request_url(repository_url, bookmark),
        bookmark,
    )
}

pub(super) fn url_query_encode(value: &str) -> String {
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

pub(super) fn osc8_link(uri: &str, label: &str) -> String {
    format!("\x1b]8;;{uri}\x1b\\{label}\x1b]8;;\x1b\\")
}
