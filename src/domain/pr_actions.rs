use std::path::{Path, PathBuf};

use crate::{
    github::PullRequestStatusRecord,
    repository::{
        render_pr_action_argument, GitHubRepository, PrActionConfigScope, PrActionSource,
        PrActionWorkingDirectory, ResolvedPrAction,
    },
};

/// Stable PR identity, independent of table ordering, displayed labels, or terminal width.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrActionKey {
    pub repository: String,
    pub number: u64,
}

/// Full PR facts available to manual actions. Local revisions must be verified, never inferred from `@`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrActionContext {
    pub repository: GitHubRepository,
    pub repository_root: Option<PathBuf>,
    pub pr_number: u64,
    pub pr_url: String,
    pub title: String,
    pub branch: String,
    pub base_branch: String,
    pub head_oid: Option<String>,
    pub local_commit_id: Option<String>,
    pub local_change_id: Option<String>,
}

impl PrActionContext {
    /// Builds action data from a status record, without inferring local revision identities.
    pub fn from_status(
        repository: GitHubRepository,
        repository_root: Option<PathBuf>,
        status: &PullRequestStatusRecord,
    ) -> Self {
        Self {
            pr_url: status
                .url
                .clone()
                .unwrap_or_else(|| format!("{}/pull/{}", repository.https_url(), status.number)),
            repository,
            repository_root,
            pr_number: status.number,
            title: status.title.clone(),
            branch: status.head_branch.clone(),
            base_branch: status.base_branch.clone(),
            head_oid: status.latest_commit_oid.clone(),
            local_commit_id: None,
            local_change_id: None,
        }
    }

    /// Returns the identity to retain across dashboard refreshes and resizes.
    pub fn key(&self) -> PrActionKey {
        PrActionKey {
            repository: self.repository.slug(),
            number: self.pr_number,
        }
    }
}

/// Prepared argv and working directory; constructing this value never executes a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPrAction {
    pub id: String,
    pub title: String,
    pub target: PrActionKey,
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub source: PrActionSource,
}

impl PreparedPrAction {
    /// Repository-local definitions require confirmation, even when overriding a global action ID.
    pub fn requires_confirmation(&self) -> bool {
        self.source.scope == PrActionConfigScope::Repository
    }
}

/// Why a configured action cannot currently be offered for the selected PR.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct PrActionUnavailable {
    pub reason: String,
}

/// Resolves one action without shell evaluation, fetching revisions, or changing the checkout.
pub fn prepare_pr_action(
    definition: &ResolvedPrAction,
    context: &PrActionContext,
    caller_directory: &Path,
) -> Result<PreparedPrAction, PrActionUnavailable> {
    let action = &definition.action;
    let cwd = match action.cwd {
        PrActionWorkingDirectory::Repository => context
            .repository_root
            .as_deref()
            .ok_or_else(|| unavailable("requires a local repository checkout"))?,
        PrActionWorkingDirectory::Caller => caller_directory,
    };
    if !cwd.is_absolute() {
        return Err(unavailable("action working directory must be absolute"));
    }
    let command = action
        .command
        .iter()
        .map(|arg| {
            render_pr_action_argument(arg, |name| parameter(context, name))
                .map_err(|reason| PrActionUnavailable { reason })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if command
        .first()
        .is_none_or(|program| program.trim().is_empty())
        || command.iter().any(|arg| arg.contains('\0'))
    {
        return Err(unavailable(
            "action command must name an executable and contain no NUL bytes",
        ));
    }
    Ok(PreparedPrAction {
        id: action.id.clone(),
        title: action.title.clone(),
        target: context.key(),
        command,
        cwd: cwd.to_path_buf(),
        source: definition.source.clone(),
    })
}

fn parameter(context: &PrActionContext, name: &str) -> Result<String, String> {
    let value = match name {
        "repo" => return Ok(context.repository.slug()),
        "repo_root" => context.repository_root.as_deref().and_then(Path::to_str),
        "pr_number" => return Ok(context.pr_number.to_string()),
        "pr_url" => Some(context.pr_url.as_str()),
        "title" => Some(context.title.as_str()),
        "branch" => Some(context.branch.as_str()),
        "base_branch" => Some(context.base_branch.as_str()),
        "head_oid" => context.head_oid.as_deref(),
        "local_commit_id" => context.local_commit_id.as_deref(),
        "local_change_id" => context.local_change_id.as_deref(),
        _ => None,
    };
    value
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("`{{{name}}}` is unavailable for this PR"))
}

fn unavailable(reason: &str) -> PrActionUnavailable {
    PrActionUnavailable {
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
#[path = "tests/pr_actions.rs"]
mod tests;
