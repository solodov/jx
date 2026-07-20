use super::*;

pub(in crate::commands) fn render_clone(plan: &ClonePlan, destination: &str) -> String {
    format!("Cloned {} to {destination}\n", clone_link(plan))
}

pub(in crate::commands) fn clone_link(plan: &ClonePlan) -> String {
    osc8_link(&clone_web_url(plan), &plan.remote_url)
}

fn clone_web_url(plan: &ClonePlan) -> String {
    format!(
        "https://{}/{}/{}",
        plan.identity.host, plan.identity.owner, plan.identity.repo
    )
}

pub(in crate::commands) fn render_work_add(plan: &WorkAddPlan) -> String {
    format!("Added workspace: {}\n", plan.destination.display())
}

pub(in crate::commands) fn render_work_info(info: &WorkInfo, format: WorkInfoFormat) -> String {
    match format {
        WorkInfoFormat::Human => render_work_info_human(info),
        WorkInfoFormat::Json => render_work_info_json(info),
    }
}

fn render_work_info_human(info: &WorkInfo) -> String {
    let rows = [
        ("workspace", info.workspace.name.clone()),
        ("root", info.workspace.root.display().to_string()),
        (
            "repository",
            format!("{}/{}", info.identity.owner, info.identity.repo),
        ),
        (
            "repository root",
            info.repository_root.display().to_string(),
        ),
        (
            "task",
            info.metadata
                .task_id
                .as_deref()
                .unwrap_or("none")
                .to_owned(),
        ),
        (
            "project",
            info.metadata
                .project
                .as_deref()
                .unwrap_or("none")
                .to_owned(),
        ),
        (
            "parent",
            work_info_parent_label(info.metadata.parent.as_ref()),
        ),
    ];
    let width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    rows.iter()
        .map(|(label, value)| {
            let padding = " ".repeat(width - label.chars().count());
            format!("{label}{padding}  {value}\n")
        })
        .collect()
}

fn render_work_info_json(info: &WorkInfo) -> String {
    let output = WorkInfoJson {
        command: "work-info",
        version: 1,
        workspace: WorkInfoWorkspaceJson {
            name: &info.workspace.name,
            root: info.workspace.root.display().to_string(),
            current: info.workspace.is_current,
            repository_root: info.repository_root.display().to_string(),
        },
        repository: WorkInfoRepositoryJson {
            source: &info.identity.source,
            host: &info.identity.host,
            owner: &info.identity.owner,
            repo: &info.identity.repo,
            slug: format!("{}/{}", info.identity.owner, info.identity.repo),
        },
        metadata: WorkInfoMetadataJson {
            task_id: info.metadata.task_id.as_deref(),
            project: info.metadata.project.as_deref(),
            parent: info.metadata.parent.as_ref().map(WorkInfoParentJson::from),
        },
    };
    let mut rendered = serde_json::to_string_pretty(&output)
        .expect("work info JSON contains only serializable values");
    rendered.push('\n');
    rendered
}

#[derive(serde::Serialize)]
struct WorkInfoJson<'a> {
    command: &'static str,
    version: u8,
    workspace: WorkInfoWorkspaceJson<'a>,
    repository: WorkInfoRepositoryJson<'a>,
    metadata: WorkInfoMetadataJson<'a>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkInfoWorkspaceJson<'a> {
    name: &'a str,
    root: String,
    current: bool,
    repository_root: String,
}

#[derive(serde::Serialize)]
struct WorkInfoRepositoryJson<'a> {
    source: &'a str,
    host: &'a str,
    owner: &'a str,
    repo: &'a str,
    slug: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkInfoMetadataJson<'a> {
    task_id: Option<&'a str>,
    project: Option<&'a str>,
    parent: Option<WorkInfoParentJson<'a>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkInfoParentJson<'a> {
    workspace_name: &'a str,
    task_id: Option<&'a str>,
    project: Option<&'a str>,
}

impl<'a> From<&'a WorkspaceParentMetadata> for WorkInfoParentJson<'a> {
    fn from(parent: &'a WorkspaceParentMetadata) -> Self {
        Self {
            workspace_name: &parent.workspace_name,
            task_id: parent.task_id.as_deref(),
            project: parent.project.as_deref(),
        }
    }
}

fn work_info_parent_label(parent: Option<&WorkspaceParentMetadata>) -> String {
    let Some(parent) = parent else {
        return "none".to_owned();
    };
    match (&parent.task_id, &parent.project) {
        (Some(task_id), Some(project)) => {
            format!("{} ({project}, {task_id})", parent.workspace_name)
        }
        (Some(task_id), None) => format!("{} ({task_id})", parent.workspace_name),
        (None, Some(project)) => format!("{} ({project})", parent.workspace_name),
        (None, None) => parent.workspace_name.clone(),
    }
}

pub(in crate::commands) fn render_work_list(entries: &[WorkListEntry], color: bool) -> String {
    if entries.iter().all(|entry| entry.project.is_none()) {
        return render_keyed_paths(
            entries
                .iter()
                .map(|entry| workspace_path_row(&entry.workspace, color)),
        );
    }

    render_project_work_list(entries, color)
}

