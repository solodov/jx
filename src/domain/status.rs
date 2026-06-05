use super::*;

/// Compares local cached remote-trunk freshness against each live GitHub remote.
pub async fn status_report(
    context: &RepositoryContext,
    workspace: StatusWorkspaceFacts,
    github: &dyn GitHubClient,
) -> Result<StatusReport, WorkflowError> {
    let mut reports = Vec::new();

    for remote in &context.github_remotes {
        let facts = workspace
            .remotes
            .iter()
            .find(|facts| facts.remote == remote.name)
            .ok_or_else(|| WorkflowError::MissingStatusRemote {
                remote: remote.name.to_owned(),
            })?;
        let comparison = github
            .compare_commits(&remote.github, &facts.trunk_git_commit_sha, &facts.branch)
            .await?;
        let comparison = status_comparison(&facts.branch, &facts.trunk_git_commit_sha, comparison)?;

        reports.push(remote_status_report_entry(
            remote.name.clone(),
            remote.url.clone(),
            &remote.github,
            facts,
            comparison,
        ));
    }

    Ok(StatusReport {
        remotes: reports,
        fork: None,
    })
}

/// Checks origin trunk freshness cheaply for stack status without exact GitHub commit counts.
pub async fn stack_trunk_status_report(
    context: &RepositoryContext,
    workspace: StatusWorkspaceFacts,
    github: &dyn GitHubClient,
) -> Result<RemoteStatusReport, WorkflowError> {
    let remote = &context.origin;
    let facts = workspace
        .remotes
        .iter()
        .find(|facts| facts.remote == remote.name)
        .ok_or_else(|| WorkflowError::MissingStatusRemote {
            remote: remote.name.to_owned(),
        })?;
    let branch_head_sha = github
        .branch_head_sha(&remote.github, &facts.branch)
        .await?;

    stack_trunk_status_report_from_branch_head(context, workspace, &branch_head_sha)
}

/// Builds the stack-status trunk report from a cheap live branch-head lookup.
pub fn stack_trunk_status_report_from_branch_head(
    context: &RepositoryContext,
    workspace: StatusWorkspaceFacts,
    branch_head_sha: &str,
) -> Result<RemoteStatusReport, WorkflowError> {
    let remote = &context.origin;
    let facts = workspace
        .remotes
        .iter()
        .find(|facts| facts.remote == remote.name)
        .ok_or_else(|| WorkflowError::MissingStatusRemote {
            remote: remote.name.to_owned(),
        })?;

    Ok(remote_status_report_entry(
        remote.name.to_owned(),
        remote.url.clone(),
        &remote.github,
        facts,
        stack_trunk_comparison(facts, branch_head_sha),
    ))
}

/// Returns the origin remote freshness entry from a full status report.
pub fn origin_status_report(
    context: &RepositoryContext,
    report: StatusReport,
) -> Option<RemoteStatusReport> {
    report
        .remotes
        .into_iter()
        .find(|remote| remote.name == context.origin.name)
}

/// Extends remote freshness with source/fork freshness when origin is a GitHub fork.
pub async fn remote_status_report(
    context: &RepositoryContext,
    workspace: StatusWorkspaceFacts,
    github: &dyn GitHubClient,
) -> Result<StatusReport, WorkflowError> {
    let mut report = status_report(context, workspace, github).await?;
    report.fork = fork_status_report(context, &report.remotes, github).await?;
    Ok(report)
}

async fn fork_status_report(
    context: &RepositoryContext,
    remotes: &[RemoteStatusReport],
    github: &dyn GitHubClient,
) -> Result<Option<ForkStatusReport>, WorkflowError> {
    let Some(origin) = remotes
        .iter()
        .find(|remote| remote.name == crate::repository::ORIGIN_REMOTE_NAME)
    else {
        return Ok(None);
    };
    let Some(fork) = github.repository_fork(&context.origin.github).await? else {
        return Ok(None);
    };
    if fork.source == context.origin.github {
        return Ok(None);
    }

    let source_branch = fork
        .source_default_branch
        .unwrap_or_else(|| origin.branch.clone());
    let fork_head = fork_head_ref(&context.origin.github, &origin.branch);
    let comparison = github
        .compare_commits(&fork.source, &source_branch, &fork_head)
        .await?;
    let comparison = fork_status_comparison(
        &fork.source,
        &source_branch,
        &context.origin.github,
        &origin.branch,
        comparison,
    )?;

    Ok(Some(ForkStatusReport {
        fork: context.origin.github.clone(),
        fork_branch: origin.branch.clone(),
        source: fork.source,
        source_branch,
        comparison,
    }))
}

fn fork_head_ref(fork: &GitHubRepository, branch: &str) -> String {
    format!("{}:{branch}", fork.owner)
}

fn fork_status_comparison(
    source: &GitHubRepository,
    source_branch: &str,
    fork: &GitHubRepository,
    fork_branch: &str,
    comparison: CommitComparison,
) -> Result<ForkStatusComparison, WorkflowError> {
    let state = match comparison.status {
        ComparisonStatus::Identical => ForkStatusState::Synced,
        ComparisonStatus::Ahead => ForkStatusState::ForkAhead,
        ComparisonStatus::Behind => ForkStatusState::SourceAhead,
        ComparisonStatus::Diverged => ForkStatusState::Diverged,
        ComparisonStatus::Unknown => {
            return Err(WorkflowError::UnavailableForkComparison {
                source_repo: source.slug(),
                source_branch: source_branch.to_owned(),
                fork: fork.slug(),
                fork_branch: fork_branch.to_owned(),
            });
        }
    };

    Ok(ForkStatusComparison {
        state,
        source_ahead_by: comparison.behind_by,
        fork_ahead_by: comparison.ahead_by,
    })
}

fn status_comparison(
    branch: &str,
    local_sha: &str,
    comparison: CommitComparison,
) -> Result<StatusComparison, WorkflowError> {
    let state = match comparison.status {
        ComparisonStatus::Identical => StatusState::UpToDate,
        ComparisonStatus::Ahead => StatusState::GithubAhead,
        ComparisonStatus::Behind => StatusState::LocalAhead,
        ComparisonStatus::Diverged => StatusState::Diverged,
        ComparisonStatus::Unknown => {
            return Err(WorkflowError::UnavailableComparison {
                branch: branch.to_owned(),
                local_sha: local_sha.to_owned(),
            });
        }
    };

    Ok(StatusComparison {
        state,
        github_ahead_by: comparison.ahead_by,
        github_behind_by: comparison.behind_by,
        counts_exact: true,
    })
}

fn stack_trunk_comparison(facts: &StatusRemoteFacts, branch_head_sha: &str) -> StatusComparison {
    if branch_head_sha == facts.trunk_git_commit_sha {
        return StatusComparison {
            state: StatusState::UpToDate,
            github_ahead_by: 0,
            github_behind_by: 0,
            counts_exact: true,
        };
    }

    StatusComparison {
        state: StatusState::GithubAhead,
        github_ahead_by: 0,
        github_behind_by: 0,
        counts_exact: false,
    }
}

fn remote_status_report_entry(
    name: String,
    url: String,
    github: &GitHubRepository,
    facts: &StatusRemoteFacts,
    comparison: StatusComparison,
) -> RemoteStatusReport {
    RemoteStatusReport {
        name,
        url,
        github_url: github.https_url(),
        branch: facts.branch.clone(),
        local_trunk_sha: facts.trunk_git_commit_sha.clone(),
        local_trunk_short_sha: facts.trunk_short_commit_id.clone(),
        local_ahead_by: facts.local_ahead_by,
        comparison,
    }
}
