use super::*;

/// Builds the operator-facing report for a completed `jx fetch` mutation.
pub fn fetch_report(context: &RepositoryContext, outcome: FetchOutcome) -> FetchReport {
    FetchReport {
        repository: repository_summary(context),
        outcome,
    }
}

/// Builds the operator-facing report for a completed `jx rebase-on-trunk` mutation.
pub fn rebase_on_trunk_report(
    context: &RepositoryContext,
    outcome: RebaseOnTrunkOutcome,
) -> RebaseOnTrunkReport {
    RebaseOnTrunkReport {
        repository: repository_summary(context),
        outcome,
    }
}

/// Builds the operator-facing report for a completed fetch-and-push synchronization.
pub fn sync_report(
    context: &RepositoryContext,
    fetch: FetchOutcome,
    push: TrackedPushOutcome,
    pull_requests: Vec<PullRequestRecord>,
) -> SyncReport {
    SyncReport {
        repository: repository_summary(context),
        fetch,
        push,
        pull_requests,
    }
}

/// Loads PRs for changed tracked bookmarks so sync can show navigable PR annotations.
pub async fn sync_pull_requests(
    context: &RepositoryContext,
    push: &TrackedPushOutcome,
    github: &dyn GitHubClient,
) -> Result<Vec<PullRequestRecord>, WorkflowError> {
    let mut seen = BTreeSet::new();
    let mut pull_requests = Vec::new();
    for bookmark in &push.bookmarks {
        if !seen.insert(bookmark.branch.as_str()) {
            continue;
        }

        let head = PullRequestHead::same_repository(&context.origin.github.owner, &bookmark.branch);
        if let Some(pull_request) = github
            .find_pull_request_for_head(&context.origin.github, &head)
            .await?
        {
            pull_requests.push(pull_request);
        }
    }

    Ok(pull_requests)
}

/// Returns an error when fetch created conflicts so sync can stop before pushing.
pub fn ensure_fetch_is_pushable(outcome: &FetchOutcome) -> Result<(), WorkflowError> {
    let conflicted = outcome
        .rebased_commits
        .iter()
        .filter(|commit| commit.has_conflict)
        .map(|commit| commit.new_short_commit_id.clone())
        .collect::<Vec<_>>();

    if conflicted.is_empty() {
        Ok(())
    } else {
        Err(WorkflowError::FetchConflicts {
            commits: conflicted.join(", "),
        })
    }
}
