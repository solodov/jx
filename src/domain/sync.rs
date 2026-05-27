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
    push: SyncPushOutcome,
    pull_requests: Vec<PullRequestRecord>,
) -> SyncReport {
    SyncReport {
        repository: repository_summary(context),
        fetch,
        push: push.pushed,
        skipped_conflicted_bookmarks: push.skipped_conflicted_bookmarks,
        pull_requests,
    }
}

/// Updates PR descriptions and stack bases for pushed tracked bookmarks.
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
            .find_open_pull_request(&context.origin.github, &head)
            .await?
        else {
            continue;
        };

        let pull_request = sync_pull_request_metadata(
            context,
            github,
            pull_request,
            bookmark.pull_request_description.as_deref(),
            bookmark.pull_request_base.as_deref(),
        )
        .await?;
        pull_requests.push(pull_request);
    }

    Ok(pull_requests)
}

async fn sync_pull_request_metadata(
    context: &RepositoryContext,
    github: &dyn GitHubClient,
    pull_request: PullRequestRecord,
    description: Option<&str>,
    base: Option<&str>,
) -> Result<PullRequestRecord, WorkflowError> {
    let mut update = PullRequestUpdate::default();

    if let Some(description) = description {
        let (title, body) = pull_request_description_from_text(description)?;
        if pull_request.title != title
            || !pull_request_body_matches(pull_request.body.as_deref(), &body)
        {
            update.title = Some(title);
            update.body = Some(body);
        }
    }

    if let Some(base) = base {
        if pull_request.base_branch != base {
            update.base = Some(base.to_owned());
        }
    }

    if update.title.is_none() && update.body.is_none() && update.base.is_none() {
        return Ok(pull_request);
    }

    Ok(github
        .update_pull_request(&context.origin.github, pull_request.number, update)
        .await?)
}

fn pull_request_body_matches(existing: Option<&str>, desired: &str) -> bool {
    if desired.is_empty() {
        existing.unwrap_or_default().is_empty()
    } else {
        existing == Some(desired)
    }
}

/// Returns an error when callers choose to block on conflicts created by fetch.
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
