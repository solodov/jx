use crate::jj::JjError;
use std::{path::PathBuf, time::Instant};

/// Log phase measurements retained even when a later phase fails.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogTimings {
    pub workspace_root: Option<PathBuf>,
    pub immutable_commit_count: Option<usize>,
    pub steps: Vec<LogTimingStep>,
}

/// One completed log phase, including the error from a failed phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTimingStep {
    pub name: &'static str,
    pub duration_us: u64,
    pub error: Option<String>,
}

impl LogTimings {
    pub(super) fn measure<T>(
        &mut self,
        name: &'static str,
        operation: impl FnOnce() -> Result<T, JjError>,
    ) -> Result<T, JjError> {
        let started = Instant::now();
        let result = operation();
        self.steps.push(LogTimingStep {
            name,
            duration_us: started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
            error: result.as_ref().err().map(ToString::to_string),
        });
        result
    }
}
