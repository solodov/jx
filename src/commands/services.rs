use super::*;
use crate::github::{
    AuthenticatedUser, CommitComparison, GitHubError, LabelApplyResult, PullRequestCreate,
    PullRequestStatusRecord, PullRequestUpdate, RepositoryAccess, RepositoryFork,
    ReviewerSyncResult,
};
use futures::{stream, StreamExt};
use std::sync::{Arc, Mutex};

pub(super) trait CommandServices {
    /// Renders the no-argument workspace log.
    fn workspace_log(&self) -> Result<String, JjError>;

    /// Shows the current jj diff, optionally constraining it to non-test files.
    fn current_diff(&self, current_dir: &Path, options: &DiffOptions) -> Result<String, JjError>;

    /// Moves to the previous commit in the active chain and renders the new position.
    fn previous_commit_log(&self, current_dir: &Path) -> Result<String, JjError>;

    /// Moves to the next commit in the active chain and renders the new position.
    fn next_commit_log(&self, current_dir: &Path) -> Result<String, JjError>;

    /// Clones a Git repository through jj into the resolved layout destination.
    fn clone_repository(&self, current_dir: &Path, plan: &ClonePlan) -> Result<(), JjError>;

    /// Initializes a layout-resolved directory as a Git-backed jj repository.
    fn init_repository(&self, current_dir: &Path) -> Result<(), JjError>;

    /// Adds a jj workspace at the resolved hidden layout destination.
    fn add_workspace(
        &self,
        current_dir: &Path,
        options: &WorkspaceAddOptions,
    ) -> Result<(), JjError>;

    /// Lists jj workspaces with root paths and current-workspace state.
    fn workspace_entries(&self, current_dir: &Path) -> Result<Vec<WorkspaceEntry>, JjError>;

    /// Returns the current workspace without resolving sibling workspace paths.
    fn current_workspace_entry(&self, current_dir: &Path) -> Result<WorkspaceEntry, JjError>;

    /// Forgets a jj workspace and deletes its managed directory.
    fn remove_workspace(
        &self,
        current_dir: &Path,
        options: &WorkspaceRemoveOptions,
    ) -> Result<(), JjError>;

    /// Selects the local commit that should seed a newly created remote repository.
    fn initial_publish_target(
        &self,
        workspace_root: &Path,
    ) -> Result<InitialPublishTarget, JjError>;

    /// Rewrites a fresh undescribed initial commit before GitHub repository bootstrap.
    fn prepare_initial_publish_target(
        &self,
        workspace_root: &Path,
        target: &InitialPublishTarget,
    ) -> Result<InitialPublishTarget, JjError>;

    /// Creates a private GitHub repository for missing-origin bootstrap.
    fn create_repository(
        &self,
        context: &LocalRepositoryContext,
        repository: &GitHubRepository,
    ) -> Result<RepositoryCreation, WorkflowError>;

    /// Adds origin/main locally and pushes the initial branch after GitHub creation.
    fn bootstrap_origin_main(
        &self,
        workspace_root: &Path,
        remote_url: &str,
        target: &InitialPublishTarget,
    ) -> Result<BootstrapPushOutcome, JjError>;

    /// Loads the current working-copy status block shared by status and PR preview.
    fn workspace_status(&self, current_dir: &Path, color: bool)
        -> Result<WorkspaceStatus, JjError>;

    /// Rewrites a selected commit description and returns the replacement commit id.
    fn rewrite_commit_description(
        &self,
        context: &RepositoryContext,
        target_commit_id: &str,
        description: &str,
    ) -> Result<CommitDescriptionRewrite, JjError>;

