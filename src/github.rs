//! Narrow GitHub boundary backed by octocrab.
//!
//! Command and domain services use the `GitHubClient` trait and the domain
//! types in this module rather than octocrab models. The concrete
//! `OctocrabGitHubClient` keeps authentication, repository metadata, status
//! comparison, pull-request lookup/create/update, and reviewer synchronization
//! behind one integration boundary so later workflow phases can compose GitHub
//! behavior without depending on octocrab directly.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use octocrab::{models, params, Octocrab};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::repository::{RuntimeEnvironment, TokenReadError, TokenSource};

mod client;
mod error;
mod reviewers;
mod types;

pub use client::*;
#[cfg(test)]
use client::{
    map_comparison_status, map_graphql_pull_request_status, pull_request_status_query,
    suggested_reviewers_from_graphql, user_profiles_query, CompareCommitsResponse,
    CompareCommitsStatus, GraphQlLabelNode, GraphQlLabels, GraphQlPullRequestStatus,
    GraphQlPullRequestStatusCommit, GraphQlPullRequestStatusCommitNode,
    GraphQlPullRequestStatusCommits, GraphQlRequestedReviewer, GraphQlReviewAuthor,
    GraphQlReviewNode, GraphQlReviewRequestNode, GraphQlReviewRequests,
    GraphQlReviewThreadCommentNode, GraphQlReviewThreadComments, GraphQlReviewThreadNode,
    GraphQlReviewThreads, GraphQlReviews, GraphQlStatusCheckContextNode,
    GraphQlStatusCheckContexts, GraphQlStatusCheckRollup, GraphQlSuggestedReviewer,
    GraphQlSuggestedReviewerUser, PULL_REQUEST_SUGGESTED_REVIEWERS_QUERY,
    REVIEW_REQUEST_SEARCH_QUERIES,
};
pub use error::*;
pub use reviewers::*;
use reviewers::{difference, normalize_names};
pub use types::*;

#[cfg(test)]
mod tests;
