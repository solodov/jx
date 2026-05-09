//! Library boundary for `jx` command orchestration.
//!
//! The binary delegates into this crate so CLI parsing, repository context
//! loading, jj/GitHub boundaries, domain planning, and rendering remain
//! testable without invoking a child process.

pub mod commands;
pub mod domain;
pub mod github;
pub mod jj;
pub mod repository;

/// Runs command orchestration using the current process arguments and environment.
pub fn run_from_process() -> Result<commands::CommandResult, commands::CommandError> {
    commands::run_from_process()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::RuntimeEnvironment;
    use jj_lib::{config::StackedConfig, settings::UserSettings, workspace::Workspace};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn run_with_args_surfaces_repository_errors() {
        // Verifies: Library orchestration surfaces repository errors without child processes.
        let workspace = TestWorkspace::new();
        let environment = RuntimeEnvironment::new(workspace.path(), []);

        let error = commands::run_with_args(["jx", "fetch"], &environment)
            .expect_err("origin is required before jj mutation");

        assert!(error
            .to_string()
            .contains("fixed `origin` remote is missing"));
    }

    struct TestWorkspace {
        root: PathBuf,
    }

    impl TestWorkspace {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("jx-lib-test-{unique}"));
            fs::create_dir_all(&root).expect("create workspace root");
            let settings =
                UserSettings::from_config(StackedConfig::with_defaults()).expect("test settings");
            pollster::block_on(Workspace::init_internal_git(&settings, &root))
                .expect("initialize jj workspace");
            Self { root }
        }

        fn path(&self) -> PathBuf {
            self.root.clone()
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
