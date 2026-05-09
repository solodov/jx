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

    Ok(StatusReport { remotes: reports })
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