    /// Loads jj facts for the working copy or an explicitly selected revision.
    fn workspace_facts(
        &self,
        context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError>;

    /// Loads push planning facts, allowing existing local bookmarks before origin trunk exists.
    fn push_workspace_facts(
        &self,
        context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError>;

    /// Runs non-mutating local and GitHub readiness checks.
    fn check_readiness(
        &self,
        context: &RepositoryContext,
        workspace: WorkspaceFacts,
    ) -> Result<CheckReport, WorkflowError>;

    /// Loads local cached trunk facts for every configured GitHub remote.
    fn status_workspace_facts(
        &self,
        context: &RepositoryContext,
    ) -> Result<StatusWorkspaceFacts, JjError>;

    /// Compares local cached remote-trunk state with live GitHub remotes.
    fn status_report(
        &self,
        context: &RepositoryContext,
        workspace: StatusWorkspaceFacts,
    ) -> Result<StatusReport, WorkflowError>;

    /// Builds the full `remote-status` report, including source/fork freshness.
    fn remote_status_report(
        &self,
        context: &RepositoryContext,
        workspace: StatusWorkspaceFacts,
    ) -> Result<StatusReport, WorkflowError> {
        self.status_report(context, workspace)
    }

    /// Returns whether the current token can push to the fixed origin repository.
    fn origin_can_push(&self, context: &RepositoryContext) -> Result<bool, WorkflowError>;

    /// Returns the authenticated GitHub login used for authored PR filters.
    fn authenticated_login(&self, token_source: &TokenSource) -> Result<String, WorkflowError>;

    /// Lists local bookmark heads that can have associated pull requests.
    fn pull_request_bookmarks(&self, context: &RepositoryContext) -> Result<Vec<String>, JjError>;

    /// Lists PR bookmark heads for the selected change, commit selector, or exact local bookmark.
    fn pull_request_candidate_bookmarks(
        &self,
        context: &RepositoryContext,
        selector: Option<&str>,
    ) -> Result<Vec<String>, JjError>;

    /// Finds an open pull request by same-repository bookmark head and author.
    fn find_authored_open_pull_request_for_head(
        &self,
        context: &RepositoryContext,
        branch: &str,
        author: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError>;

    /// Finds an open GitHub pull request for a same-repository bookmark head.
    fn find_open_pull_request_for_head(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError>;

    /// Finds the most recent GitHub pull request for a same-repository bookmark head.
    fn find_pull_request_for_head(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError>;

    /// Finds a GitHub pull request by durable repository-local PR number.
    fn find_pull_request_by_number(
        &self,
        context: &RepositoryContext,
        number: u64,
    ) -> Result<Option<PullRequestRecord>, WorkflowError>;

    /// Finds several GitHub pull requests by durable repository-local PR number.
    fn find_pull_requests_by_numbers(
        &self,
        context: &RepositoryContext,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestRecord>, WorkflowError> {
        let mut pull_requests = Vec::new();
        for number in unique_pull_request_numbers(numbers) {
            if let Some(pull_request) = self.find_pull_request_by_number(context, number)? {
                pull_requests.push(pull_request);
            }
        }
        Ok(pull_requests)
    }

    /// Loads batched read-only GitHub status facts for stack pull requests.
    fn pull_request_statuses(
        &self,
        context: &RepositoryContext,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, WorkflowError>;

    /// Opens a URL in the platform default browser.
    fn open_url(&self, url: &str) -> io::Result<()>;

    /// Loads global stack-status rows, preserving layout order and skipping repositories without stack metadata.
    fn global_stack_status_entries(
        &self,
        repositories: &[WorkRepository],
        request: &StackStatusRequest,
        environment: &RuntimeEnvironment,
        progress: &dyn ProgressSink,
    ) -> Vec<GlobalStackStatusEntry> {
        let mut entries = Vec::new();

        for (index, repository) in repositories.iter().enumerate() {
            let entry = stack_status_entry_for_repository(
                repository,
                environment,
                |context, numbers| {
                    self.pull_request_statuses(context, numbers)
                        .map_err(CommandError::from)
                        .map_err(|error| error.to_string())
                },
                |context, branch| {
                    self.find_open_pull_request_for_head(context, branch)
                        .map_err(CommandError::from)
                        .map_err(|error| error.to_string())
                },
                |context| {
                    let workspace = self
                        .status_workspace_facts(context)
                        .map_err(CommandError::from)
                        .map_err(|error| error.to_string())?;
                    let report = self
                        .status_report(context, workspace)
                        .map_err(CommandError::from)
                        .map_err(|error| error.to_string())?;
                    Ok(domain::origin_status_report(context, report))
                },
            );
            if let Some(entry) = entry {
                entries.push(entry);
            }
            progress.percentage("Checking stack status", index + 1, repositories.len());
        }

        let _ = request;
        entries
    }

    /// Loads global remote-status rows, preserving layout order and filtering clean repos when requested.
    fn global_remote_status_entries(
        &self,
        repositories: &[WorkRepository],
        request: &RemoteStatusRequest,
        environment: &RuntimeEnvironment,
        progress: &dyn ProgressSink,
    ) -> Vec<GlobalStatusEntry> {
        let mut entries = Vec::new();

        for (index, repository) in repositories.iter().enumerate() {
            let resolved: Result<(GitHubRepository, StatusReport), CommandError> = (|| {
                let environment = environment.with_current_dir(&repository.root);
                let context = RepositoryContext::discover(&environment)?;
                let origin = context.origin.github.clone();
                let workspace = self.status_workspace_facts(&context)?;
                Ok((origin, self.remote_status_report(&context, workspace)?))
            })();
            let (repository_identity, result) = match resolved {
                Ok((repository, report)) => (Some(repository), Ok(report)),
                Err(error) => (None, Err(error.to_string())),
            };
            if !(request.changed
                && result
                    .as_ref()
                    .is_ok_and(|report| !status_report_has_changes(report)))
            {
                entries.push(GlobalStatusEntry {
                    key: Some(repository.key.clone()),
                    root: repository.root.clone(),
                    display_root: display_path(&repository.root, environment),
                    repository: repository_identity,
                    result,
                });
            }
            progress.percentage("Checking remote status", index + 1, repositories.len());
        }

        entries
    }

    /// Returns whether global fetch can safely mutate this repository without touching local work.
    fn global_fetch_ready(&self, context: &RepositoryContext) -> Result<bool, JjError>;

    /// Fetches origin and applies jj stack repair/rebase behavior.
    fn fetch_origin(&self, context: &RepositoryContext) -> Result<FetchOutcome, JjError>;

    /// Rebases selected source revisions and descendants onto the fixed origin trunk.
    fn rebase_on_trunk(
        &self,
        context: &RepositoryContext,
        sources: &[String],
    ) -> Result<RebaseOnTrunkOutcome, JjError>;

    /// Moves the current change and descendants onto a stack target or trunk.
    fn move_current_stack(
        &self,
        context: &RepositoryContext,
        target: &StackMoveTarget,
    ) -> Result<StackMoveOutcome, JjError>;

    /// Reads local branch ancestry from jj for stack metadata repair.
    fn local_stack_branches(
        &self,
        context: &RepositoryContext,
    ) -> Result<Vec<LocalStackBranch>, JjError>;

    /// Reads local branch ancestry with jj-internal timing metrics for perf tracing.
    fn local_stack_branch_facts(
        &self,
        context: &RepositoryContext,
    ) -> Result<LocalStackBranchFacts, JjError> {
        self.local_stack_branches(context)
            .map(LocalStackBranchFacts::from_branches)
    }

    /// Reads the local stack selected for PR publishing.
    fn stack_publish_facts(
        &self,
        context: &RepositoryContext,
        selection: &StackPublishSelection,
    ) -> Result<StackPublishFacts, JjError>;

    /// Reads the local stack neighbourhood selected for read-only planning.
    fn stack_plan_facts(
        &self,
        context: &RepositoryContext,
        selection: &StackPlanSelection,
    ) -> Result<StackPlanFacts, JjError>;

    /// Ensures the selected PR bookmark points at the selected jj commit.
    fn ensure_bookmark(
        &self,
        context: &RepositoryContext,
        branch: &str,
        target_commit_id: &str,
    ) -> Result<BookmarkUpdate, JjError>;

    /// Ensures selected PR bookmarks point at their selected jj commits.
    fn ensure_bookmarks(
        &self,
        context: &RepositoryContext,
        targets: &[(String, String)],
    ) -> Result<Vec<BookmarkUpdate>, JjError> {
        targets
            .iter()
            .map(|(branch, commit_id)| self.ensure_bookmark(context, branch, commit_id))
            .collect()
    }

    /// Pushes the selected bookmark through the jj Git transport boundary.
    fn push_bookmark(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<PushOutcome, JjError>;

    /// Pushes selected bookmarks and returns jj-internal timing metrics for perf tracing.
    fn push_bookmarks_with_metrics(
        &self,
        context: &RepositoryContext,
        branches: &[String],
    ) -> Result<PushBookmarksOutcome, JjError> {
        branches
            .iter()
            .map(|branch| self.push_bookmark(context, branch))
            .collect::<Result<Vec<_>, _>>()
            .map(PushBookmarksOutcome::from_outcomes)
    }

    /// Optionally prepares sync by advancing the local trunk bookmark to current work.
    fn advance_trunk_for_sync(
        &self,
        context: &RepositoryContext,
    ) -> Result<AdvanceTrunkOutcome, JjError>;

    /// Pushes all tracked fixed-origin bookmarks, including deleted bookmarks.
    fn push_tracked(&self, context: &RepositoryContext) -> Result<TrackedPushOutcome, JjError>;

    /// Pushes one selected bookmarked revision when its update does not contain conflicted commits.
    fn push_syncable_revision(
        &self,
        context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<SyncPushOutcome, JjError>;

    /// Pushes tracked bookmarks whose updates do not contain conflicted commits.
    fn push_syncable_tracked(
        &self,
        context: &RepositoryContext,
    ) -> Result<SyncPushOutcome, JjError>;

    /// Pushes syncable tracked bookmarks and returns jj-internal timing metrics.
    fn push_syncable_tracked_with_metrics(
        &self,
        context: &RepositoryContext,
    ) -> Result<SyncPushMetricsOutcome, JjError> {
        self.push_syncable_tracked(context)
            .map(SyncPushMetricsOutcome::from_outcome)
    }

    /// Syncs PR descriptions for tracked bookmark updates and returns PR metadata rendered by sync.
    fn sync_pull_requests(
        &self,
        context: &RepositoryContext,
        push: &TrackedPushOutcome,
        stack_metadata: &StackMetadata,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError>;

    /// Builds PR metadata and bookmark intent before mutation.
    fn pull_request_plan(
        &self,
        context: &RepositoryContext,
        workspace: WorkspaceFacts,
        task_id: Option<String>,
        labels: Vec<String>,
        readiness: PullRequestReadiness,
    ) -> Result<PullRequestPlan, WorkflowError>;

    /// Creates or updates the GitHub PR after local bookmark state has been pushed.
    fn publish_pull_request(
        &self,
        context: &RepositoryContext,
        plan: PullRequestPlan,
        bookmark_update: BookmarkUpdate,
        push: PushOutcome,
        options: PullRequestPublishOptions,
    ) -> Result<PullRequestReport, WorkflowError>;

    /// Updates only PR metadata when the selected branch already matches GitHub.
    fn publish_pull_request_metadata_only(
        &self,
        context: &RepositoryContext,
        plan: PullRequestPlan,
        bookmark_update: BookmarkUpdate,
        push: PushOutcome,
    ) -> Result<PullRequestReport, WorkflowError>;
}

/// Production command boundary backed by real jj and GitHub clients.
pub(super) struct ProductionServices<'environment> {
    environment: &'environment RuntimeEnvironment,
    pub(super) github_runtime: tokio::runtime::Runtime,
    github_cache: Arc<Mutex<GitHubFactCache>>,
}

impl<'environment> ProductionServices<'environment> {
    /// Builds production services with a Tokio runtime for octocrab's background HTTP tasks.
    pub(super) fn new(environment: &'environment RuntimeEnvironment) -> io::Result<Self> {
        let github_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        Ok(Self {
            environment,
            github_runtime,
            github_cache: Arc::new(Mutex::new(GitHubFactCache::default())),
        })
    }

    fn traced_github_client(
        &self,
        context: &RepositoryContext,
    ) -> Result<TracedGitHubClient<OctocrabGitHubClient>, GitHubError> {
        let perf = PerfLog::from_environment(self.environment);
        let repo = context.origin.github.slug();
        let mut span = perf.start("github.client", [perf_attr("repo", repo.clone())]);
        let result =
            OctocrabGitHubClient::from_token_source(&context.token_source, self.environment);
        if let Err(error) = &result {
            span.record_error(error);
        }
        span.end();
        result.map(|inner| TracedGitHubClient {
            inner,
            perf,
            repo,
            cache: Arc::clone(&self.github_cache),
        })
    }
}

#[derive(Debug, Default)]
struct GitHubFactCache {
    authenticated_user: Option<AuthenticatedUser>,
    repository_access_by_slug: BTreeMap<String, RepositoryAccess>,
    authored_open_pull_request_by_head:
        BTreeMap<(String, String, String), Option<PullRequestRecord>>,
    open_pull_request_by_head: BTreeMap<(String, String), Option<PullRequestRecord>>,
    pull_request_by_head: BTreeMap<(String, String), Option<PullRequestRecord>>,
    pull_request_by_number: BTreeMap<(String, u64), Option<PullRequestRecord>>,
}

struct TracedGitHubClient<C> {
    inner: C,
    perf: PerfLog,
    repo: String,
    cache: Arc<Mutex<GitHubFactCache>>,
}

impl<C> TracedGitHubClient<C> {
    fn start_span(
        &self,
        op: &'static str,
        repository: Option<&GitHubRepository>,
        attrs: impl IntoIterator<Item = PerfAttr>,
    ) -> PerfSpan {
        let mut attrs = attrs.into_iter().collect::<Vec<_>>();
        attrs.push(perf_attr(
            "repo",
            repository.map_or_else(|| self.repo.clone(), GitHubRepository::slug),
        ));
        self.perf.start(op, attrs)
    }

    fn finish<T>(
        &self,
        mut span: PerfSpan,
        result: Result<T, GitHubError>,
        attrs: impl IntoIterator<Item = PerfAttr>,
    ) -> Result<T, GitHubError> {
        span.set(attrs);
        if let Err(error) = &result {
            span.record_error(error);
        }
        span.end();
        result
    }

    fn head_key(repository: &GitHubRepository, head: &PullRequestHead) -> (String, String) {
        (repository.slug(), head.label())
    }

    fn record_head_key(
        repository: &GitHubRepository,
        pull_request: &PullRequestRecord,
    ) -> (String, String) {
        let head = PullRequestHead::same_repository(&repository.owner, &pull_request.head_branch);
        Self::head_key(repository, &head)
    }

    fn number_key(repository: &GitHubRepository, number: u64) -> (String, u64) {
        (repository.slug(), number)
    }

    fn cache_open_pull_request_lookup(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
        result: &Result<Option<PullRequestRecord>, GitHubError>,
    ) {
        let Ok(pull_request) = result else {
            return;
        };
        let mut cache = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned");
        cache
            .open_pull_request_by_head
            .insert(Self::head_key(repository, head), pull_request.clone());
        if let Some(pull_request) = pull_request {
            cache.pull_request_by_number.insert(
                Self::number_key(repository, pull_request.number),
                Some(pull_request.clone()),
            );
        }
    }

    fn cache_authored_open_pull_request_lookup(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
        author: &str,
        result: &Result<Option<PullRequestRecord>, GitHubError>,
    ) {
        let Ok(pull_request) = result else {
            return;
        };
        let mut cache = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned");
        let (repo, head_label) = Self::head_key(repository, head);
        cache.authored_open_pull_request_by_head.insert(
            (repo.clone(), head_label.clone(), author.to_owned()),
            pull_request.clone(),
        );
        cache
            .open_pull_request_by_head
            .insert((repo, head_label), pull_request.clone());
        if let Some(pull_request) = pull_request {
            cache.pull_request_by_number.insert(
                Self::number_key(repository, pull_request.number),
                Some(pull_request.clone()),
            );
        }
    }

    fn cache_pull_request_for_head_lookup(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
        result: &Result<Option<PullRequestRecord>, GitHubError>,
    ) {
        let Ok(pull_request) = result else {
            return;
        };
        let mut cache = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned");
        cache
            .pull_request_by_head
            .insert(Self::head_key(repository, head), pull_request.clone());
        if let Some(pull_request) = pull_request {
            cache.pull_request_by_number.insert(
                Self::number_key(repository, pull_request.number),
                Some(pull_request.clone()),
            );
        }
    }

    fn cache_pull_request_by_number_lookup(
        &self,
        repository: &GitHubRepository,
        number: u64,
        result: &Result<Option<PullRequestRecord>, GitHubError>,
    ) {
        let Ok(pull_request) = result else {
            return;
        };
        let mut cache = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned");
        cache
            .pull_request_by_number
            .insert(Self::number_key(repository, number), pull_request.clone());
        if let Some(pull_request) = pull_request {
            let head_key = Self::record_head_key(repository, pull_request);
            cache
                .pull_request_by_head
                .insert(head_key.clone(), Some(pull_request.clone()));
            if !pull_request.merged {
                cache
                    .open_pull_request_by_head
                    .insert(head_key, Some(pull_request.clone()));
            }
        }
    }

    fn cache_mutated_pull_request(
        &self,
        repository: &GitHubRepository,
        result: &Result<PullRequestRecord, GitHubError>,
    ) {
        let Ok(pull_request) = result else {
            return;
        };
        let mut cache = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned");
        cache.pull_request_by_number.insert(
            Self::number_key(repository, pull_request.number),
            Some(pull_request.clone()),
        );
        cache.open_pull_request_by_head.insert(
            Self::record_head_key(repository, pull_request),
            Some(pull_request.clone()),
        );
        cache.pull_request_by_head.insert(
            Self::record_head_key(repository, pull_request),
            Some(pull_request.clone()),
        );
    }
}

fn cached_pull_request_lookup_result(
    value: Option<PullRequestRecord>,
) -> Result<Option<PullRequestRecord>, GitHubError> {
    Ok(value)
}

fn cached_pull_request_lookup_attrs(
    result: &Result<Option<PullRequestRecord>, GitHubError>,
) -> Vec<PerfAttr> {
    let mut attrs = pull_request_lookup_attrs(result);
    attrs.push(perf_attr("cache_hit", true));
    attrs
}

fn uncached_pull_request_lookup_attrs(
    result: &Result<Option<PullRequestRecord>, GitHubError>,
) -> Vec<PerfAttr> {
    let mut attrs = pull_request_lookup_attrs(result);
    attrs.push(perf_attr("cache_hit", false));
    attrs
}

fn unique_pull_request_numbers(numbers: &[u64]) -> Vec<u64> {
    let mut seen = BTreeSet::new();
    numbers
        .iter()
        .copied()
        .filter(|number| seen.insert(*number))
        .collect()
}

#[async_trait::async_trait]
impl<C> GitHubClient for TracedGitHubClient<C>
where
    C: GitHubClient,
{
    async fn authenticated_user(&self) -> Result<AuthenticatedUser, GitHubError> {
        let span = self.start_span("github.authenticated_user", None, Vec::new());
        if let Some(user) = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned")
            .authenticated_user
            .clone()
        {
            return self.finish(span, Ok(user), [perf_attr("cache_hit", true)]);
        }

        let result = self.inner.authenticated_user().await;
        if let Ok(user) = &result {
            self.cache
                .lock()
                .expect("GitHub fact cache lock is not poisoned")
                .authenticated_user = Some(user.clone());
        }
        self.finish(span, result, [perf_attr("cache_hit", false)])
    }

    async fn repository_access(
        &self,
        repository: &GitHubRepository,
    ) -> Result<RepositoryAccess, GitHubError> {
        let span = self.start_span("github.repository_access", Some(repository), Vec::new());
        let slug = repository.slug();
        if let Some(access) = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned")
            .repository_access_by_slug
            .get(&slug)
            .cloned()
        {
            return self.finish(span, Ok(access), [perf_attr("cache_hit", true)]);
        }

        let result = self.inner.repository_access(repository).await;
        if let Ok(access) = &result {
            self.cache
                .lock()
                .expect("GitHub fact cache lock is not poisoned")
                .repository_access_by_slug
                .insert(slug, access.clone());
        }
        self.finish(span, result, [perf_attr("cache_hit", false)])
    }

    async fn repository_fork(
        &self,
        repository: &GitHubRepository,
    ) -> Result<Option<RepositoryFork>, GitHubError> {
        let span = self.start_span("github.repository_fork", Some(repository), Vec::new());
        let result = self.inner.repository_fork(repository).await;
        let attrs = result
            .as_ref()
            .map(|fork| vec![perf_attr("found", fork.is_some())])
            .unwrap_or_default();
        self.finish(span, result, attrs)
    }

    async fn create_repository(
        &self,
        repository: &GitHubRepository,
        private: bool,
    ) -> Result<RepositoryCreation, GitHubError> {
        let span = self.start_span(
            "github.create_repository",
            Some(repository),
            [perf_attr("private", private)],
        );
        let result = self.inner.create_repository(repository, private).await;
        self.finish(span, result, Vec::new())
    }

    async fn compare_commits(
        &self,
        repository: &GitHubRepository,
        base: &str,
        head: &str,
    ) -> Result<CommitComparison, GitHubError> {
        let span = self.start_span(
            "github.compare_commits",
            Some(repository),
            [perf_attr("base", base), perf_attr("head", head)],
        );
        let result = self.inner.compare_commits(repository, base, head).await;
        self.finish(span, result, Vec::new())
    }

    async fn find_authored_open_pull_request_for_head(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
        author: &str,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        let span = self.start_span(
            "github.find_authored_open_pull_request_for_head",
            Some(repository),
            [
                perf_attr("head", head.label()),
                perf_attr("head_branch", &head.branch),
                perf_attr("author", author),
            ],
        );
        let key = {
            let (repo, head_label) = Self::head_key(repository, head);
            (repo, head_label, author.to_owned())
        };
        if let Some(cached) = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned")
            .authored_open_pull_request_by_head
            .get(&key)
            .cloned()
        {
            let result = cached_pull_request_lookup_result(cached);
            let attrs = cached_pull_request_lookup_attrs(&result);
            return self.finish(span, result, attrs);
        }

        let result = self
            .inner
            .find_authored_open_pull_request_for_head(repository, head, author)
            .await;
        self.cache_authored_open_pull_request_lookup(repository, head, author, &result);
        let attrs = uncached_pull_request_lookup_attrs(&result);
        self.finish(span, result, attrs)
    }

    async fn find_open_pull_request(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        let span = self.start_span(
            "github.find_open_pull_request",
            Some(repository),
            [
                perf_attr("head", head.label()),
                perf_attr("head_branch", &head.branch),
            ],
        );
        let key = Self::head_key(repository, head);
        if let Some(cached) = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned")
            .open_pull_request_by_head
            .get(&key)
            .cloned()
        {
            let result = cached_pull_request_lookup_result(cached);
            let attrs = cached_pull_request_lookup_attrs(&result);
            return self.finish(span, result, attrs);
        }

        let result = self.inner.find_open_pull_request(repository, head).await;
        self.cache_open_pull_request_lookup(repository, head, &result);
        let attrs = uncached_pull_request_lookup_attrs(&result);
        self.finish(span, result, attrs)
    }

    async fn find_pull_request_for_head(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        let span = self.start_span(
            "github.find_pull_request_for_head",
            Some(repository),
            [
                perf_attr("head", head.label()),
                perf_attr("head_branch", &head.branch),
            ],
        );
        let key = Self::head_key(repository, head);
        if let Some(cached) = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned")
            .pull_request_by_head
            .get(&key)
            .cloned()
        {
            let result = cached_pull_request_lookup_result(cached);
            let attrs = cached_pull_request_lookup_attrs(&result);
            return self.finish(span, result, attrs);
        }

        let result = self
            .inner
            .find_pull_request_for_head(repository, head)
            .await;
        self.cache_pull_request_for_head_lookup(repository, head, &result);
        let attrs = uncached_pull_request_lookup_attrs(&result);
        self.finish(span, result, attrs)
    }

    async fn find_pull_request_by_number(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        let span = self.start_span(
            "github.find_pull_request_by_number",
            Some(repository),
            [perf_attr("number", number)],
        );
        let key = Self::number_key(repository, number);
        if let Some(cached) = self
            .cache
            .lock()
            .expect("GitHub fact cache lock is not poisoned")
            .pull_request_by_number
            .get(&key)
            .cloned()
        {
            let result = cached_pull_request_lookup_result(cached);
            let attrs = cached_pull_request_lookup_attrs(&result);
            return self.finish(span, result, attrs);
        }

        let result = self
            .inner
            .find_pull_request_by_number(repository, number)
            .await;
        self.cache_pull_request_by_number_lookup(repository, number, &result);
        let attrs = uncached_pull_request_lookup_attrs(&result);
        self.finish(span, result, attrs)
    }

    async fn find_pull_requests_by_numbers(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestRecord>, GitHubError> {
        let requested_numbers = unique_pull_request_numbers(numbers);
        let span = self.start_span(
            "github.find_pull_requests_by_numbers",
            Some(repository),
            [perf_attr("number_count", requested_numbers.len())],
        );
        let (mut pull_requests_by_number, missing_numbers) = {
            let cache = self
                .cache
                .lock()
                .expect("GitHub fact cache lock is not poisoned");
            let mut pull_requests_by_number = BTreeMap::new();
            let mut missing_numbers = Vec::new();
            for number in &requested_numbers {
                match cache
                    .pull_request_by_number
                    .get(&Self::number_key(repository, *number))
                    .cloned()
                {
                    Some(Some(pull_request)) => {
                        pull_requests_by_number.insert(*number, pull_request);
                    }
                    Some(None) => {}
                    None => missing_numbers.push(*number),
                }
            }
            (pull_requests_by_number, missing_numbers)
        };

        if !missing_numbers.is_empty() {
            let loaded_pull_requests = match self
                .inner
                .find_pull_requests_by_numbers(repository, &missing_numbers)
                .await
            {
                Ok(pull_requests) => pull_requests,
                Err(error) => return self.finish(span, Err(error), Vec::new()),
            };
            for number in &missing_numbers {
                let pull_request = loaded_pull_requests
                    .iter()
                    .find(|pull_request| pull_request.number == *number)
                    .cloned();
                self.cache_pull_request_by_number_lookup(repository, *number, &Ok(pull_request));
            }
            for pull_request in loaded_pull_requests {
                pull_requests_by_number.insert(pull_request.number, pull_request);
            }
        }

        let pull_requests = requested_numbers
            .iter()
            .filter_map(|number| pull_requests_by_number.get(number).cloned())
            .collect::<Vec<_>>();
        self.finish(
            span,
            Ok(pull_requests),
            [
                perf_attr(
                    "cache_hit_count",
                    requested_numbers.len() - missing_numbers.len(),
                ),
                perf_attr("miss_count", missing_numbers.len()),
                perf_attr("found_count", pull_requests_by_number.len()),
            ],
        )
    }

    async fn pull_request_statuses(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, GitHubError> {
        let span = self.start_span(
            "github.pull_request_statuses",
            Some(repository),
            [
                perf_attr("number_count", numbers.len()),
                perf_attr(
                    "chunk_count",
                    pull_request_status_chunk_count(numbers.len()),
                ),
            ],
        );
        let result = self.inner.pull_request_statuses(repository, numbers).await;
        let attrs = result
            .as_ref()
            .map(|statuses| vec![perf_attr("status_count", statuses.len())])
            .unwrap_or_default();
        self.finish(span, result, attrs)
    }

    async fn create_pull_request(
        &self,
        repository: &GitHubRepository,
        request: PullRequestCreate,
    ) -> Result<PullRequestRecord, GitHubError> {
        let span = self.start_span(
            "github.create_pull_request",
            Some(repository),
            [
                perf_attr("head", request.head.label()),
                perf_attr("head_branch", &request.head.branch),
                perf_attr("base", &request.base),
                perf_attr("draft", request.draft),
            ],
        );
        let result = self.inner.create_pull_request(repository, request).await;
        self.cache_mutated_pull_request(repository, &result);
        let attrs = pull_request_record_attrs(&result);
        self.finish(span, result, attrs)
    }

    async fn update_pull_request(
        &self,
        repository: &GitHubRepository,
        number: u64,
        request: PullRequestUpdate,
    ) -> Result<PullRequestRecord, GitHubError> {
        let span = self.start_span(
            "github.update_pull_request",
            Some(repository),
            [
                perf_attr("number", number),
                perf_attr("update_title", request.title.is_some()),
                perf_attr("update_body", request.body.is_some()),
                perf_attr("update_base", request.base.is_some()),
            ],
        );
        let result = self
            .inner
            .update_pull_request(repository, number, request)
            .await;
        self.cache_mutated_pull_request(repository, &result);
        let attrs = pull_request_record_attrs(&result);
        self.finish(span, result, attrs)
    }

    async fn mark_pull_request_ready(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<PullRequestRecord, GitHubError> {
        let span = self.start_span(
            "github.mark_pull_request_ready",
            Some(repository),
            [perf_attr("number", number)],
        );
        let result = self.inner.mark_pull_request_ready(repository, number).await;
        self.cache_mutated_pull_request(repository, &result);
        let attrs = pull_request_record_attrs(&result);
        self.finish(span, result, attrs)
    }

    async fn convert_pull_request_to_draft(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<PullRequestRecord, GitHubError> {
        let span = self.start_span(
            "github.convert_pull_request_to_draft",
            Some(repository),
            [perf_attr("number", number)],
        );
        let result = self
            .inner
            .convert_pull_request_to_draft(repository, number)
            .await;
        self.cache_mutated_pull_request(repository, &result);
        let attrs = pull_request_record_attrs(&result);
        self.finish(span, result, attrs)
    }

    async fn pull_request_labels(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<Vec<String>, GitHubError> {
        let span = self.start_span(
            "github.pull_request_labels",
            Some(repository),
            [perf_attr("number", number)],
        );
        let result = self.inner.pull_request_labels(repository, number).await;
        let attrs = result
            .as_ref()
            .map(|labels| vec![perf_attr("label_count", labels.len())])
            .unwrap_or_default();
        self.finish(span, result, attrs)
    }

    async fn add_labels(
        &self,
        repository: &GitHubRepository,
        number: u64,
        labels: Vec<String>,
    ) -> Result<LabelApplyResult, GitHubError> {
        let span = self.start_span(
            "github.add_labels",
            Some(repository),
            [
                perf_attr("number", number),
                perf_attr("label_count", labels.len()),
            ],
        );
        let result = self.inner.add_labels(repository, number, labels).await;
        self.finish(span, result, Vec::new())
    }

    async fn sync_reviewers(
        &self,
        repository: &GitHubRepository,
        number: u64,
        desired: ReviewerSelection,
    ) -> Result<ReviewerSyncResult, GitHubError> {
        let span = self.start_span(
            "github.sync_reviewers",
            Some(repository),
            [
                perf_attr("number", number),
                perf_attr("user_count", desired.users.len()),
                perf_attr("team_count", desired.teams.len()),
            ],
        );
        let result = self.inner.sync_reviewers(repository, number, desired).await;
        self.finish(span, result, Vec::new())
    }
}

fn pull_request_status_chunk_count(number_count: usize) -> usize {
    if number_count == 0 {
        0
    } else {
        number_count.div_ceil(50)
    }
}

fn pull_request_lookup_attrs(
    result: &Result<Option<PullRequestRecord>, GitHubError>,
) -> Vec<PerfAttr> {
    match result {
        Ok(Some(pull_request)) => vec![
            perf_attr("found", true),
            perf_attr("number", pull_request.number),
            perf_attr("head_branch", &pull_request.head_branch),
            perf_attr("base", &pull_request.base_branch),
        ],
        Ok(None) => vec![perf_attr("found", false)],
        Err(_) => Vec::new(),
    }
}

fn pull_request_record_attrs(result: &Result<PullRequestRecord, GitHubError>) -> Vec<PerfAttr> {
    match result {
        Ok(pull_request) => vec![
            perf_attr("number", pull_request.number),
            perf_attr("head_branch", &pull_request.head_branch),
            perf_attr("base", &pull_request.base_branch),
        ],
        Err(_) => Vec::new(),
    }
}

impl CommandServices for ProductionServices<'_> {
    fn workspace_log(&self) -> Result<String, JjError> {
        JjWorkspace::current_workspace_log(self.environment.current_dir())
    }

    fn current_diff(&self, current_dir: &Path, options: &DiffOptions) -> Result<String, JjError> {
        run_current_diff(current_dir, options)?;
        Ok(String::new())
    }

    fn previous_commit_log(&self, current_dir: &Path) -> Result<String, JjError> {
        JjWorkspace::move_to_previous_commit_and_render_log(current_dir)
    }

    fn next_commit_log(&self, current_dir: &Path) -> Result<String, JjError> {
        JjWorkspace::move_to_next_commit_and_render_log(current_dir)
    }

    fn clone_repository(&self, current_dir: &Path, plan: &ClonePlan) -> Result<(), JjError> {
        run_jj_git_clone(current_dir, &plan.remote_url, &plan.destination)
    }

    fn init_repository(&self, current_dir: &Path) -> Result<(), JjError> {
        run_jj_git_init(current_dir)
    }

    fn add_workspace(
        &self,
        current_dir: &Path,
        options: &WorkspaceAddOptions,
    ) -> Result<(), JjError> {
        run_jj_workspace_add(current_dir, options)
    }

    fn workspace_entries(&self, current_dir: &Path) -> Result<Vec<WorkspaceEntry>, JjError> {
        jj_workspace_entries(current_dir)
    }

    fn current_workspace_entry(&self, current_dir: &Path) -> Result<WorkspaceEntry, JjError> {
        current_workspace_entry(current_dir)
    }

    fn remove_workspace(
        &self,
        current_dir: &Path,
        options: &WorkspaceRemoveOptions,
    ) -> Result<(), JjError> {
        remove_jj_workspace(current_dir, options)
    }

    fn initial_publish_target(
        &self,
        workspace_root: &Path,
    ) -> Result<InitialPublishTarget, JjError> {
        JjWorkspace::load(workspace_root.to_path_buf())?.initial_publish_target()
    }

    fn prepare_initial_publish_target(
        &self,
        workspace_root: &Path,
        target: &InitialPublishTarget,
    ) -> Result<InitialPublishTarget, JjError> {
        JjWorkspace::load(workspace_root.to_path_buf())?.prepare_initial_publish_target(target)
    }

    fn create_repository(
        &self,
        context: &LocalRepositoryContext,
        repository: &GitHubRepository,
    ) -> Result<RepositoryCreation, WorkflowError> {
        self.github_runtime.block_on(async {
            let github =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)?;
            Ok(github.create_repository(repository, true).await?)
        })
    }

    fn bootstrap_origin_main(
        &self,
        workspace_root: &Path,
        remote_url: &str,
        target: &InitialPublishTarget,
    ) -> Result<BootstrapPushOutcome, JjError> {
        JjWorkspace::load(workspace_root.to_path_buf())?.bootstrap_origin_main(remote_url, target)
    }

    fn workspace_status(
        &self,
        current_dir: &Path,
        color: bool,
    ) -> Result<WorkspaceStatus, JjError> {
        JjWorkspace::current_status(current_dir, color)
    }

    fn rewrite_commit_description(
        &self,
        context: &RepositoryContext,
        target_commit_id: &str,
        description: &str,
    ) -> Result<CommitDescriptionRewrite, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?
            .rewrite_commit_description(target_commit_id, description)
    }

    fn workspace_facts(
        &self,
        context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.facts_for_revision(revision)
    }

    fn push_workspace_facts(
        &self,
        context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.push_facts_for_revision(revision)
    }

    fn check_readiness(
        &self,
        context: &RepositoryContext,
        workspace: WorkspaceFacts,
    ) -> Result<CheckReport, WorkflowError> {
        self.github_runtime.block_on(async {
            let github =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)?;

            domain::check_readiness(context, workspace, &github).await
        })
    }

    fn status_workspace_facts(
        &self,
        context: &RepositoryContext,
    ) -> Result<StatusWorkspaceFacts, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.status_facts(
            context
                .github_remotes
                .iter()
                .map(|remote| remote.name.as_str()),
        )
    }

    fn status_report(
        &self,
        context: &RepositoryContext,
        workspace: StatusWorkspaceFacts,
    ) -> Result<StatusReport, WorkflowError> {
        self.github_runtime.block_on(async {
            let github =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)?;

            domain::status_report(context, workspace, &github).await
        })
    }

    fn remote_status_report(
        &self,
        context: &RepositoryContext,
        workspace: StatusWorkspaceFacts,
    ) -> Result<StatusReport, WorkflowError> {
        self.github_runtime.block_on(async {
            let github =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)?;

            domain::remote_status_report(context, workspace, &github).await
        })
    }

    fn origin_can_push(&self, context: &RepositoryContext) -> Result<bool, WorkflowError> {
        self.github_runtime.block_on(async {
            let github =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)?;
            let access = github.repository_access(&context.origin.github).await?;
            if !access.can_read {
                return Err(WorkflowError::MissingReadAccess {
                    repository: context.origin.github.slug(),
                });
            }

            Ok(access.can_push)
        })
    }

    fn authenticated_login(&self, token_source: &TokenSource) -> Result<String, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = OctocrabGitHubClient::from_token_source(token_source, self.environment)?;
            let user = github.authenticated_user().await?;
            if user.login.is_empty() {
                return Err(WorkflowError::MissingGitHubLogin);
            }

            Ok(user.login)
        })
    }

    fn pull_request_bookmarks(&self, context: &RepositoryContext) -> Result<Vec<String>, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.pull_request_bookmarks()
    }

    fn pull_request_candidate_bookmarks(
        &self,
        context: &RepositoryContext,
        selector: Option<&str>,
    ) -> Result<Vec<String>, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?
            .pull_request_candidate_bookmarks(selector)
    }

    fn find_authored_open_pull_request_for_head(
        &self,
        context: &RepositoryContext,
        branch: &str,
        author: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;
            let head = PullRequestHead::same_repository(&context.origin.github.owner, branch);

            Ok(github
                .find_authored_open_pull_request_for_head(&context.origin.github, &head, author)
                .await?)
        })
    }

    fn find_open_pull_request_for_head(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;
            let head = PullRequestHead::same_repository(&context.origin.github.owner, branch);

            Ok(github
                .find_open_pull_request(&context.origin.github, &head)
                .await?)
        })
    }

    fn find_pull_request_for_head(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;
            let head = PullRequestHead::same_repository(&context.origin.github.owner, branch);

            Ok(github
                .find_pull_request_for_head(&context.origin.github, &head)
                .await?)
        })
    }

    fn find_pull_request_by_number(
        &self,
        context: &RepositoryContext,
        number: u64,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;

            Ok(github
                .find_pull_request_by_number(&context.origin.github, number)
                .await?)
        })
    }

    fn find_pull_requests_by_numbers(
        &self,
        context: &RepositoryContext,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;

            Ok(github
                .find_pull_requests_by_numbers(&context.origin.github, numbers)
                .await?)
        })
    }

    fn pull_request_statuses(
        &self,
        context: &RepositoryContext,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;

            Ok(github
                .pull_request_statuses(&context.origin.github, numbers)
                .await?)
        })
    }

    fn open_url(&self, url: &str) -> io::Result<()> {
        open_url_in_browser(url)
    }

    fn global_stack_status_entries(
        &self,
        repositories: &[WorkRepository],
        request: &StackStatusRequest,
        environment: &RuntimeEnvironment,
        progress: &dyn ProgressSink,
    ) -> Vec<GlobalStackStatusEntry> {
        let parallelism = request.parallelism.max(1);

        self.github_runtime.block_on(async {
            let mut stream = stream::iter(repositories.iter().enumerate().map(
                |(index, repository)| async move {
                    (
                        index,
                        production_global_stack_status_entry(
                            repository,
                            environment,
                            self.environment,
                        )
                        .await,
                    )
                },
            ))
            .buffer_unordered(parallelism);
            let mut completed = 0;
            let mut entries = Vec::new();

            while let Some((index, entry)) = stream.next().await {
                completed += 1;
                progress.percentage("Checking stack status", completed, repositories.len());
                if let Some(entry) = entry {
                    entries.push((index, entry));
                }
            }

            entries.sort_by_key(|(index, _)| *index);
            entries
                .into_iter()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>()
        })
    }

    fn global_remote_status_entries(
        &self,
        repositories: &[WorkRepository],
        request: &RemoteStatusRequest,
        environment: &RuntimeEnvironment,
        progress: &dyn ProgressSink,
    ) -> Vec<GlobalStatusEntry> {
        let parallelism = request.parallelism.max(1);
        let changed = request.changed;

        self.github_runtime.block_on(async {
            let mut stream = stream::iter(repositories.iter().enumerate().map(
                |(index, repository)| async move {
                    (
                        index,
                        production_global_remote_status_entry(
                            repository,
                            environment,
                            self.environment,
                            changed,
                        )
                        .await,
                    )
                },
            ))
            .buffer_unordered(parallelism);
            let mut completed = 0;
            let mut entries = Vec::new();

            while let Some((index, entry)) = stream.next().await {
                completed += 1;
                progress.percentage("Checking remote status", completed, repositories.len());
                if let Some(entry) = entry {
                    entries.push((index, entry));
                }
            }

            entries.sort_by_key(|(index, _)| *index);
            entries
                .into_iter()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>()
        })
    }

    fn global_fetch_ready(&self, context: &RepositoryContext) -> Result<bool, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?
            .is_empty_working_copy_child_of_origin_trunk()
    }

    fn fetch_origin(&self, context: &RepositoryContext) -> Result<FetchOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.fetch_origin()
    }

    fn rebase_on_trunk(
        &self,
        context: &RepositoryContext,
        sources: &[String],
    ) -> Result<RebaseOnTrunkOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.rebase_on_trunk(sources)
    }

    fn move_current_stack(
        &self,
        context: &RepositoryContext,
        target: &StackMoveTarget,
    ) -> Result<StackMoveOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.move_current_stack(target.clone())
    }

    fn local_stack_branches(
        &self,
        context: &RepositoryContext,
    ) -> Result<Vec<LocalStackBranch>, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.local_stack_branches()
    }

    fn local_stack_branch_facts(
        &self,
        context: &RepositoryContext,
    ) -> Result<LocalStackBranchFacts, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.local_stack_branch_facts()
    }

    fn stack_publish_facts(
        &self,
        context: &RepositoryContext,
        selection: &StackPublishSelection,
    ) -> Result<StackPublishFacts, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.stack_publish_facts(selection)
    }

    fn stack_plan_facts(
        &self,
        context: &RepositoryContext,
        selection: &StackPlanSelection,
    ) -> Result<StackPlanFacts, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.stack_plan_facts(selection)
    }

    fn ensure_bookmark(
        &self,
        context: &RepositoryContext,
        branch: &str,
        target_commit_id: &str,
    ) -> Result<BookmarkUpdate, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.ensure_bookmark(branch, target_commit_id)
    }

    fn ensure_bookmarks(
        &self,
        context: &RepositoryContext,
        targets: &[(String, String)],
    ) -> Result<Vec<BookmarkUpdate>, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.ensure_bookmarks(targets)
    }

    fn push_bookmark(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<PushOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.push_bookmark(branch)
    }

    fn push_bookmarks_with_metrics(
        &self,
        context: &RepositoryContext,
        branches: &[String],
    ) -> Result<PushBookmarksOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.push_bookmarks_with_metrics(branches)
    }

    fn advance_trunk_for_sync(
        &self,
        context: &RepositoryContext,
    ) -> Result<AdvanceTrunkOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.advance_trunk_for_sync()
    }

    fn push_tracked(&self, context: &RepositoryContext) -> Result<TrackedPushOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.push_tracked_deleted()
    }

    fn push_syncable_revision(
        &self,
        context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<SyncPushOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.push_syncable_revision(revision)
    }

    fn push_syncable_tracked(
        &self,
        context: &RepositoryContext,
    ) -> Result<SyncPushOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.push_syncable_tracked()
    }

    fn push_syncable_tracked_with_metrics(
        &self,
        context: &RepositoryContext,
    ) -> Result<SyncPushMetricsOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.push_syncable_tracked_with_metrics()
    }

    fn sync_pull_requests(
        &self,
        context: &RepositoryContext,
        push: &TrackedPushOutcome,
        stack_metadata: &StackMetadata,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError> {
        if push.bookmarks.is_empty() {
            return Ok(Vec::new());
        }

        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;
            domain::sync_pull_requests(context, push, stack_metadata, &github).await
        })
    }

    fn pull_request_plan(
        &self,
        context: &RepositoryContext,
        workspace: WorkspaceFacts,
        task_id: Option<String>,
        labels: Vec<String>,
        readiness: PullRequestReadiness,
    ) -> Result<PullRequestPlan, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;
            domain::pull_request_plan(context, workspace, &github, task_id, labels, readiness).await
        })
    }

    fn publish_pull_request(
        &self,
        context: &RepositoryContext,
        plan: PullRequestPlan,
        bookmark_update: BookmarkUpdate,
        push: PushOutcome,
        options: PullRequestPublishOptions,
    ) -> Result<PullRequestReport, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;
            domain::publish_pull_request(context, plan, bookmark_update, push, options, &github)
                .await
        })
    }

    fn publish_pull_request_metadata_only(
        &self,
        context: &RepositoryContext,
        plan: PullRequestPlan,
        bookmark_update: BookmarkUpdate,
        push: PushOutcome,
    ) -> Result<PullRequestReport, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;
            domain::publish_pull_request_metadata_only(
                context,
                plan,
                bookmark_update,
                push,
                &github,
            )
            .await
        })
    }
}

