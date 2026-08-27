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

    /// Returns status lines for the working copy or a selected revision.
    pub fn status(
        current_dir: &Path,
        revision: Option<&str>,
        color: bool,
    ) -> Result<WorkspaceStatus, JjError> {
        match revision {
            Some(revision) => Self::selected_revision_status(current_dir, revision),
            None => Self::current_status(current_dir, color),
        }
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

    /// Returns status lines for one selected commit without moving the working copy.
    pub fn selected_revision_status(
        current_dir: &Path,
        revision: &str,
    ) -> Result<WorkspaceStatus, JjError> {
        let workspace = Self::load_after_working_copy_snapshot(current_dir)?;
        workspace.status_for_revision(revision)
    }

    pub(super) fn status_for_revision(&self, revision: &str) -> Result<WorkspaceStatus, JjError> {
        let target = self.resolve_single_revision_or_local_bookmark_fragment(
            revision,
            "In selected jj revision",
        )?;
        self.status_for_commit(&target)
    }

    fn status_for_commit(&self, target: &Commit) -> Result<WorkspaceStatus, JjError> {
        let changed = changed_file_facts_for_commit(self.repo.as_ref(), target)?;
        let change_lines = if changed.lines.is_empty() {
            vec!["The selected commit has no changes.".to_owned()]
        } else {
            changed.lines
        };
        let mut commit_lines = vec![format!(
            "Selected commit: {}",
            self.status_commit_summary(target)
        )];
        for parent_id in target.parent_ids() {
            let parent = self.load_commit(parent_id)?;
            commit_lines.push(format!(
                "Parent commit  : {}",
                self.status_commit_summary(&parent)
            ));
        }

        Ok(WorkspaceStatus {
            commit_lines,
            description: target.description().to_owned(),
            change_lines,
            extra_lines: Vec::new(),
        })
    }

    fn status_commit_summary(&self, commit: &Commit) -> String {
        let mut summary = format!(
            "{} {}",
            short_change_id(commit),
            short_commit_id(commit.id())
        );
        let mut bookmarks = self.local_bookmarks_for_commit(commit.id());
        bookmarks.sort();
        bookmarks.dedup();
        if !bookmarks.is_empty() {
            summary.push(' ');
            summary.push_str(&bookmarks.join(" "));
        }
        summary.push_str(" | ");
        summary.push_str(first_description_line(commit.description()));
        summary
    }
}

/// Snapshots pending disk changes through jj's command path and returns the workspace root.
pub(super) fn snapshot_working_copy_for_read(current_dir: &Path) -> Result<PathBuf, JjError> {
    let workspace_root = find_jj_workspace_root(current_dir)?;
    // jj-lib loads are read-only with respect to the working copy. Reuse jj's
    // command entrypoint so jx read commands see the same disk state as jj.
    run_quiet_jj_status(&workspace_root, false)?;
    Ok(workspace_root)
}

pub(super) fn run_jj_status(current_dir: &Path, color: bool) -> Result<String, JjError> {
    run_jj_status_with_stderr(current_dir, color, JjStatusStderr::Inherit)
}

fn run_quiet_jj_status(current_dir: &Path, color: bool) -> Result<String, JjError> {
    run_jj_status_with_stderr(current_dir, color, JjStatusStderr::Capture)
}

fn run_jj_status_with_stderr(
    current_dir: &Path,
    color: bool,
    stderr: JjStatusStderr,
) -> Result<String, JjError> {
    let mut command = Command::new("jj");
    command
        .arg("--no-pager")
        .arg(if color {
            "--color=always"
        } else {
            "--color=never"
        })
        .arg("status")
        .current_dir(current_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped());
    match stderr {
        JjStatusStderr::Inherit => {
            command.stderr(Stdio::inherit());
        }
        JjStatusStderr::Capture => {
            command.stderr(Stdio::piped());
        }
    }

    let output = command
        .spawn()
        .and_then(|child| child.wait_with_output())
        .map_err(|source| JjError::StatusStart { source })?;

    if !output.status.success() {
        return Err(JjError::StatusFailed {
            status: jj_status_failure_summary(exit_status_summary(output.status), &output.stderr),
        });
    }

    String::from_utf8(output.stdout).map_err(|source| JjError::StatusDecode { source })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JjStatusStderr {
    Inherit,
    Capture,
}

pub(super) fn jj_status_failure_summary(mut status: String, stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        status.push_str(": ");
        status.push_str(stderr);
    }
    status
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
