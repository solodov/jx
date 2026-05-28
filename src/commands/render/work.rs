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

pub(in crate::commands) fn render_work_list(workspaces: &[WorkspaceEntry]) -> String {
    let labels = workspaces.iter().map(workspace_label).collect::<Vec<_>>();
    render_keyed_paths(
        labels
            .into_iter()
            .zip(workspaces.iter().map(|workspace| &workspace.root)),
    )
}

pub(in crate::commands) fn render_global_work_list(locations: &[WorkLocation]) -> String {
    render_keyed_paths(
        locations
            .iter()
            .map(|location| (location.key.clone(), &location.root)),
    )
}

pub(in crate::commands) fn render_work_complete(locations: &[WorkLocation]) -> String {
    locations
        .iter()
        .map(|location| format!("{}\n", location.key))
        .collect()
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
