//! Cross-process coordination for participating jx repository mutations.

use super::*;
use std::{
    fs::{File, OpenOptions, TryLockError},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

static INTERRUPT_CHECK: OnceLock<fn() -> Result<(), JjError>> = OnceLock::new();

/// Installs the application's cooperative interrupt check without coupling JJ to CLI state.
pub(crate) fn install_mutation_interrupt_check(check: fn() -> Result<(), JjError>) {
    INTERRUPT_CHECK.get_or_init(|| check);
}

/// Holds the shared repository's mutation lock until dropped.
///
/// Native jj processes do not participate in this lock. JJ's own publication,
/// Git import/export, and working-copy locks remain authoritative underneath it.
pub(crate) struct RepositoryLock {
    _file: File,
}

impl RepositoryLock {
    /// Locks the canonical shared repository, including when called from a linked workspace.
    pub(crate) fn acquire(current_dir: &Path) -> Result<Self, JjError> {
        Self::acquire_checked(current_dir, || {
            INTERRUPT_CHECK.get().map_or(Ok(()), |check| check())
        })
    }

    /// Waits for the repository lock, checking cancellation between attempts.
    pub(crate) fn acquire_checked(
        current_dir: &Path,
        mut check: impl FnMut() -> Result<(), JjError>,
    ) -> Result<Self, JjError> {
        let root = find_jj_workspace_root(current_dir)?;
        let loader = DefaultWorkspaceLoaderFactory
            .create(&root)
            .map_err(workspace_load_error)?;
        let repo_path =
            fs::canonicalize(loader.repo_path()).map_err(|source| JjError::WorkspaceIo {
                action: "resolve shared repository",
                path: loader.repo_path().to_owned(),
                source,
            })?;
        Self::lock_path(
            &repo_path.join("jx-mutation.lock"),
            Duration::from_secs(30),
            &mut check,
        )
    }

    fn lock_path(
        path: &Path,
        timeout: Duration,
        check: &mut impl FnMut() -> Result<(), JjError>,
    ) -> Result<Self, JjError> {
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        // Never unlink this file: replacing its inode would split waiting writers.
        let file = options.open(path).map_err(|source| JjError::WorkspaceIo {
            action: "open repository mutation lock",
            path: path.to_owned(),
            source,
        })?;
        let started = Instant::now();
        loop {
            check()?;
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) if started.elapsed() < timeout => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(JjError::MutationBusy {
                        path: path.to_owned(),
                    });
                }
                Err(TryLockError::Error(source)) => {
                    return Err(JjError::WorkspaceIo {
                        action: "lock repository mutations",
                        path: path.to_owned(),
                        source,
                    });
                }
            }
        }
    }
}

impl JjWorkspace {
    /// Acquires the mutation lock and reloads state before making any decisions.
    pub(super) fn start_mutation(&mut self) -> Result<RepositoryLock, JjError> {
        let guard = RepositoryLock::acquire(self.workspace.workspace_root())?;
        self.repo = load_single_head(self.workspace.repo_loader())?;
        Ok(guard)
    }
}

/// Pins one operation without implicitly reconciling concurrent operations.
pub(super) fn load_single_head(
    loader: &jj_lib::repo::RepoLoader,
) -> Result<Arc<ReadonlyRepo>, JjError> {
    pollster::block_on(async {
        let heads = {
            // Publication adds a new head before removing its parents. Wait for
            // that short update rather than misreporting its intermediate state.
            let store = loader.op_heads_store();
            let _lock = store.lock().await.map_err(repo_load_error)?;
            store.get_op_heads().await.map_err(repo_load_error)?
        };
        let [head] = heads.as_slice() else {
            return Err(JjError::ConcurrentOperations);
        };
        let operation = loader.load_operation(head).await.map_err(repo_load_error)?;
        loader.load_at(&operation).await.map_err(repo_load_error)
    })
}

/// Publishes a prepared JJ transaction only if its base is still the sole operation head.
///
/// JJ's publication lock makes the check and update indivisible with ordinary JJ
/// publishers. Native commands already computing against an older base can still
/// publish later, so this is not universal exclusion against native JJ clients.
pub(super) fn publish_checked(
    tx: jj_lib::transaction::Transaction,
    description: &str,
    expected: &Arc<ReadonlyRepo>,
) -> Result<Arc<ReadonlyRepo>, JjError> {
    pollster::block_on(async {
        let unpublished = tx.write(description).await.map_err(transaction_error)?;
        let store = expected.loader().op_heads_store();
        let _lock = store.lock().await.map_err(transaction_error)?;
        let heads = store.get_op_heads().await.map_err(transaction_error)?;
        if heads.as_slice() != [expected.op_id().clone()] {
            unpublished.leave_unpublished();
            return Err(JjError::ConcurrentOperations);
        }
        store
            .update_op_heads(
                unpublished.operation().parent_ids(),
                unpublished.operation().id(),
            )
            .await
            .map_err(transaction_error)?;
        Ok(unpublished.leave_unpublished())
    })
}

fn repo_load_error(error: impl ToString) -> JjError {
    JjError::RepoLoad {
        message: error.to_string(),
    }
}

fn transaction_error(error: impl ToString) -> JjError {
    JjError::Transaction {
        message: error.to_string(),
    }
}

#[cfg(test)]
#[path = "tests/coordination.rs"]
mod tests;