fn render_project_work_list(entries: &[WorkListEntry], color: bool) -> String {
    let mut output = String::new();
    let projects = entries
        .iter()
        .filter_map(|entry| entry.project.as_deref())
        .collect::<BTreeSet<_>>();

    for project in projects {
        append_project_work_list_group(
            &mut output,
            project,
            entries
                .iter()
                .filter(|entry| entry.project.as_deref() == Some(project)),
            color,
        );
    }

    let unprojected = entries.iter().filter(|entry| entry.project.is_none());
    if unprojected.clone().next().is_some() {
        append_project_work_list_group(&mut output, "No project", unprojected, color);
    }

    output
}

fn append_project_work_list_group<'a>(
    output: &mut String,
    title: &str,
    entries: impl Iterator<Item = &'a WorkListEntry>,
    color: bool,
) {
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(title);
    output.push('\n');
    output.push_str(&render_keyed_paths(
        entries.map(|entry| indented_workspace_path_row(&entry.workspace, color)),
    ));
}

fn indented_workspace_path_row(workspace: &WorkspaceEntry, color: bool) -> KeyedPathRow<'_> {
    let mut row = workspace_path_row(workspace, color);
    row.label = format!("  {}", row.label);
    row.visible_label_width += 2;
    row
}

pub(in crate::commands) fn render_global_work_list(entries: &[WorkLocationListEntry]) -> String {
    if entries.iter().all(|entry| entry.project.is_none()) {
        return render_keyed_paths(entries.iter().map(|entry| {
            keyed_path_row(entry.location.key.clone(), entry.location.root.as_path())
        }));
    }

    render_project_location_list(entries)
}

fn render_project_location_list(entries: &[WorkLocationListEntry]) -> String {
    let mut output = String::new();
    let projects = entries
        .iter()
        .filter_map(|entry| entry.project.as_deref())
        .collect::<BTreeSet<_>>();

    for project in projects {
        append_project_location_list_group(
            &mut output,
            project,
            entries
                .iter()
                .filter(|entry| entry.project.as_deref() == Some(project)),
        );
    }

    let unprojected = entries.iter().filter(|entry| entry.project.is_none());
    if unprojected.clone().next().is_some() {
        append_project_location_list_group(&mut output, "No project", unprojected);
    }

    output
}

fn append_project_location_list_group<'a>(
    output: &mut String,
    title: &str,
    entries: impl Iterator<Item = &'a WorkLocationListEntry>,
) {
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(title);
    output.push('\n');
    output.push_str(&render_keyed_paths(entries.map(|entry| {
        let mut row = keyed_path_row(entry.location.key.clone(), entry.location.root.as_path());
        row.label = format!("  {}", row.label);
        row.visible_label_width += 2;
        row
    })));
}

pub(in crate::commands) fn render_work_complete(
    locations: &[WorkLocation],
    format: WorkCompleteFormat,
) -> String {
    match format {
        WorkCompleteFormat::Simple => locations
            .iter()
            .map(|location| format!("{}\n", location.key))
            .collect(),
        WorkCompleteFormat::Picker => locations
            .iter()
            .map(|location| format!("{}\t{}\n", location.key, location.root.display()))
            .collect(),
    }
}

pub(in crate::commands) fn render_work_repository_complete(
    repositories: &[WorkRepository],
) -> String {
    repositories
        .iter()
        .map(|repository| format!("{}\n", repository.key))
        .collect()
}

pub(in crate::commands) fn render_workspace_name_complete(workspaces: &[WorkspaceEntry]) -> String {
    workspaces
        .iter()
        .map(|workspace| format!("{}\n", workspace.name))
        .collect()
}

pub(in crate::commands) fn render_work_root(root: &Path) -> String {
    format!("{}\n", root.display())
}

pub(in crate::commands) fn render_work_delete(workspace: &WorkspaceEntry) -> String {
    format!("Deleted workspace: {}\n", workspace.name)
}

struct KeyedPathRow<'a> {
    label: String,
    visible_label_width: usize,
    path: &'a Path,
}

fn render_keyed_paths<'a>(rows: impl IntoIterator<Item = KeyedPathRow<'a>>) -> String {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let width = rows
        .iter()
        .map(|row| row.visible_label_width)
        .max()
        .unwrap_or(0);
    let mut output = String::new();
    for row in rows {
        let padding = " ".repeat(width.saturating_sub(row.visible_label_width));
        output.push_str(&format!("{}{padding}  {}\n", row.label, row.path.display()));
    }
    output
}

fn workspace_path_row(workspace: &WorkspaceEntry, color: bool) -> KeyedPathRow<'_> {
    let label = if workspace.is_current {
        current_workspace_label(&workspace.name, color)
    } else {
        workspace.name.clone()
    };
    let visible_label_width = if workspace.is_current && !color {
        workspace.name.chars().count() + 1
    } else {
        workspace.name.chars().count()
    };

    KeyedPathRow {
        label,
        visible_label_width,
        path: workspace.root.as_path(),
    }
}

fn keyed_path_row(label: String, path: &Path) -> KeyedPathRow<'_> {
    let visible_label_width = label.chars().count();
    KeyedPathRow {
        label,
        visible_label_width,
        path,
    }
}

fn current_workspace_label(name: &str, color: bool) -> String {
    if color {
        format!("{CURRENT_WORKSPACE_STYLE}{name}{RESET_STYLE}")
    } else {
        format!("{name}@")
    }
}

const CURRENT_WORKSPACE_STYLE: &str = "\x1b[1m\x1b[32m";
const RESET_STYLE: &str = "\x1b[0m";
