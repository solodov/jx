use super::*;
use std::cell::RefCell;
use tokio::sync::Semaphore;

#[test]
fn repository_loads_overlap_but_results_and_progress_stay_stable() {
    let temp = tempfile::tempdir().unwrap();
    let environment = test_environment(temp.path());
    let (github, probe, targets) = parallel_fixture();
    let service = PullRequestService::new(&environment, &github, PullRequestFetchBudget::new(2));
    let progress = RefCell::new(Vec::new());

    test_github_runtime().block_on(async {
        let mut load = Box::pin(service.load_many(&targets, |done, total| {
            progress.borrow_mut().push((done, total));
        }));
        assert!(futures::poll!(load.as_mut()).is_pending());
        assert_eq!(probe.active.load(Ordering::Relaxed), 2);
        assert_eq!(probe.started.lock().unwrap().len(), 2);

        // Complete B and C while A is still waiting on GitHub.
        for name in ["b", "c"] {
            probe.release(name, "summary");
            assert!(futures::poll!(load.as_mut()).is_pending());
            probe.release(name, "details");
            assert!(futures::poll!(load.as_mut()).is_pending());
        }
        assert_eq!(*progress.borrow(), vec![(1, 3), (2, 3)]);
        let stored = PullRequestStore::open(&environment)
            .unwrap()
            .latest_pull_request_snapshots(&repository("b"), &[12])
            .unwrap();
        assert_eq!(stored[0].title, "b");

        probe.release("a", "summary");
        probe.release("a", "details");
        let loaded = load.await;
        assert_eq!(loaded.len(), 3);
        for (entry, name) in loaded.into_iter().zip(["a", "b", "c"]) {
            assert_eq!(entry.repository, repository(name));
            let pull_requests = entry.result.unwrap();
            assert_eq!(pull_requests.len(), 1);
            assert_eq!(pull_requests[0].status.title, name);
            assert_eq!(pull_requests[0].history[0].kind, "first_seen");
        }
    });
    assert_eq!(*progress.borrow(), vec![(1, 3), (2, 3), (3, 3)]);
    assert_eq!(probe.maximum.load(Ordering::Relaxed), 2);
    assert_eq!(probe.active.load(Ordering::Relaxed), 0);
}

#[test]
fn stack_style_single_loads_and_review_batches_share_one_request_budget() {
    let temp = tempfile::tempdir().unwrap();
    let environment = test_environment(temp.path());
    let (github, probe, mut targets) = parallel_fixture();
    let stack_repository = repository("c");
    let numbers = targets.remove(&stack_repository).unwrap();
    let budget = PullRequestFetchBudget::new(2);
    let stack = PullRequestService::new(&environment, &github, budget.clone());
    let review = PullRequestService::new(&environment, &github, budget);

    test_github_runtime().block_on(async {
        let mut loads = Box::pin(async {
            futures::join!(
                stack.pull_requests(&stack_repository, &numbers),
                review.load_many(&targets, |_, _| {})
            )
        });
        assert!(futures::poll!(loads.as_mut()).is_pending());
        assert_eq!(probe.active.load(Ordering::Relaxed), 2);
        assert_eq!(probe.started.lock().unwrap().len(), 2);

        // A queued repository must share the budget with the next detail request.
        probe.release("c", "summary");
        assert!(futures::poll!(loads.as_mut()).is_pending());
        assert_eq!(probe.active.load(Ordering::Relaxed), 2);
        probe.release_all();
        let (stack, review) = loads.await;
        assert_eq!(stack.unwrap()[0].title, "c");
        assert_eq!(review.len(), 2);
        assert!(review.iter().all(|entry| entry.result.is_ok()));
    });
    assert_eq!(probe.maximum.load(Ordering::Relaxed), 2);
}

#[test]
fn repository_failure_does_not_discard_other_results_or_saved_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let environment = test_environment(temp.path());
    let (github, probe, mut targets) = parallel_fixture();
    targets.insert(repository("b"), vec![7]);
    *github.status_failure_number.lock().unwrap() = Some(7);
    probe.release_all();
    let service = PullRequestService::new(&environment, &github, PullRequestFetchBudget::default());
    let mut progress = Vec::new();
    let loaded = test_github_runtime().block_on(service.load_many(&targets, |done, total| {
        progress.push((done, total));
    }));

    assert_eq!(loaded.len(), 3);
    assert!(loaded[0].result.is_ok());
    assert!(loaded[1].result.is_err());
    assert!(loaded[2].result.is_ok());
    assert_eq!(progress, vec![(1, 3), (2, 3), (3, 3)]);
    let store = PullRequestStore::open(&environment).unwrap();
    for name in ["a", "c"] {
        let saved = store
            .latest_pull_request_snapshots(&repository(name), &[12])
            .unwrap();
        assert_eq!(saved[0].title, name);
    }
    assert!(store
        .latest_pull_request_snapshots(&repository("b"), &[7])
        .unwrap()
        .is_empty());
    assert_eq!(probe.active.load(Ordering::Relaxed), 0);
}

