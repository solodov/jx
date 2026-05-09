use super::*;

pub(super) fn render_linked_output(
    current_dir: &Path,
    color: bool,
    render: impl FnOnce(&mut dyn Formatter) -> io::Result<()>,
) -> Result<String, JjError> {
    if color {
        JjWorkspace::render_workspace_formatted_output(current_dir, render)
    } else {
        let mut output = Vec::new();
        let mut formatter = PlainTextFormatter::new(&mut output);
        render(&mut formatter).expect("writing command output to a string cannot fail");
        Ok(String::from_utf8(output).expect("command output is UTF-8"))
    }
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

pub(super) fn render_fetch(
    report: &FetchReport,
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        write_fetch(formatter, report)
    })
}

pub(super) fn write_fetch(formatter: &mut dyn Formatter, report: &FetchReport) -> io::Result<()> {
    write!(
        formatter,
        "Fetched: {}/{} (",
        report.repository.origin_name, report.outcome.branch
    )?;
    write_osc8_link(
        formatter,
        &branch_url(&report.repository.github_url, &report.outcome.branch),
        &report.repository.origin_url,
    )?;
    writeln!(formatter, ")")?;
    write_rebased_section(
        formatter,
        report.repository.origin_name,
        &report.outcome.branch,
        visible_rebased_commits(&report.outcome),
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

pub(super) fn render_clone(plan: &ClonePlan) -> String {
    format!("Cloned: {}\n", plan.destination.display())
}

pub(super) fn render_work_add(options: &WorkspaceAddOptions) -> String {
    format!("Added workspace: {}\n", options.destination.display())
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

pub(super) fn render_work_root(root: &Path) -> String {
    format!("{}\n", root.display())
}

pub(super) fn render_work_remove(workspace: &WorkspaceEntry) -> String {
    format!("Removed workspace: {}\n", workspace.name)
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
        "Remove workspace `{}` at {}?",
        workspace.name,
        workspace.root.display()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GlobalStatusEntry {
    pub(super) display_root: String,
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

pub(super) fn render_global_status(
    entries: &[GlobalStatusEntry],
    current_dir: &Path,
    color: bool,
) -> Result<String, JjError> {
    render_linked_output(current_dir, color, |formatter| {
        for entry in entries {
            match &entry.result {
                Ok(report) => write_status_with_prefix(formatter, report, &entry.display_root)?,
                Err(message) => writeln!(formatter, "{} error: {message}", entry.display_root)?,
            }
        }
        Ok(())
    })
}

pub(super) fn write_status(formatter: &mut dyn Formatter, report: &StatusReport) -> io::Result<()> {
    for remote in &report.remotes {
        write_remote_status(formatter, remote)?;
    }
    Ok(())
}

pub(super) fn write_status_with_prefix(
    formatter: &mut dyn Formatter,
    report: &StatusReport,
    prefix: &str,
) -> io::Result<()> {
    for remote in &report.remotes {
        write!(formatter, "{prefix} ")?;
        write_remote_status(formatter, remote)?;
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
    let ahead = remote.comparison.github_ahead_by;
    let behind = remote.comparison.github_behind_by + remote.local_ahead_by;
    let mut parts = Vec::new();

    if ahead > 0 {
        parts.push(commit_delta(ahead, "ahead"));
    }
    if behind > 0 {
        parts.push(commit_delta(behind, "behind"));
    }

    if parts.is_empty() {
        "up to date".to_owned()
    } else {
        parts.join(", ")
    }
}

pub(super) fn commit_delta(count: i64, direction: &str) -> String {
    let noun = if count == 1 { "commit" } else { "commits" };
    format!("{count} {noun} {direction}")
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

#[cfg(test)]
pub(super) fn osc8_link(uri: &str, label: &str) -> String {
    format!("\x1b]8;;{uri}\x1b\\{label}\x1b]8;;\x1b\\")
}
