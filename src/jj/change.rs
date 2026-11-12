//! Revision-scoped operations using JJ tree/transaction APIs and native search/run commands.

use super::*;
use serde::{Deserialize, Serialize};
use std::process::Output;

mod batch;
pub use batch::{ChangeBatch, FileOperation, FileReplacement};

/// A change identity together with its immutable version and operation snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSnapshot {
    pub change_id: String,
    pub commit_id: String,
    pub operation_id: String,
}

/// Text read from a pinned change, never from the caller's working files.
#[derive(Debug, Serialize)]
pub struct ChangeRead {
    pub snapshot: ChangeSnapshot,
    pub path: String,
    pub content: String,
}

/// Result of a JJ mutation, including conflicts introduced in descendants.
#[derive(Debug, Serialize)]
pub struct ChangeResult {
    pub snapshot: ChangeSnapshot,
    pub changed: bool,
    pub conflicted_changes: Vec<String>,
}

impl JjWorkspace {
    /// Reads a regular UTF-8 file at the selected revision without snapshotting disk changes.
    pub fn read_change_file(&self, revision: &str, path: &str) -> Result<ChangeRead, JjError> {
        batch::validate_path(path)?;
        let commit = self.resolve_single_revision(revision, "jx change read")?;
        let content = read_regular_file(&commit, path)?;
        Ok(ChangeRead {
            snapshot: self.change_snapshot(&commit),
            path: path.to_owned(),
            content,
        })
    }

    /// Searches the selected revision using jj's native string-pattern syntax.
    pub fn search_change(
        &self,
        revision: &str,
        pattern: &str,
    ) -> Result<(ChangeSnapshot, String), JjError> {
        let commit = self.resolve_single_revision(revision, "jx change search")?;
        let output = self.change_command_at([
            "file",
            "search",
            "-r",
            &commit.id().hex(),
            "--line-number",
            "--pattern",
            pattern,
        ])?;
        Ok((
            self.change_snapshot(&commit),
            String::from_utf8(output.stdout).map_err(change_request_error)?,
        ))
    }

    /// Creates a described change without switching or updating the caller's working copy.
    pub fn new_change(
        current_dir: &Path,
        parent: &str,
        description: &str,
        check: impl FnMut() -> Result<(), JjError>,
    ) -> Result<ChangeResult, JjError> {
        if description.trim().is_empty() {
            return Err(change_request_error(
                "a new change needs a non-empty message",
            ));
        }
        let _guard = RepositoryLock::acquire_checked(current_dir, check)?;
        let root = find_jj_workspace_root(current_dir)?;
        let mut workspace = Self::load(&root)?;
        let parent = workspace.resolve_single_revision(parent, "jx change new")?;
        let mut tx = workspace.repo.start_transaction();
        let created = pollster::block_on(
            tx.repo_mut()
                .new_commit(vec![parent.id().clone()], parent.tree())
                .set_description(description)
                .write(),
        )
        .map_err(change_request_error)?;
        let conflicted_changes = if created.has_conflict() {
            vec![created.change_id().reverse_hex()]
        } else {
            Vec::new()
        };
        workspace.repo =
            super::coordination::publish_checked(tx, "jx change new", &workspace.repo)?;
        Ok(ChangeResult {
            snapshot: workspace.change_snapshot(&created),
            changed: true,
            conflicted_changes,
        })
    }

    /// Applies one atomic batch through JJ, rejecting stale versions and occupied stacks.
    pub fn apply_change(
        current_dir: &Path,
        revision: &str,
        expected: &str,
        batch: &ChangeBatch,
        check: impl FnMut() -> Result<(), JjError>,
    ) -> Result<ChangeResult, JjError> {
        batch.validate()?;
        let _guard = RepositoryLock::acquire_checked(current_dir, check)?;
        let root = find_jj_workspace_root(current_dir)?;
        let before = Self::load(&root)?;
        let expected_id = CommitId::try_from_hex(expected)
            .filter(|_| expected.len() == 2 * before.repo.store().commit_id_length())
            .ok_or_else(|| {
                change_request_error(
                    "--expect must be a full commit hash from a previous read or mutation",
                )
            })?;
        before.check_live_commit(&before.load_commit(&expected_id)?)?;
        let commit = before.resolve_single_revision(revision, "jx change apply")?;
        before.check_live_commit(&commit)?;
        if commit.id().hex() != expected {
            return Err(JjError::StaleChange {
                expected: expected.to_owned(),
                actual: commit.id().hex(),
            });
        }
        if before.is_immutable_commit(&commit)? {
            return Err(change_request_error("the selected change is immutable"));
        }
        if commit.has_conflict() {
            return Err(change_request_error(
                "resolve the selected change's conflicts before applying a file batch",
            ));
        }
        let tree = pollster::block_on(batch.apply(commit.tree()))?;
        if tree.tree_ids() == commit.tree_ids() {
            return before.change_result(&commit, false);
        }
        // Leaving an occupied revision stale can make native JJ recovery merge
        // later disk edits into a divergent version and overwrite working files.
        let mut occupied = Vec::new();
        for (name, current) in before.repo.view().wc_commit_ids() {
            if before.is_ancestor_or_equal(commit.id(), current)? {
                occupied.push(name.as_str().to_owned());
            }
        }
        if !occupied.is_empty() {
            occupied.sort();
            return Err(JjError::CheckedOutChanges {
                workspaces: occupied,
            });
        }
        let mut tx = before.repo.start_transaction();
        let updated = pollster::block_on(async {
            let updated = tx
                .repo_mut()
                .rewrite_commit(&commit)
                .set_tree(tree)
                .write()
                .await?;
            tx.repo_mut().rebase_descendants().await?;
            Ok::<_, BackendError>(updated)
        })
        .map_err(change_request_error)?;
        let conflicted_changes = conflicted_change_ids(tx.repo(), &updated)?;
        let repo = super::coordination::publish_checked(tx, "jx change apply", &before.repo)?;
        let after = Self {
            workspace: before.workspace,
            repo,
        };
        Ok(ChangeResult {
            snapshot: after.change_snapshot(&updated),
            changed: true,
            conflicted_changes,
        })
    }

