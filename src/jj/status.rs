use super::*;

impl JjWorkspace {
    /// Loads a workspace after jj has captured pending disk changes.
    pub fn load_after_working_copy_snapshot(
        current_dir: impl AsRef<Path>,
    ) -> Result<Self, JjError> {
        let workspace_root = snapshot_working_copy_for_read(current_dir.as_ref())?;
        Self::load(workspace_root)
    }

    /// Snapshots pending disk changes and returns the current jj working-copy commit id.
    pub fn snapshot_working_copy(current_dir: &Path) -> Result<WorkingCopySnapshot, JjError> {
        let workspace = Self::load_after_working_copy_snapshot(current_dir)?;
        let commit = workspace.current_commit()?;

        Ok(WorkingCopySnapshot {
            commit_id: commit.id().hex(),
        })
    }

    /// Refreshes this wrapper after jj has captured pending disk changes.
    pub(super) fn reload_after_working_copy_snapshot(&mut self) -> Result<(), JjError> {
        let workspace_root = self.workspace.workspace_root().to_path_buf();
        let refreshed = Self::load_after_working_copy_snapshot(&workspace_root)?;
        self.workspace = refreshed.workspace;
        self.repo = refreshed.repo;
        Ok(())
    }

    /// Returns status lines for the current working-copy commit using jj's own summary rendering.
    pub fn current_status(current_dir: &Path, color: bool) -> Result<WorkspaceStatus, JjError> {
        let workspace_root = find_jj_workspace_root(current_dir)?;
        let jj_status = run_jj_status(current_dir, color)?;
        let workspace = Self::load(workspace_root)?;
        let description = match workspace.current_commit() {
            Ok(commit) => commit.description().to_owned(),
            Err(JjError::MissingWorkingCopy { .. }) => String::new(),
            Err(error) => return Err(error),
        };

        let mut status = workspace_status_from_jj_status(&jj_status, description);
        if let Ok(lines) = workspace.tracked_bookmark_sync_status_lines() {
            status.extra_lines.extend(lines);
        }
        Ok(status)
    }
}

/// Snapshots pending disk changes through jj's command path and returns the workspace root.
pub(super) fn snapshot_working_copy_for_read(current_dir: &Path) -> Result<PathBuf, JjError> {
    let workspace_root = find_jj_workspace_root(current_dir)?;
    // jj-lib loads are read-only with respect to the working copy. Reuse jj's
    // command entrypoint so jx read commands see the same disk state as jj.
    run_jj_status(&workspace_root, false)?;
    Ok(workspace_root)
}

pub(super) fn run_jj_status(current_dir: &Path, color: bool) -> Result<String, JjError> {
    let output = Command::new("jj")
        .arg("--no-pager")
        .arg(if color {
            "--color=always"
        } else {
            "--color=never"
        })
        .arg("status")
        .current_dir(current_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .and_then(|child| child.wait_with_output())
        .map_err(|source| JjError::StatusStart { source })?;

    if !output.status.success() {
        return Err(JjError::StatusFailed {
            status: exit_status_summary(output.status),
        });
    }

    String::from_utf8(output.stdout).map_err(|source| JjError::StatusDecode { source })
}

pub(super) fn workspace_status_from_jj_status(
    output: &str,
    description: String,
) -> WorkspaceStatus {
    let mut commit_lines = Vec::new();
    let mut change_lines = Vec::new();
    let mut extra_lines = Vec::new();
    let mut no_changes = None;
    let mut section = StatusOutputSection::Other;

    for line in output.lines() {
        match line {
            "Working copy changes:" => {
                section = StatusOutputSection::Changes;
            }
            "Untracked paths:" => {
                section = StatusOutputSection::Changes;
                change_lines.push(line.to_owned());
            }
            "The working copy has no changes." => {
                section = StatusOutputSection::Other;
                no_changes = Some(line.to_owned());
            }
            "No working copy" => {
                section = StatusOutputSection::Other;
                commit_lines.push(line.to_owned());
            }
            _ if line.starts_with("Working copy  (@)")
                || line.starts_with("Parent commit (@-)") =>
            {
                section = StatusOutputSection::Other;
                commit_lines.push(line.to_owned());
            }
            _ => match section {
                StatusOutputSection::Changes => change_lines.push(line.to_owned()),
                StatusOutputSection::Other => extra_lines.push(line.to_owned()),
            },
        }
    }

    if change_lines.is_empty() {
        if let Some(no_changes) = no_changes {
            change_lines.push(no_changes);
        }
    }

    WorkspaceStatus {
        commit_lines,
        description,
        change_lines,
        extra_lines,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusOutputSection {
    Changes,
    Other,
}
