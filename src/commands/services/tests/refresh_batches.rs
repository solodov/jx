use super::*;

#[test]
fn old_review_snapshots_refresh_in_small_batches_then_use_warm_cache() {
    let temp = tempfile::tempdir().unwrap();
    let (environment, repository, github) = old_review_snapshot_fixture(temp.path());
    let service = PullRequestService {
        environment: &environment,
        github: &github,
    };
    let runtime = test_github_runtime();
    let numbers = (1..=16).collect::<Vec<_>>();

    for _ in 0..2 {
        let loaded = runtime
            .block_on(service.pull_requests(&repository, &numbers))
            .unwrap();
        assert_eq!(loaded, *github.statuses.lock().unwrap());
    }

    let expected = numbers.chunks(3).map(<[u64]>::to_vec).collect::<Vec<_>>();
    assert_eq!(*github.status_requests.lock().unwrap(), expected);
    assert_eq!(
        github
            .calls
            .pull_request_update_summaries
            .load(Ordering::Relaxed),
        2
    );
}

#[test]
fn failed_refresh_retains_completed_batches_and_only_retries_stale_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let (environment, repository, github) = old_review_snapshot_fixture(temp.path());
    *github.status_failure_number.lock().unwrap() = Some(4);
    let client = traced_client(github.clone());
    let service = PullRequestService {
        environment: &environment,
        github: &client,
    };
    let runtime = test_github_runtime();
    let numbers = (1..=16).collect::<Vec<_>>();

    let error = runtime
        .block_on(service.pull_requests(&repository, &numbers))
        .expect_err("second batch fails after retries");
    assert!(error.to_string().contains("Timed out"));
    let mut expected = vec![vec![1, 2, 3], vec![4, 5, 6], vec![4, 5, 6]];
    assert_eq!(*github.status_requests.lock().unwrap(), expected);

    let persisted = PullRequestStore::open(&environment)
        .unwrap()
        .latest_pull_request_snapshots(&repository, &numbers)
        .unwrap();
    assert_eq!(persisted.len(), numbers.len());
    for status in persisted {
        assert_eq!(
            status.review_refresh_key.as_deref(),
            (status.number <= 3).then_some("unchanged-reviews")
        );
    }

    *github.status_failure_number.lock().unwrap() = None;
    for _ in 0..2 {
        let loaded = runtime
            .block_on(service.pull_requests(&repository, &numbers))
            .unwrap();
        assert_eq!(loaded, *github.statuses.lock().unwrap());
    }
    expected.extend(numbers[3..].chunks(3).map(<[u64]>::to_vec));
    assert_eq!(*github.status_requests.lock().unwrap(), expected);
}

#[test]
fn traced_status_requests_share_the_small_batch_limit_and_deduplicate_numbers() {
    let github = CountingGitHub::default();
    *github.statuses.lock().unwrap() = (1..=7)
        .map(|number| test_pull_request_status(number, "Review"))
        .collect();
    let client = traced_client(github.clone());
    let repository = GitHubRepository {
        owner: "owner".to_owned(),
        name: "repo".to_owned(),
    };
    let runtime = test_github_runtime();

    assert!(runtime
        .block_on(client.pull_request_statuses(&repository, &[]))
        .unwrap()
        .is_empty());
    assert!(github.status_requests.lock().unwrap().is_empty());
    let loaded = runtime
        .block_on(client.pull_request_statuses(&repository, &[7, 2, 4, 1, 2, 6, 3, 5, 7]))
        .unwrap();

    assert_eq!(
        loaded
            .iter()
            .map(|status| status.number)
            .collect::<Vec<_>>(),
        vec![7, 2, 4, 1, 6, 3, 5]
    );
    assert_eq!(
        *github.status_requests.lock().unwrap(),
        vec![vec![7, 2, 4], vec![1, 6, 3], vec![5]]
    );
    assert_eq!(PULL_REQUEST_STATUS_BATCH_SIZE, 3);
    assert_eq!(pull_request_status_chunk_count(0), 0);
    assert_eq!(pull_request_status_chunk_count(3), 1);
    assert_eq!(pull_request_status_chunk_count(7), 3);
}

fn traced_client(github: CountingGitHub) -> TracedGitHubClient<CountingGitHub> {
    TracedGitHubClient {
        inner: github,
        perf: PerfLog::disabled(),
        repo: "owner/repo".to_owned(),
        cache: Arc::new(Mutex::new(GitHubFactCache::default())),
        durable_auth_cache: None,
    }
}

fn old_review_snapshot_fixture(
    root: &Path,
) -> (RuntimeEnvironment, GitHubRepository, CountingGitHub) {
    let environment =
        RuntimeEnvironment::new(root, [("HOME".to_owned(), root.display().to_string())]);
    let repository = GitHubRepository {
        owner: "owner".to_owned(),
        name: "repo".to_owned(),
    };
    let github = CountingGitHub::default();
    let statuses = (1..=16)
        .map(|number| test_pull_request_status(number, "Cached review"))
        .collect::<Vec<_>>();
    let old_statuses = statuses
        .iter()
        .cloned()
        .map(|mut status| {
            status.review_refresh_key = None;
            status
        })
        .collect::<Vec<_>>();
    let updated_at = "2026-01-01T00:00:00Z";
    PullRequestStore::open(&environment)
        .unwrap()
        .record_pull_request_snapshots_with_updates(
            &repository,
            &old_statuses,
            &statuses
                .iter()
                .map(|status| (status.number, review_timestamp_unix(updated_at).unwrap()))
                .collect(),
        )
        .unwrap();
    *github.update_summaries.lock().unwrap() = statuses
        .iter()
        .map(|status| PullRequestUpdateSummary {
            number: status.number,
            updated_at: updated_at.to_owned(),
            latest_commit_oid: status.latest_commit_oid.clone(),
            checks: status.checks.clone(),
            review_refresh_key: status.review_refresh_key.clone().unwrap(),
        })
        .collect();
    *github.statuses.lock().unwrap() = statuses;
    (environment, repository, github)
}
