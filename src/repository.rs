//! Repository context discovery for the opinionated GitHub workflow.
//!
//! `jx` treats `origin` as the publishing remote while allowing read-only status
//! reporting across every configured GitHub remote. This module discovers the
//! enclosing jj workspace, reads Git remotes, parses GitHub repository identity,
//! records the safe token source, and composes the small optional global and
//! project configs before command handlers cross integration boundaries.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Component, Path, PathBuf},
};

use globset::Glob;
use thiserror::Error;

use crate::{
    github::{ReviewerCandidate, ReviewerTarget},
    jj::JjWorkspace,
};

pub use crate::github::GitHubRepository;

mod auth;
mod config;
mod context;
mod environment;
mod github_user_cache;
mod workspace_metadata;

pub use auth::*;
pub use config::*;
use context::find_workspace_root;
pub use context::*;
pub use environment::*;
pub use github_user_cache::*;
pub use workspace_metadata::*;

/// The only remote name used by the opinionated GitHub publishing workflow.
pub const ORIGIN_REMOTE_NAME: &str = "origin";

#[cfg(test)]
mod tests;