#[test]
fn cancelling_a_load_releases_its_shared_request_permit() {
    let temp = tempfile::tempdir().unwrap();
    let environment = test_environment(temp.path());
    let (github, probe, _) = parallel_fixture();
    // Zero parallelism is normalized rather than deadlocking.
    let service = PullRequestService::new(&environment, &github, PullRequestFetchBudget::new(0));
    let repository = repository("a");
    test_github_runtime().block_on(async {
        let mut cancelled = Box::pin(service.pull_requests(&repository, &[12]));
        assert!(futures::poll!(cancelled.as_mut()).is_pending());
        assert_eq!(probe.active.load(Ordering::Relaxed), 1);
        drop(cancelled);
        assert_eq!(probe.active.load(Ordering::Relaxed), 0);

        let mut next = Box::pin(service.pull_requests(&repository, &[12]));
        assert!(futures::poll!(next.as_mut()).is_pending());
        assert_eq!(probe.started.lock().unwrap().len(), 2);
        probe.release_all();
        assert_eq!(next.await.unwrap()[0].title, "a");
    });
    assert_eq!(probe.maximum.load(Ordering::Relaxed), 1);
}

#[test]
fn production_cached_loads_need_no_token_and_do_not_refresh_old_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let environment = test_environment(temp.path());
    let services = ProductionServices::new(&environment).unwrap();
    let mut old = test_pull_request_status(12, "Cached");
    old.review_refresh_key = None;
    let store = PullRequestStore::open(&environment).unwrap();
    store
        .record_pull_request_snapshots(&repository("a"), &[old.clone()])
        .unwrap();
    let targets = BTreeMap::from([
        (repository("a"), vec![12, 12, 99]),
        (repository("b"), vec![12]),
    ]);
    let loaded = services
        .load_pull_requests(
            &environment,
            &targets,
            PullRequestLoadSource::CachedOnly,
            &SilentProgress,
        )
        .unwrap();

    assert_eq!(loaded.len(), 2);
    let mut entries = loaded.into_iter();
    let cached = entries.next().unwrap().result.unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].status, old);
    assert_eq!(cached[0].history.len(), 1);
    assert!(entries.next().unwrap().result.unwrap().is_empty());
    let after = store
        .latest_pull_requests_with_history(&repository("a"), &[12])
        .unwrap();
    assert_eq!(after[0].status, old);
    assert_eq!(after[0].history.len(), cached[0].history.len());
}

#[test]
fn empty_repository_loads_do_not_fetch_or_report_progress() {
    let temp = tempfile::tempdir().unwrap();
    let environment = test_environment(temp.path());
    let github = CountingGitHub::default();
    let service = PullRequestService::new(&environment, &github, PullRequestFetchBudget::default());
    let loaded = test_github_runtime().block_on(service.load_many(&BTreeMap::new(), |_, _| {
        panic!("empty load has no progress");
    }));
    assert!(loaded.is_empty());
    assert_eq!(
        github
            .calls
            .pull_request_update_summaries
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(
        github.calls.pull_request_statuses.load(Ordering::Relaxed),
        0
    );
}

fn test_environment(root: &Path) -> RuntimeEnvironment {
    RuntimeEnvironment::new(root, [("HOME".to_owned(), root.display().to_string())])
}

fn repository(name: &str) -> GitHubRepository {
    GitHubRepository {
        owner: "owner".to_owned(),
        name: name.to_owned(),
    }
}

fn parallel_fixture() -> (
    CountingGitHub,
    Arc<RequestProbe>,
    BTreeMap<GitHubRepository, Vec<u64>>,
) {
    let probe = Arc::new(RequestProbe::default());
    let github = CountingGitHub {
        request_probe: Some(Arc::clone(&probe)),
        ..CountingGitHub::default()
    };
    let mut targets = BTreeMap::new();
    for name in ["a", "b", "c"] {
        let repository = repository(name);
        github
            .repository_statuses
            .lock()
            .unwrap()
            .insert(repository.clone(), vec![test_pull_request_status(12, name)]);
        targets.insert(repository, vec![12, 12]);
    }
    (github, probe, targets)
}

/// Explicit gates keep concurrency assertions independent of wall-clock timing.
#[derive(Default)]
pub(super) struct RequestProbe {
    active: AtomicUsize,
    maximum: AtomicUsize,
    started: Mutex<Vec<(String, &'static str)>>,
    gates: Mutex<BTreeMap<(String, &'static str), Arc<Semaphore>>>,
}

impl RequestProbe {
    pub(super) async fn begin(
        &self,
        repository: &GitHubRepository,
        kind: &'static str,
    ) -> RequestGuard<'_> {
        let active = self.active.fetch_add(1, Ordering::Relaxed) + 1;
        self.maximum.fetch_max(active, Ordering::Relaxed);
        self.started
            .lock()
            .unwrap()
            .push((repository.name.clone(), kind));
        let guard = RequestGuard(self);
        self.gate(&repository.name, kind)
            .acquire()
            .await
            .unwrap()
            .forget();
        guard
    }

    fn release(&self, repository: &str, kind: &'static str) {
        self.gate(repository, kind).add_permits(1);
    }

    fn release_all(&self) {
        for repository in ["a", "b", "c"] {
            for kind in ["summary", "details"] {
                self.release(repository, kind);
            }
        }
    }

    fn gate(&self, repository: &str, kind: &'static str) -> Arc<Semaphore> {
        Arc::clone(
            self.gates
                .lock()
                .unwrap()
                .entry((repository.to_owned(), kind))
                .or_insert_with(|| Arc::new(Semaphore::new(0))),
        )
    }
}

pub(super) struct RequestGuard<'a>(&'a RequestProbe);

impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}
