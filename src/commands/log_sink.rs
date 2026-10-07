//! Bounded diagnostic logs with complete, coordinated appends across jx processes.

use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

const MAX_LOG_BYTES: u64 = 20 * 1024 * 1024;
const LOG_ARCHIVES: usize = 5;

/// Owns diagnostic-file access and retention; clones never retain an active file descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct LogSink {
    path: PathBuf,
    max_bytes: u64,
    archives: usize,
}

impl LogSink {
    /// Opens a diagnostic log with size-triggered rotation and five retained archives.
    pub(in crate::commands) fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        Self::with_limits(path.into(), MAX_LOG_BYTES, LOG_ARCHIVES)
    }

    /// Returns the active log path for operator-visible diagnostics.
    pub(in crate::commands) fn path(&self) -> &Path {
        &self.path
    }

    /// Serializes and appends one complete JSONL record without interleaving other writers.
    pub(in crate::commands) fn append_json(&self, record: &impl Serialize) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
        bytes.push(b'\n');
        self.append(&bytes)
    }

    /// Appends one complete record or bounded output chunk, rotating before it would exceed the limit.
    pub(in crate::commands) fn append(&self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() as u64 > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "diagnostic record exceeds log size limit",
            ));
        }
        let _lock = self.lock()?;
        // Reopen under the shared lock: another process may have rotated since the last append.
        let mut file = private_append_options().open(&self.path)?;
        let size = file.metadata()?.len();
        if size > 0 && size.saturating_add(bytes.len() as u64) > self.max_bytes {
            drop(file);
            self.rotate()?;
            file = private_append_options().open(&self.path)?;
        }
        file.write_all(bytes)
    }

    fn with_limits(path: PathBuf, max_bytes: u64, archives: usize) -> io::Result<Self> {
        if max_bytes == 0 || archives == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "diagnostic log limits must be positive",
            ));
        }
        if path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "diagnostic log path is a directory",
            ));
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let sink = Self {
            path,
            max_bytes,
            archives,
        };
        let _lock = sink.lock()?;
        private_append_options().open(&sink.path)?;
        Ok(sink)
    }

    fn lock(&self) -> io::Result<File> {
        let mut path = self.path.as_os_str().to_os_string();
        path.push(".lock");
        let file = private_append_options().read(true).open(Path::new(&path))?;
        // The lock file is never renamed or removed; locking the active log would split writers at rotation.
        file.lock()?;
        Ok(file)
    }

    fn rotate(&self) -> io::Result<()> {
        let mut rotator =
            BasicRollingFileAppender::new(&self.path, RollingConditionBasic::new(), self.archives)?;
        rotator.rollover()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // The library creates an empty replacement using default permissions; restrict it before any data is written.
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

fn private_append_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

#[cfg(test)]
#[path = "log_sink/tests/mod.rs"]
mod tests;