    /// Runs a check against an isolated pinned revision, never saving its file changes.
    pub fn run_change_command(
        &self,
        revision: &str,
        args: &[String],
    ) -> Result<(ChangeSnapshot, Output), JjError> {
        if args.is_empty() {
            return Err(change_request_error("a command is required"));
        }
        let commit = self.resolve_single_revision(revision, "jx change run")?;
        let output = native_change_command(self.workspace.workspace_root())
            .args([
                "--at-operation",
                &self.repo.op_id().hex(),
                "run",
                "--root",
                "--sparse-patterns",
                "full",
                "--ignore-changes",
                "-r",
                &commit.id().hex(),
                "--",
            ])
            .args(args)
            .output()
            .map_err(|error| JjError::ChangeCommand {
                message: error.to_string(),
            })?;
        Ok((self.change_snapshot(&commit), output))
    }

    /// Rejects hidden or superseded objects instead of rewriting an obsolete version of a change.
    pub(super) fn check_live_commit(&self, commit: &Commit) -> Result<(), JjError> {
        let change_id = commit.change_id().reverse_hex();
        let live = pollster::block_on(self.repo.resolve_change_id(commit.change_id()))
            .map_err(|error| JjError::Index {
                message: error.to_string(),
            })?
            .and_then(|targets| targets.into_visible())
            .unwrap_or_default();
        match live.as_slice() {
            [] => Err(JjError::AbandonedChange { change_id }),
            [live] if live == commit.id() => Ok(()),
            [live] => Err(JjError::StaleChange {
                expected: commit.id().hex(),
                actual: live.hex(),
            }),
            _ => Err(JjError::DivergentChange { change_id }),
        }
    }

    fn change_command_at<const N: usize>(&self, args: [&str; N]) -> Result<Output, JjError> {
        checked_output(
            native_change_command(self.workspace.workspace_root())
                .args(["--at-operation", &self.repo.op_id().hex()])
                .args(args)
                .output(),
        )
    }

    fn change_snapshot(&self, commit: &Commit) -> ChangeSnapshot {
        ChangeSnapshot {
            change_id: commit.change_id().reverse_hex(),
            commit_id: commit.id().hex(),
            operation_id: self.repo.op_id().hex(),
        }
    }

    fn change_result(&self, commit: &Commit, changed: bool) -> Result<ChangeResult, JjError> {
        let conflicted_changes = conflicted_change_ids(self.repo.as_ref(), commit)?;
        Ok(ChangeResult {
            snapshot: self.change_snapshot(commit),
            changed,
            conflicted_changes,
        })
    }
}

fn conflicted_change_ids(
    repo: &dyn jj_lib::repo::Repo,
    commit: &Commit,
) -> Result<Vec<String>, JjError> {
    let descendants = ResolvedRevsetExpression::commit(commit.id().clone())
        .descendants()
        .evaluate(repo)
        .map_err(|error| JjError::Index {
            message: error.to_string(),
        })?;
    let ids = pollster::block_on(descendants.stream().try_collect::<Vec<_>>())
        .map_err(change_request_error)?;
    let mut conflicts = Vec::new();
    for id in ids {
        let commit = load_commit_from_repo(repo, &id)?;
        if commit.has_conflict() {
            conflicts.push(commit.change_id().reverse_hex());
        }
    }
    conflicts.sort();
    conflicts.dedup();
    Ok(conflicts)
}

fn native_change_command(root: &Path) -> Command {
    let mut command = Command::new("jj");
    command
        .args(["--no-pager", "--color=never", "--ignore-working-copy"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn checked_output(result: io::Result<Output>) -> Result<Output, JjError> {
    let output = result.map_err(|error| JjError::ChangeCommand {
        message: error.to_string(),
    })?;
    if !output.status.success() {
        return Err(JjError::ChangeCommand {
            message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output)
}

fn read_regular_file(commit: &Commit, path: &str) -> Result<String, JjError> {
    let path =
        jj_lib::repo_path::RepoPathBuf::from_internal_string(path).map_err(change_request_error)?;
    let value =
        pollster::block_on(commit.tree().path_value(&path)).map_err(change_request_error)?;
    let Some(Some(jj_lib::backend::TreeValue::File { id, .. })) = value.as_resolved() else {
        return Err(change_request_error(
            "the path must name an existing, unconflicted regular file",
        ));
    };
    pollster::block_on(async {
        use futures::AsyncReadExt as _;
        let mut input = commit
            .tree()
            .store()
            .read_file(&path, id)
            .await
            .map_err(change_request_error)?;
        let mut content = String::new();
        input
            .read_to_string(&mut content)
            .await
            .map_err(change_request_error)?;
        Ok(content)
    })
}

fn change_request_error(error: impl ToString) -> JjError {
    JjError::ChangeRequest {
        message: error.to_string(),
    }
}