fn open_url_in_browser(url: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        ProcessCommand::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        ProcessCommand::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        ProcessCommand::new("xdg-open").arg(url).spawn()?;
    }

    Ok(())
}

fn stack_status_entry_for_repository(
    repository: &WorkRepository,
    environment: &RuntimeEnvironment,
    fetch_statuses: impl FnOnce(
        &RepositoryContext,
        &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, String>,
    mut resolve_pull_request: impl FnMut(
        &RepositoryContext,
        &str,
    ) -> Result<Option<PullRequestRecord>, String>,
    fetch_trunk: impl FnOnce(&RepositoryContext) -> Result<Option<RemoteStatusReport>, String>,
) -> Option<GlobalStackStatusEntry> {
    let display_root = display_path(&repository.root, environment);
    let metadata = match read_stack_metadata(&repository.root) {
        Ok(metadata) if metadata.nodes.is_empty() => return None,
        Ok(metadata) => metadata,
        Err(error) => {
            return Some(GlobalStackStatusEntry {
                key: Some(repository.key.clone()),
                root: repository.root.clone(),
                display_root,
                repository: None,
                result: Err(error.to_string()),
            });
        }
    };
    let repository_environment = environment.with_current_dir(&repository.root);
    let context = match RepositoryContext::discover(&repository_environment) {
        Ok(context) => context,
        Err(error) => {
            return Some(GlobalStackStatusEntry {
                key: Some(repository.key.clone()),
                root: repository.root.clone(),
                display_root,
                repository: None,
                result: Err(error.to_string()),
            });
        }
    };
    let repository_identity = context.origin.github.clone();
    let discovered_pull_requests =
        stack_status_missing_pull_requests_from_metadata(&metadata, |branch| {
            resolve_pull_request(&context, branch)
        });
    let numbers = discovered_pull_requests
        .as_ref()
        .map(|pull_requests| stack_status_numbers_from_metadata(&metadata, pull_requests))
        .unwrap_or_else(|_| stack_status_numbers_from_metadata(&metadata, &[]));
    let statuses = if numbers.is_empty() {
        Ok(Vec::new())
    } else {
        fetch_statuses(&context, &numbers)
    };
    let result = discovered_pull_requests.and(statuses).and_then(|statuses| {
        maintained_stack_status_report(&context, &repository.root, &metadata, statuses, || {
            fetch_trunk(&context)
        })
    });
    let result = match result {
        Ok(Some(report)) => Ok(report),
        Ok(None) => return None,
        Err(error) => Err(error),
    };

    Some(GlobalStackStatusEntry {
        key: Some(repository.key.clone()),
        root: repository.root.clone(),
        display_root,
        repository: Some(repository_identity),
        result,
    })
}

async fn production_global_stack_status_entry(
    repository: &WorkRepository,
    environment: &RuntimeEnvironment,
    token_environment: &RuntimeEnvironment,
) -> Option<GlobalStackStatusEntry> {
    let perf = PerfLog::from_environment(token_environment);
    let mut span = perf.start(
        "stack.status.repo",
        [
            perf_attr("repository_key", &repository.key),
            perf_attr("root", repository.root.display().to_string()),
        ],
    );
    let result = production_global_stack_status_entry_traced(
        repository,
        environment,
        token_environment,
        &perf,
        &mut span,
    )
    .await;
    if let Some(entry) = &result {
        span.set([
            perf_attr("has_stack_metadata", entry.result.is_ok()),
            perf_attr(
                "pr_count",
                entry
                    .result
                    .as_ref()
                    .map_or(0, |report| report.statuses.len()),
            ),
        ]);
        if let Err(error) = &entry.result {
            span.record_error(error);
        }
    } else {
        span.set([perf_attr("has_stack_metadata", false)]);
    }
    span.end();
    result
}

async fn production_global_stack_status_entry_traced(
    repository: &WorkRepository,
    environment: &RuntimeEnvironment,
    token_environment: &RuntimeEnvironment,
    perf: &PerfLog,
    span: &mut PerfSpan,
) -> Option<GlobalStackStatusEntry> {
    let display_root = display_path(&repository.root, environment);
    let prepare_step = span.start_step("load_stack_metadata", Vec::new());
    let prepared_result =
        prepare_global_stack_status(repository.root.clone(), environment.clone()).await;
    span.finish_step(prepare_step, Vec::new(), prepared_result.as_ref().err());
    let prepared = match prepared_result {
        Ok(Some(prepared)) => prepared,
        Ok(None) => return None,
        Err(error) => {
            return Some(GlobalStackStatusEntry {
                key: Some(repository.key.clone()),
                root: repository.root.clone(),
                display_root,
                repository: None,
                result: Err(error),
            });
        }
    };
    let (context, metadata, status_workspace) = prepared;
    let repository_identity = context.origin.github.clone();
    let github =
        match OctocrabGitHubClient::from_token_source(&context.token_source, token_environment)
            .map_err(WorkflowError::from)
            .map_err(CommandError::from)
            .map_err(|error| error.to_string())
        {
            Ok(github) => TracedGitHubClient {
                inner: github,
                perf: perf.clone(),
                repo: repository_identity.slug(),
                cache: Arc::new(Mutex::new(GitHubFactCache::default())),
            },
            Err(error) => {
                return Some(GlobalStackStatusEntry {
                    key: Some(repository.key.clone()),
                    root: repository.root.clone(),
                    display_root,
                    repository: Some(repository_identity),
                    result: Err(error),
                });
            }
        };
    let discover_step = span.start_step("discover_missing_pull_requests", Vec::new());
    let discovered_pull_requests = async {
        let mut pull_requests = Vec::new();
        for branch in stack_status_missing_pull_request_branches_from_metadata(&metadata) {
            let head = PullRequestHead::same_repository(&context.origin.github.owner, &branch);
            if let Some(pull_request) = github
                .find_open_pull_request(&context.origin.github, &head)
                .await
                .map_err(WorkflowError::from)
                .map_err(CommandError::from)
                .map_err(|error| error.to_string())?
            {
                pull_requests.push(pull_request);
            }
        }
        Ok::<_, String>(pull_requests)
    }
    .await;
    let discover_attrs = discovered_pull_requests
        .as_ref()
        .map(|pull_requests| vec![perf_attr("discovered_pr_count", pull_requests.len())])
        .unwrap_or_default();
    span.finish_step(
        discover_step,
        discover_attrs,
        discovered_pull_requests.as_ref().err(),
    );
    let numbers = discovered_pull_requests
        .as_ref()
        .map(|pull_requests| stack_status_numbers_from_metadata(&metadata, pull_requests))
        .unwrap_or_else(|_| stack_status_numbers_from_metadata(&metadata, &[]));
    span.set([
        perf_attr("repo", repository_identity.slug()),
        perf_attr("metadata_node_count", metadata.nodes.len()),
        perf_attr("pr_count", numbers.len()),
    ]);

    let statuses_result = if numbers.is_empty() {
        Ok(Vec::new())
    } else {
        let fetch_step = span.start_step(
            "fetch_github_status",
            [perf_attr("pr_count", numbers.len())],
        );
        let statuses_result = github
            .pull_request_statuses(&context.origin.github, &numbers)
            .await
            .map_err(WorkflowError::from)
            .map_err(CommandError::from)
            .map_err(|error| error.to_string());
        let status_attrs = statuses_result
            .as_ref()
            .map(|statuses| vec![perf_attr("status_count", statuses.len())])
            .unwrap_or_default();
        span.finish_step(fetch_step, status_attrs, statuses_result.as_ref().err());
        statuses_result
    };
    let result = match discovered_pull_requests.and(statuses_result) {
        Ok(statuses) => {
            let maintain_step = span.start_step(
                "maintain_stack_metadata",
                [perf_attr("status_count", statuses.len())],
            );
            let maintained = maintain_stack_status_metadata(&repository.root, &metadata, &statuses);
            span.finish_step(
                maintain_step,
                maintained
                    .as_ref()
                    .map(|metadata| {
                        vec![perf_attr(
                            "metadata_node_count",
                            metadata.as_ref().map_or(0, |metadata| metadata.nodes.len()),
                        )]
                    })
                    .unwrap_or_default(),
                maintained.as_ref().err(),
            );
            match maintained {
                Ok(Some(maintained)) => {
                    let trunk_step = span.start_step("fetch_trunk_status", Vec::new());
                    let trunk_result = domain::status_report(&context, status_workspace, &github)
                        .await
                        .map(|report| domain::origin_status_report(&context, report))
                        .map_err(CommandError::from)
                        .map_err(|error| error.to_string());
                    span.finish_step(trunk_step, Vec::new(), trunk_result.as_ref().err());
                    match trunk_result {
                        Ok(trunk) => {
                            let snapshot = PullRequestStackSnapshot::from_metadata(
                                &maintained,
                                &[],
                                &[],
                                PullRequestStackSelection::default(),
                            );
                            Ok(Some(domain::pull_request_stack_status_report(
                                &context, snapshot, statuses, trunk,
                            )))
                        }
                        Err(error) => Err(error),
                    }
                }
                Ok(None) => return None,
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    let result = match result {
        Ok(Some(report)) => Ok(report),
        Ok(None) => return None,
        Err(error) => Err(error),
    };

    Some(GlobalStackStatusEntry {
        key: Some(repository.key.clone()),
        root: repository.root.clone(),
        display_root,
        repository: Some(repository_identity),
        result,
    })
}

async fn prepare_global_stack_status(
    root: PathBuf,
    environment: RuntimeEnvironment,
) -> Result<Option<(RepositoryContext, StackMetadata, StatusWorkspaceFacts)>, String> {
    tokio::task::spawn_blocking(move || {
        let metadata = read_stack_metadata(&root).map_err(|error| error.to_string())?;
        if metadata.nodes.is_empty() {
            return Ok(None);
        }
        let environment = environment.with_current_dir(&root);
        let context =
            RepositoryContext::discover(&environment).map_err(|error| error.to_string())?;
        let workspace = JjWorkspace::load(context.workspace_root.clone())
            .map_err(CommandError::from)
            .and_then(|workspace| {
                workspace
                    .status_facts(
                        context
                            .github_remotes
                            .iter()
                            .map(|remote| remote.name.as_str()),
                    )
                    .map_err(CommandError::from)
            })
            .map_err(|error| error.to_string())?;
        Ok(Some((context, metadata, workspace)))
    })
    .await
    .map_err(|error| format!("stack status worker failed: {error}"))?
}

fn maintained_stack_status_report(
    context: &RepositoryContext,
    root: &Path,
    metadata: &StackMetadata,
    statuses: Vec<PullRequestStatusRecord>,
    fetch_trunk: impl FnOnce() -> Result<Option<RemoteStatusReport>, String>,
) -> Result<Option<PullRequestStackStatusReport>, String> {
    let Some(maintained) = maintain_stack_status_metadata(root, metadata, &statuses)? else {
        return Ok(None);
    };

    let trunk = fetch_trunk()?;
    let snapshot = PullRequestStackSnapshot::from_metadata(
        &maintained,
        &[],
        &[],
        PullRequestStackSelection::default(),
    );
    Ok(Some(domain::pull_request_stack_status_report(
        context, snapshot, statuses, trunk,
    )))
}

fn maintain_stack_status_metadata(
    root: &Path,
    metadata: &StackMetadata,
    statuses: &[PullRequestStatusRecord],
) -> Result<Option<StackMetadata>, String> {
    let maintained = domain::maintain_stack_metadata_pull_request_statuses(statuses, metadata);
    if &maintained != metadata {
        write_stack_metadata(root, &maintained).map_err(|error| error.to_string())?;
    }
    if maintained.nodes.is_empty() {
        return Ok(None);
    }

    Ok(Some(maintained))
}

fn stack_status_numbers_from_metadata(
    metadata: &StackMetadata,
    discovered_pull_requests: &[PullRequestRecord],
) -> Vec<u64> {
    let mut seen = BTreeSet::new();
    metadata
        .nodes
        .iter()
        .filter_map(|node| node.pull_request)
        .chain(
            discovered_pull_requests
                .iter()
                .map(|pull_request| pull_request.number),
        )
        .filter(|number| seen.insert(*number))
        .collect()
}

fn stack_status_missing_pull_requests_from_metadata(
    metadata: &StackMetadata,
    mut resolve_pull_request: impl FnMut(&str) -> Result<Option<PullRequestRecord>, String>,
) -> Result<Vec<PullRequestRecord>, String> {
    let mut pull_requests = Vec::new();
    for branch in stack_status_missing_pull_request_branches_from_metadata(metadata) {
        if let Some(pull_request) = resolve_pull_request(&branch)? {
            pull_requests.push(pull_request);
        }
    }
    Ok(pull_requests)
}

fn stack_status_missing_pull_request_branches_from_metadata(
    metadata: &StackMetadata,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    metadata
        .nodes
        .iter()
        .filter(|node| node.pull_request.is_none())
        .map(|node| node.branch.as_str())
        .filter(|branch| seen.insert((*branch).to_owned()))
        .map(str::to_owned)
        .collect()
}

async fn production_global_remote_status_entry(
    repository: &WorkRepository,
    environment: &RuntimeEnvironment,
    token_environment: &RuntimeEnvironment,
    changed: bool,
) -> Option<GlobalStatusEntry> {
    let display_root = display_path(&repository.root, environment);
    let result = prepare_global_remote_status(repository.root.clone(), environment.clone()).await;
    let (repository_identity, result) = match result {
        Ok((context, workspace)) => {
            let repository_identity = context.origin.github.clone();
            let result = match OctocrabGitHubClient::from_token_source(
                &context.token_source,
                token_environment,
            )
            .map_err(WorkflowError::from)
            .map_err(CommandError::from)
            .map_err(|error| error.to_string())
            {
                Ok(github) => domain::remote_status_report(&context, workspace, &github)
                    .await
                    .map_err(CommandError::from)
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error),
            };
            (Some(repository_identity), result)
        }
        Err(error) => (None, Err(error)),
    };
    if changed
        && result
            .as_ref()
            .is_ok_and(|report| !status_report_has_changes(report))
    {
        return None;
    }

    Some(GlobalStatusEntry {
        key: Some(repository.key.clone()),
        root: repository.root.clone(),
        display_root,
        repository: repository_identity,
        result,
    })
}

async fn prepare_global_remote_status(
    root: PathBuf,
    environment: RuntimeEnvironment,
) -> Result<(RepositoryContext, StatusWorkspaceFacts), String> {
    tokio::task::spawn_blocking(move || {
        let result: Result<(RepositoryContext, StatusWorkspaceFacts), CommandError> = (|| {
            let environment = environment.with_current_dir(&root);
            let context = RepositoryContext::discover(&environment)?;
            let workspace = JjWorkspace::load(context.workspace_root.clone())?.status_facts(
                context
                    .github_remotes
                    .iter()
                    .map(|remote| remote.name.as_str()),
            )?;

            Ok((context, workspace))
        })();

        result.map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("status worker failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingGitHubCalls {
        authenticated_user: AtomicUsize,
        repository_access: AtomicUsize,
        find_open_pull_request: AtomicUsize,
        find_pull_request_by_number: AtomicUsize,
        pull_request_statuses: AtomicUsize,
        update_pull_request: AtomicUsize,
    }

    #[derive(Clone, Default)]
    struct CountingGitHub {
        calls: Arc<CountingGitHubCalls>,
    }

    #[async_trait::async_trait]
    impl GitHubClient for CountingGitHub {
        async fn authenticated_user(&self) -> Result<AuthenticatedUser, GitHubError> {
            self.calls
                .authenticated_user
                .fetch_add(1, Ordering::Relaxed);
            Ok(AuthenticatedUser {
                login: "example-user".to_owned(),
            })
        }

        async fn repository_access(
            &self,
            repository: &GitHubRepository,
        ) -> Result<RepositoryAccess, GitHubError> {
            self.calls.repository_access.fetch_add(1, Ordering::Relaxed);
            Ok(RepositoryAccess {
                repository: repository.clone(),
                default_branch: Some("main".to_owned()),
                can_read: true,
                can_push: true,
                can_admin: false,
            })
        }

        async fn repository_fork(
            &self,
            _repository: &GitHubRepository,
        ) -> Result<Option<RepositoryFork>, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn create_repository(
            &self,
            _repository: &GitHubRepository,
            _private: bool,
        ) -> Result<RepositoryCreation, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn compare_commits(
            &self,
            _repository: &GitHubRepository,
            _base: &str,
            _head: &str,
        ) -> Result<CommitComparison, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn find_authored_open_pull_request_for_head(
            &self,
            _repository: &GitHubRepository,
            _head: &PullRequestHead,
            _author: &str,
        ) -> Result<Option<PullRequestRecord>, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn find_open_pull_request(
            &self,
            repository: &GitHubRepository,
            head: &PullRequestHead,
        ) -> Result<Option<PullRequestRecord>, GitHubError> {
            self.calls
                .find_open_pull_request
                .fetch_add(1, Ordering::Relaxed);
            Ok(Some(test_pull_request(repository, 7, &head.branch, "main")))
        }

        async fn find_pull_request_for_head(
            &self,
            _repository: &GitHubRepository,
            _head: &PullRequestHead,
        ) -> Result<Option<PullRequestRecord>, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn find_pull_request_by_number(
            &self,
            repository: &GitHubRepository,
            number: u64,
        ) -> Result<Option<PullRequestRecord>, GitHubError> {
            self.calls
                .find_pull_request_by_number
                .fetch_add(1, Ordering::Relaxed);
            Ok(Some(test_pull_request(
                repository,
                number,
                "topic/root",
                "main",
            )))
        }

        async fn pull_request_statuses(
            &self,
            _repository: &GitHubRepository,
            _numbers: &[u64],
        ) -> Result<Vec<PullRequestStatusRecord>, GitHubError> {
            self.calls
                .pull_request_statuses
                .fetch_add(1, Ordering::Relaxed);
            Ok(Vec::new())
        }

        async fn create_pull_request(
            &self,
            _repository: &GitHubRepository,
            _request: PullRequestCreate,
        ) -> Result<PullRequestRecord, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn update_pull_request(
            &self,
            repository: &GitHubRepository,
            number: u64,
            request: PullRequestUpdate,
        ) -> Result<PullRequestRecord, GitHubError> {
            self.calls
                .update_pull_request
                .fetch_add(1, Ordering::Relaxed);
            let mut pull_request = test_pull_request(repository, number, "topic/root", "main");
            if let Some(title) = request.title {
                pull_request.title = title;
            }
            if let Some(body) = request.body {
                pull_request.body = Some(body);
            }
            if let Some(base) = request.base {
                pull_request.base_branch = base;
            }
            Ok(pull_request)
        }

        async fn mark_pull_request_ready(
            &self,
            _repository: &GitHubRepository,
            _number: u64,
        ) -> Result<PullRequestRecord, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn convert_pull_request_to_draft(
            &self,
            _repository: &GitHubRepository,
            _number: u64,
        ) -> Result<PullRequestRecord, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn pull_request_labels(
            &self,
            _repository: &GitHubRepository,
            _number: u64,
        ) -> Result<Vec<String>, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn add_labels(
            &self,
            _repository: &GitHubRepository,
            _number: u64,
            _labels: Vec<String>,
        ) -> Result<LabelApplyResult, GitHubError> {
            unimplemented!("unused in this test")
        }

        async fn sync_reviewers(
            &self,
            _repository: &GitHubRepository,
            _number: u64,
            _desired: ReviewerSelection,
        ) -> Result<ReviewerSyncResult, GitHubError> {
            unimplemented!("unused in this test")
        }
    }

    #[test]
    fn traced_github_client_caches_facts_and_pull_request_lookups() {
        let inner = CountingGitHub::default();
        let calls = Arc::clone(&inner.calls);
        let client = TracedGitHubClient {
            inner,
            perf: PerfLog::disabled(),
            repo: "example-owner/example-repo".to_owned(),
            cache: Arc::new(Mutex::new(GitHubFactCache::default())),
        };
        let repository = GitHubRepository {
            owner: "example-owner".to_owned(),
            name: "example-repo".to_owned(),
        };
        let head = PullRequestHead::same_repository("example-owner", "topic/root");

        pollster::block_on(client.authenticated_user()).expect("first user loads");
        pollster::block_on(client.authenticated_user()).expect("second user is cached");
        pollster::block_on(client.repository_access(&repository)).expect("first access loads");
        pollster::block_on(client.repository_access(&repository)).expect("second access is cached");
        pollster::block_on(client.find_open_pull_request(&repository, &head))
            .expect("first head lookup loads");
        pollster::block_on(client.find_open_pull_request(&repository, &head))
            .expect("second head lookup is cached");
        pollster::block_on(client.find_pull_request_by_number(&repository, 7))
            .expect("number lookup reuses head result");
        pollster::block_on(client.pull_request_statuses(&repository, &[7]))
            .expect("status lookup is traced");
        pollster::block_on(client.update_pull_request(
            &repository,
            7,
            PullRequestUpdate {
                title: Some("Updated title".to_owned()),
                body: None,
                base: None,
            },
        ))
        .expect("mutation refreshes cache");
        let updated = pollster::block_on(client.find_open_pull_request(&repository, &head))
            .expect("post-update lookup is cached")
            .expect("cached PR exists");

        assert_eq!(updated.title, "Updated title");
        assert_eq!(calls.authenticated_user.load(Ordering::Relaxed), 1);
        assert_eq!(calls.repository_access.load(Ordering::Relaxed), 1);
        assert_eq!(calls.find_open_pull_request.load(Ordering::Relaxed), 1);
        assert_eq!(calls.find_pull_request_by_number.load(Ordering::Relaxed), 0);
        assert_eq!(calls.pull_request_statuses.load(Ordering::Relaxed), 1);
        assert_eq!(calls.update_pull_request.load(Ordering::Relaxed), 1);
    }

    fn test_pull_request(
        repository: &GitHubRepository,
        number: u64,
        head_branch: &str,
        base_branch: &str,
    ) -> PullRequestRecord {
        PullRequestRecord {
            number,
            title: "Example PR".to_owned(),
            body: None,
            head_branch: head_branch.to_owned(),
            base_branch: base_branch.to_owned(),
            html_url: Some(format!(
                "https://github.com/{}/{}/pull/{number}",
                repository.owner, repository.name
            )),
            draft: false,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }
    }
}
