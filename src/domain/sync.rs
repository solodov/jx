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

/// Updates PR descriptions for pushed tracked bookmarks and returns PRs for sync annotations.
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
        let Some(pull_request) = github
            .find_pull_request_for_head(&context.origin.github, &head)
            .await?
        else {
            continue;
        };

        let pull_request = if let Some(description) = bookmark.new_full_description.as_deref() {
            sync_pull_request_description(context, github, pull_request, description).await?
        } else {
            pull_request
        };
        pull_requests.push(pull_request);
    }

    Ok(pull_requests)
}

async fn sync_pull_request_description(
    context: &RepositoryContext,
    github: &dyn GitHubClient,
    pull_request: PullRequestRecord,
    description: &str,
) -> Result<PullRequestRecord, WorkflowError> {
    let (title, body) = pull_request_description_from_text(description)?;
    if pull_request.title == title && pull_request.body.as_deref() == Some(body.as_str()) {
        return Ok(pull_request);
    }

    Ok(github
        .update_pull_request(
            &context.origin.github,
            pull_request.number,
            PullRequestUpdate {
                title: Some(title),
                body: Some(body),
                base: None,
            },
        )
        .await?)
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
