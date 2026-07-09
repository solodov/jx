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

pub(in crate::commands) fn render_work_list(workspaces: &[WorkspaceEntry], color: bool) -> String {
    render_keyed_paths(
        workspaces
            .iter()
            .map(|workspace| workspace_path_row(workspace, color)),
    )
}

pub(in crate::commands) fn render_global_work_list(locations: &[WorkLocation]) -> String {
    render_keyed_paths(
        locations
            .iter()
            .map(|location| keyed_path_row(location.key.clone(), location.root.as_path())),
    )
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
