//! Domain services for workflow command behavior.
//!
//! Command handlers load repository context and integration boundaries, then
//! delegate to this module for deterministic readiness, per-remote freshness,
//! bookmark, push, sync, and pull-request decisions. `check`, `remote-status`,
//! `fetch`, push planning, sync guards, and stack PR planning are implemented
//! here so they can be tested with fake jj/GitHub boundaries.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    github::{
        CommitComparison, ComparisonStatus, GitHubClient, GitHubError, LabelApplyResult,
        PullRequestAutoMergeStatus, PullRequestCheck, PullRequestCheckStatus, PullRequestCreate,
        PullRequestHead, PullRequestMergeStatus, PullRequestRecord, PullRequestReviewStatus,
        PullRequestStatusRecord, PullRequestUpdate, RepositoryAccess, ReviewerCandidate,
        ReviewerSelection, ReviewerSyncResult, ReviewerTarget,
    },
    jj::{
        BookmarkUpdate, FetchOutcome, LocalStackBranch, PushOutcome, SkippedPushBookmarkSummary,
        SkippedSameTreeBookmarkSummary, StatusRemoteFacts, StatusWorkspaceFacts, SyncPushOutcome,
        TrackedPushOutcome, WorkspaceFacts,
    },
    repository::{
        GitHubRepository, PullRequestEventPredicate, PullRequestEventQuery, RepoEvent,
        RepoEventHandler, RepoEventHandlerRun, RepoReviewConfig, RepoStackStatusConfig,
        RepositoryContext, RepositoryError, StackMetadata, StackMetadataNode,
    },
};

mod bookmark;
mod errors;
mod pull_request;
mod pull_request_stack;
mod push;
mod readiness;
mod reports;
mod review;
mod status;
mod sync;

pub use bookmark::*;
pub use errors::*;
pub use pull_request::*;
pub use pull_request_stack::*;
pub use push::*;
pub use readiness::*;
use reports::repository_summary;
pub use reports::*;
pub use review::*;
pub use status::*;
pub use sync::*;

#[cfg(test)]
mod tests;
