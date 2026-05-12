use super::*;

/// Initializes the current directory as a Git-backed jj repository.
pub fn run_jj_git_init(current_dir: &Path) -> Result<(), JjError> {
    let status = Command::new("jj")
        .arg("--no-pager")
        .arg("--quiet")
        .arg("git")
        .arg("init")
        .current_dir(current_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| JjError::InitStart { source })?;

    if status.success() {
        Ok(())
    } else {
        Err(JjError::InitFailed {
            status: exit_status_summary(status),
        })
    }
}

/// Adds a jj workspace at the resolved destination while keeping command output concise.
pub fn run_jj_workspace_add(
    current_dir: &Path,
    options: &WorkspaceAddOptions,
) -> Result<(), JjError> {
    if let Some(parent) = options.destination.parent() {
        fs::create_dir_all(parent).map_err(|source| JjError::WorkspaceIo {
            action: "create workspace parent",
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let mut command = Command::new("jj");
    command
        .arg("--no-pager")
        .arg("--quiet")
        .arg("workspace")
        .arg("add")
        .arg("--name")
        .arg(&options.name);
    if let Some(revision) = &options.revision {
        command.arg("-r").arg(revision);
    }
    command.arg(&options.destination);

    let output = command
        .current_dir(current_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|child| child.wait_with_output())
        .map_err(|source| JjError::WorkspaceAddStart { source })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(JjError::WorkspaceAddFailed {
            status: process_failure_summary(output.status, &output.stderr),
        })
    }
}

/// Returns the active jj workspace without resolving any other workspace paths.
pub fn current_workspace_entry(current_dir: &Path) -> Result<WorkspaceEntry, JjError> {
    let workspace_root = find_jj_workspace_root(current_dir)?;
    let name = JjWorkspace::load(&workspace_root)?.workspace_name();
    Ok(WorkspaceEntry {
        name,
        root: workspace_root,
        is_current: true,
    })
}

/// Lists jj workspaces with their root paths and current-workspace marker.
pub fn jj_workspace_entries(current_dir: &Path) -> Result<Vec<WorkspaceEntry>, JjError> {
    let current_entry = current_workspace_entry(current_dir)?;
    let workspace_root = current_entry.root.clone();
    let current_workspace = current_entry.name.clone();
    let output = Command::new("jj")
        .arg("--no-pager")
        .arg("--color=never")
        .arg("workspace")
        .arg("list")
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .and_then(|child| child.wait_with_output())
        .map_err(|source| JjError::WorkspaceListStart { source })?;

    if !output.status.success() {
        return Err(JjError::WorkspaceListFailed {
            status: exit_status_summary(output.status),
        });
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|source| JjError::WorkspaceListDecode { source })?;
    workspace_names_from_jj_list(&stdout)
        .into_iter()
        .map(|name| {
            let is_current = name == current_workspace;
            Ok(WorkspaceEntry {
                root: if is_current {
                    workspace_root.clone()
                } else {
                    jj_workspace_root(current_dir, &name)?
                },
                is_current,
                name,
            })
        })
        .collect()
}

/// Forgets a jj workspace after moving its managed directory aside, then deletes it.
pub fn remove_jj_workspace(
    current_dir: &Path,
    options: &WorkspaceRemoveOptions,
) -> Result<(), JjError> {
    // Move the process itself to the safe operation directory before deleting so
    // removing the current workspace cannot leave the `jx` process inside it.
    std::env::set_current_dir(current_dir).map_err(|source| JjError::WorkspaceIo {
        action: "enter workspace removal directory",
        path: current_dir.to_path_buf(),
        source,
    })?;
    let tombstone = unique_tombstone_path(&options.root)?;
    fs::rename(&options.root, &tombstone).map_err(|source| JjError::WorkspaceIo {
        action: "move workspace aside",
        path: options.root.clone(),
        source,
    })?;

    if let Err(error) = run_jj_workspace_forget(current_dir, &options.name) {
        if let Err(rollback) = fs::rename(&tombstone, &options.root) {
            return Err(JjError::WorkspaceRemove {
                name: options.name.clone(),
                message: format!(
                    "forget failed ({error}); rollback from `{}` failed: {rollback}",
                    tombstone.display()
                ),
            });
        }
        return Err(error);
    }

    fs::remove_dir_all(&tombstone).map_err(|source| JjError::WorkspaceIo {
        action: "delete moved workspace",
        path: tombstone,
        source,
    })?;
    remove_empty_workspace_dirs(&options.root, &options.cleanup_root)
}

pub(super) fn remove_empty_workspace_dirs(
    workspace_root: &Path,
    cleanup_root: &Path,
) -> Result<(), JjError> {
    if !workspace_root.starts_with(cleanup_root) {
        return Err(JjError::WorkspaceRemove {
            name: workspace_root.display().to_string(),
            message: format!(
                "workspace root is outside cleanup root `{}`",
                cleanup_root.display()
            ),
        });
    }

    let mut cursor = workspace_root.parent();
    while let Some(path) = cursor {
        if !path.starts_with(cleanup_root) {
            break;
        }
        if !directory_is_empty(path)? {
            break;
        }
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => break,
            Err(source) => {
                return Err(JjError::WorkspaceIo {
                    action: "remove empty workspace directory",
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
        if path == cleanup_root {
            break;
        }
        cursor = path.parent();
    }

    Ok(())
}

fn directory_is_empty(path: &Path) -> Result<bool, JjError> {
    let mut entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(source) => {
            return Err(JjError::WorkspaceIo {
                action: "read workspace directory",
                path: path.to_path_buf(),
                source,
            });
        }
    };

    match entries.next() {
        Some(Ok(_)) => Ok(false),
        Some(Err(source)) => Err(JjError::WorkspaceIo {
            action: "read workspace directory",
            path: path.to_path_buf(),
            source,
        }),
        None => Ok(true),
    }
}

fn process_failure_summary(status: std::process::ExitStatus, stderr: &[u8]) -> String {
    let status = exit_status_summary(status);
    let stderr = String::from_utf8_lossy(stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        status
    } else {
        format!("{status}: {stderr}")
    }
}

pub(super) fn workspace_names_from_jj_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim()))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn jj_workspace_root(current_dir: &Path, name: &str) -> Result<PathBuf, JjError> {
    let output = Command::new("jj")
        .arg("--no-pager")
        .arg("--color=never")
        .arg("workspace")
        .arg("root")
        .arg("--name")
        .arg(name)
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .and_then(|child| child.wait_with_output())
        .map_err(|source| JjError::WorkspaceRootStart { source })?;

    if !output.status.success() {
        return Err(JjError::WorkspaceRootFailed {
            name: name.to_owned(),
            status: exit_status_summary(output.status),
        });
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|source| JjError::WorkspaceRootDecode {
            name: name.to_owned(),
            source,
        })?;
    Ok(PathBuf::from(stdout.trim_end_matches(['\r', '\n'])))
}

fn run_jj_workspace_forget(current_dir: &Path, name: &str) -> Result<(), JjError> {
    let status = Command::new("jj")
        .arg("--no-pager")
        .arg("--quiet")
        .arg("workspace")
        .arg("forget")
        .arg(name)
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| JjError::WorkspaceForgetStart {
            name: name.to_owned(),
            source,
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(JjError::WorkspaceForgetFailed {
            name: name.to_owned(),
            status: exit_status_summary(status),
        })
    }
}

fn unique_tombstone_path(root: &Path) -> Result<PathBuf, JjError> {
    let parent = root.parent().ok_or_else(|| JjError::WorkspaceRemove {
        name: root.display().to_string(),
        message: "workspace root has no parent directory".to_owned(),
    })?;
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| JjError::WorkspaceRemove {
            name: root.display().to_string(),
            message: "workspace root name is not valid UTF-8".to_owned(),
        })?;

    for attempt in 0..100 {
        let candidate = parent.join(format!(
            ".jx-removing-{name}-{}-{attempt}",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(JjError::WorkspaceRemove {
        name: name.to_owned(),
        message: "could not allocate a temporary removal path".to_owned(),
    })
}
