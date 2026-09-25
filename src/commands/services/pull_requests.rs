use super::*;

/// Loads PR facts through the shared snapshot store for stack and review workflows.
pub(super) struct PullRequestService<'a, G: ?Sized> {
    pub(super) environment: &'a RuntimeEnvironment,
    pub(super) github: &'a G,
}

impl<G> PullRequestService<'_, G>
where
    G: GitHubClient + ?Sized,
{
    /// Loads current PR records through the shared local snapshot store.
    pub(super) async fn pull_requests(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, WorkflowError> {
        self.refresh_pull_request_snapshots(repository, numbers)
            .await
    }

    /// Loads current PR records together with derived history and local actions.
    pub(super) async fn pull_requests_with_history(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestWithHistory>, WorkflowError> {
        let fetched = self
            .refresh_pull_request_snapshots(repository, numbers)
            .await?;
        let fetched_numbers = fetched
            .iter()
            .map(|status| status.number)
            .collect::<Vec<_>>();
        let store = PullRequestStore::open(self.environment)?;
        Ok(store.latest_pull_requests_with_history(repository, &fetched_numbers)?)
    }

    /// Refreshes stale snapshots in bounded batches, retaining progress on later failures.
    async fn refresh_pull_request_snapshots(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, WorkflowError> {
        let requested_numbers = unique_pull_request_numbers(numbers);
        if requested_numbers.is_empty() {
            return Ok(Vec::new());
        }
        let store = PullRequestStore::open(self.environment)?;
        let summaries = self
            .github
            .pull_request_update_summaries(repository, &requested_numbers)
            .await?;
        let refresh_plan =
            pull_request_refresh_plan(&store, repository, &requested_numbers, &summaries)?;
        for numbers in refresh_plan
            .numbers_to_fetch
            .chunks(PULL_REQUEST_STATUS_BATCH_SIZE)
        {
            let fetched = self
                .github
                .pull_request_statuses(repository, numbers)
                .await?;
            store.record_pull_request_snapshots_with_updates(
                repository,
                &fetched,
                &refresh_plan.github_updated_at_by_number,
            )?;
        }
        store
            .latest_pull_request_snapshots(repository, &refresh_plan.available_numbers)
            .map_err(WorkflowError::from)
    }
}

struct PullRequestRefreshPlan {
    available_numbers: Vec<u64>,
    github_updated_at_by_number: BTreeMap<u64, i64>,
    numbers_to_fetch: Vec<u64>,
}

struct PullRequestRefreshMetadata {
    schema_version: i64,
    github_updated_at_unix: Option<i64>,
}

fn pull_request_refresh_plan(
    store: &PullRequestStore,
    repository: &GitHubRepository,
    requested_numbers: &[u64],
    summaries: &[PullRequestUpdateSummary],
) -> Result<PullRequestRefreshPlan, WorkflowError> {
    let summary_numbers = summaries
        .iter()
        .map(|summary| summary.number)
        .collect::<Vec<_>>();
    let available_numbers = if summaries.is_empty() {
        requested_numbers.to_vec()
    } else {
        summary_numbers
    };
    let stored_metadata = store
        .latest_pull_request_snapshot_metadata(repository, &available_numbers)?
        .into_iter()
        .map(|metadata| {
            (
                metadata.number,
                PullRequestRefreshMetadata {
                    schema_version: metadata.schema_version,
                    github_updated_at_unix: metadata.github_updated_at_unix,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let stored_statuses = store
        .latest_pull_request_snapshots(repository, &available_numbers)?
        .into_iter()
        .map(|status| (status.number, status))
        .collect::<BTreeMap<_, _>>();
    let github_updated_at_by_number = summaries
        .iter()
        .filter_map(|summary| {
            review_timestamp_unix(&summary.updated_at).map(|timestamp| (summary.number, timestamp))
        })
        .collect::<BTreeMap<_, _>>();
    let numbers_to_fetch = if summaries.is_empty() {
        available_numbers.clone()
    } else {
        summaries
            .iter()
            .filter_map(|summary| {
                let updated_at_unix = github_updated_at_by_number.get(&summary.number).copied()?;
                let metadata = stored_metadata.get(&summary.number);
                let needs_current_schema = metadata.is_none_or(|metadata| {
                    metadata.schema_version < PULL_REQUEST_SNAPSHOT_SCHEMA_VERSION
                });
                let github_updated = metadata.and_then(|metadata| metadata.github_updated_at_unix)
                    != Some(updated_at_unix);
                let stored_status = stored_statuses.get(&summary.number);
                let head_changed = stored_status
                    .and_then(|status| status.latest_commit_oid.as_deref())
                    != summary.latest_commit_oid.as_deref();
                let checks_changed = stored_status.is_none_or(|status| {
                    pull_request_check_summary(&status.checks)
                        != pull_request_check_summary(&summary.checks)
                });
                // Approvals can change while updatedAt and the aggregate review decision stay fixed.
                let reviews_changed = stored_status
                    .and_then(|status| status.review_refresh_key.as_deref())
                    != Some(summary.review_refresh_key.as_str());
                (needs_current_schema
                    || github_updated
                    || head_changed
                    || checks_changed
                    || reviews_changed)
                    .then_some(summary.number)
            })
            .chain(summaries.iter().filter_map(|summary| {
                (!github_updated_at_by_number.contains_key(&summary.number))
                    .then_some(summary.number)
            }))
            .collect::<Vec<_>>()
    };
    Ok(PullRequestRefreshPlan {
        available_numbers,
        github_updated_at_by_number,
        numbers_to_fetch,
    })
}

fn pull_request_check_summary(
    checks: &[PullRequestCheck],
) -> BTreeMap<&str, (PullRequestCheckStatus, bool)> {
    checks
        .iter()
        .map(|check| (check.name.as_str(), (check.status, check.required)))
        .collect()
}

pub(super) fn review_timestamp_unix(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp())
}
