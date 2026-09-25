use super::*;
use crate::github::AuthoredPullRequestInventory;
use tokio::sync::OnceCell;

/// Refresh-local discovery and client reuse, isolated by credential source rather than login.
pub(super) struct AuthoredPullRequestDiscovery<C> {
    scopes: Mutex<BTreeMap<String, ScopeCell<C>>>,
}

type ScopeCell<C> = Arc<OnceCell<Result<Arc<AuthoredPullRequestScope<C>>, String>>>;

impl<C: GitHubClient> AuthoredPullRequestDiscovery<C> {
    /// Starts a fresh inventory cache for one global stack refresh.
    pub(super) fn new() -> Self {
        Self {
            scopes: Mutex::new(BTreeMap::new()),
        }
    }

    /// Initializes each credential scope once, sharing both failures and successful discoveries.
    pub(super) async fn scope(
        &self,
        token_source: &TokenSource,
        create_client: impl FnOnce() -> Result<C, GitHubError>,
    ) -> Result<Arc<AuthoredPullRequestScope<C>>, String> {
        let cell = {
            let mut scopes = self
                .scopes
                .lock()
                .expect("authored PR scope lock is not poisoned");
            Arc::clone(
                scopes
                    .entry(token_source.cache_key())
                    .or_insert_with(|| Arc::new(OnceCell::new())),
            )
        };
        cell.get_or_init(|| async {
            let github = create_client().map_err(|error| error.to_string())?;
            // Failed or partial bulk reads are not authoritative negative results.
            let inventory = github.authored_pull_request_inventory().await.ok();
            Ok(Arc::new(AuthoredPullRequestScope { github, inventory }))
        })
        .await
        .clone()
    }
}

pub(super) struct AuthoredPullRequestScope<C> {
    pub(super) github: C,
    inventory: Option<AuthoredPullRequestInventory>,
}

impl<C: GitHubClient> AuthoredPullRequestScope<C> {
    /// Loads repository PRs from a complete inventory, or uses the existing repository fallback.
    pub(super) async fn for_repository(
        &self,
        repository: &GitHubRepository,
    ) -> Result<Vec<PullRequestRecord>, String> {
        if let Some(inventory) = &self.inventory {
            return Ok(inventory.for_repository(repository).to_vec());
        }
        let author = self
            .github
            .authenticated_user()
            .await
            .map_err(|error| error.to_string())?;
        if author.login.is_empty() {
            return Err(WorkflowError::MissingGitHubLogin.to_string());
        }
        self.github
            .authored_open_pull_requests(repository, &author.login)
            .await
            .map_err(|error| error.to_string())
    }

    /// Skips only authoritative empty repositories without local work requiring refresh or cleanup.
    pub(super) fn can_skip_repository(
        &self,
        repository: &GitHubRepository,
        has_local_stack: bool,
    ) -> bool {
        !has_local_stack
            && self
                .inventory
                .as_ref()
                .is_some_and(|inventory| inventory.for_repository(repository).is_empty())
    }

    /// Identifies the discovery path for refresh diagnostics.
    pub(super) fn discovery_mode(&self) -> &'static str {
        if self.inventory.is_some() {
            "bulk"
        } else {
            "repository-fallback"
        }
    }
}
