use super::*;

#[test]
fn concurrent_repositories_share_one_client_and_complete_inventory() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let github = CountingGitHub {
        inventory_gate: Some(Arc::clone(&gate)),
        ..CountingGitHub::default()
    };
    *github.inventory.lock().unwrap() = Some(inventory(true));
    let discovery = AuthoredPullRequestDiscovery::new();
    let source = TokenSource::Environment("GH_TOKEN");
    let builds = AtomicUsize::new(0);

    test_github_runtime().block_on(async {
        let mut pending = Box::pin(async {
            futures::join!(
                discovery.scope(&source, || {
                    builds.fetch_add(1, Ordering::Relaxed);
                    Ok(github.clone())
                }),
                discovery.scope(&source, || {
                    builds.fetch_add(1, Ordering::Relaxed);
                    Ok(github.clone())
                }),
            )
        });
        assert!(futures::poll!(pending.as_mut()).is_pending());
        assert_eq!(builds.load(Ordering::Relaxed), 1);
        assert_eq!(
            github
                .calls
                .authored_pull_request_inventory
                .load(Ordering::Relaxed),
            1
        );
        gate.add_permits(1);
        let (first, second) = pending.await;
        let first = first.unwrap();
        let second = second.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.discovery_mode(), "bulk");
        assert_eq!(
            first
                .for_repository(&repository("active"))
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(second
            .for_repository(&repository("empty"))
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            github
                .calls
                .authored_open_pull_requests
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(github.calls.authenticated_user.load(Ordering::Relaxed), 0);
    });
}

#[test]
fn complete_inventory_skips_only_empty_repositories_without_local_metadata() {
    let github = CountingGitHub::default();
    *github.inventory.lock().unwrap() = Some(inventory(true));
    let discovery = AuthoredPullRequestDiscovery::new();
    test_github_runtime().block_on(async {
        let scope = discovery
            .scope(&TokenSource::Environment("GH_TOKEN"), || Ok(github))
            .await
            .unwrap();
        assert!(scope.can_skip_repository(&repository("empty"), false));
        assert!(!scope.can_skip_repository(&repository("empty"), true));
        // Newly authored PRs remain discoverable without local bookmarks or metadata.
        assert!(!scope.can_skip_repository(&repository("active"), false));
        assert!(!scope.can_skip_repository(&repository("ACTIVE"), false));
    });
}

#[test]
fn failed_bulk_discovery_falls_back_without_treating_repositories_as_empty() {
    let github = CountingGitHub::default();
    let repo = repository("active");
    github.authored.lock().unwrap().insert(
        repo.clone(),
        vec![test_pull_request(&repo, 7, "topic", "main")],
    );
    let discovery = AuthoredPullRequestDiscovery::new();
    test_github_runtime().block_on(async {
        let scope = discovery
            .scope(&TokenSource::Environment("GH_TOKEN"), || Ok(github.clone()))
            .await
            .unwrap();
        assert_eq!(scope.discovery_mode(), "repository-fallback");
        assert!(!scope.can_skip_repository(&repository("active"), false));
        assert!(!scope.can_skip_repository(&repository("empty"), false));
        assert_eq!(scope.for_repository(&repo).await.unwrap()[0].number, 7);
        assert!(scope
            .for_repository(&repository("empty"))
            .await
            .unwrap()
            .is_empty());
        let repeated = discovery
            .scope(&TokenSource::Environment("GH_TOKEN"), || {
                panic!("client must be reused")
            })
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&scope, &repeated));
        assert_eq!(
            github
                .calls
                .authored_pull_request_inventory
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            github
                .calls
                .authored_open_pull_requests
                .load(Ordering::Relaxed),
            2
        );
    });
}

#[test]
fn fallback_failures_remain_errors_instead_of_silent_skips() {
    let github = CountingGitHub {
        authored_error: Some("repository access denied"),
        ..CountingGitHub::default()
    };
    let discovery = AuthoredPullRequestDiscovery::new();
    test_github_runtime().block_on(async {
        let scope = discovery
            .scope(&TokenSource::Environment("GH_TOKEN"), || Ok(github))
            .await
            .unwrap();
        assert!(!scope.can_skip_repository(&repository("active"), false));
        let error = scope
            .for_repository(&repository("active"))
            .await
            .unwrap_err();
        assert!(error.contains("repository access denied"));
    });
}

