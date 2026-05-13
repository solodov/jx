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
                remote: remote.name.clone(),
            })?;
        let comparison = github
            .compare_commits(&remote.github, &facts.trunk_git_commit_sha, &facts.branch)
            .await?;
        let comparison = status_comparison(&facts.branch, &facts.trunk_git_commit_sha, comparison)?;

        reports.push(RemoteStatusReport {
            name: remote.name.clone(),
            url: remote.url.clone(),
            github_url: remote.github.https_url(),
            branch: facts.branch.clone(),
            local_trunk_sha: facts.trunk_git_commit_sha.clone(),
            local_trunk_short_sha: facts.trunk_short_commit_id.clone(),
            local_ahead_by: facts.local_ahead_by,
            comparison,
        });
    }

    Ok(StatusReport {
        remotes: reports,
        fork: None,
    })
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
    })
}