#[test]
fn same_viewer_with_different_credential_sources_gets_separate_inventories() {
    let first = CountingGitHub::default();
    *first.inventory.lock().unwrap() = Some(inventory(true));
    let second = CountingGitHub::default();
    *second.inventory.lock().unwrap() = Some(inventory(false));
    let discovery = AuthoredPullRequestDiscovery::new();
    test_github_runtime().block_on(async {
        let first = discovery
            .scope(&TokenSource::Environment("GH_TOKEN"), || Ok(first.clone()))
            .await
            .unwrap();
        let second = discovery
            .scope(&TokenSource::Environment("GITHUB_TOKEN"), || {
                Ok(second.clone())
            })
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!first.can_skip_repository(&repository("active"), false));
        assert!(second.can_skip_repository(&repository("active"), false));
    });
    assert_eq!(
        first
            .calls
            .authored_pull_request_inventory
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        second
            .calls
            .authored_pull_request_inventory
            .load(Ordering::Relaxed),
        1
    );
}

#[test]
fn failed_client_initialization_is_shared_only_within_its_authentication_scope() {
    let discovery = AuthoredPullRequestDiscovery::new();
    let builds = AtomicUsize::new(0);
    test_github_runtime().block_on(async {
        for _ in 0..2 {
            let result = discovery
                .scope(&TokenSource::Missing, || {
                    builds.fetch_add(1, Ordering::Relaxed);
                    Err(GitHubError::MissingToken)
                })
                .await;
            assert!(result.is_err());
        }
        let github = CountingGitHub::default();
        *github.inventory.lock().unwrap() = Some(inventory(false));
        assert!(discovery
            .scope(&TokenSource::Environment("GH_TOKEN"), || Ok(github))
            .await
            .is_ok());
    });
    assert_eq!(builds.load(Ordering::Relaxed), 1);
}

#[test]
fn new_refresh_rechecks_previously_empty_repositories() {
    let github = CountingGitHub::default();
    *github.inventory.lock().unwrap() = Some(inventory(false));
    test_github_runtime().block_on(async {
        for active in [false, true] {
            *github.inventory.lock().unwrap() = Some(inventory(active));
            let discovery = AuthoredPullRequestDiscovery::new();
            let scope = discovery
                .scope(&TokenSource::Environment("GH_TOKEN"), || Ok(github.clone()))
                .await
                .unwrap();
            assert_eq!(
                scope.can_skip_repository(&repository("active"), false),
                !active
            );
        }
    });
    assert_eq!(
        github
            .calls
            .authored_pull_request_inventory
            .load(Ordering::Relaxed),
        2
    );
}

#[test]
fn traced_inventory_reuses_the_viewer_identity_for_later_requests() {
    let github = CountingGitHub::default();
    *github.inventory.lock().unwrap() = Some(inventory(false));
    let client = TracedGitHubClient {
        inner: github.clone(),
        perf: PerfLog::disabled(),
        repo: "stack-discovery".to_owned(),
        cache: Arc::new(Mutex::new(GitHubFactCache::default())),
        durable_auth_cache: None,
    };
    test_github_runtime().block_on(async {
        client.authored_pull_request_inventory().await.unwrap();
        assert_eq!(
            client.authenticated_user().await.unwrap().login,
            "example-user"
        );
    });
    assert_eq!(github.calls.authenticated_user.load(Ordering::Relaxed), 0);
}

fn repository(name: &str) -> GitHubRepository {
    GitHubRepository {
        owner: "owner".to_owned(),
        name: name.to_owned(),
    }
}

fn inventory(active: bool) -> AuthoredPullRequestInventory {
    let repo = repository("active");
    AuthoredPullRequestInventory {
        viewer: AuthenticatedUser {
            login: "example-user".to_owned(),
        },
        repositories: if active {
            BTreeMap::from([(
                repo.clone(),
                vec![test_pull_request(&repo, 7, "topic", "main")],
            )])
        } else {
            BTreeMap::new()
        },
    }
}
