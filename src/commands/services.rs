use super::*;
use crate::github::{
    AuthenticatedUser, CommitComparison, ComparisonStatus, GitHubError, LabelApplyResult,
    PullRequestCreate, PullRequestStatusRecord, PullRequestUpdate, RepositoryAccess,
    RepositoryFork, ReviewerSyncResult,
};
use chrono::Utc;
use futures::{stream, StreamExt};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(super) struct StackStatusFetches {
    pub(super) statuses: Vec<PullRequestStatusRecord>,
    pub(super) trunk: Option<RemoteStatusReport>,
    pub(super) fetch_github_status_us: u64,
    pub(super) fetch_trunk_status_us: Option<u64>,
}

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

    /// Checks origin trunk freshness cheaply for stack status without exact GitHub commit counts.
    fn stack_trunk_status_report(
        &self,
        context: &RepositoryContext,
        workspace: StatusWorkspaceFacts,
    ) -> Result<RemoteStatusReport, WorkflowError>;

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

    /// Finds open pull requests authored by a user in the fixed-origin repository.
    fn authored_open_pull_requests(
        &self,
        context: &RepositoryContext,
        author: &str,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError>;

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

    /// Loads stack-status GitHub facts, allowing production to overlap independent requests.
    fn stack_status_fetches(
        &self,
        context: &RepositoryContext,
        numbers: &[u64],
        fetch_trunk: bool,
    ) -> Result<StackStatusFetches, CommandError> {
        let fetch_github_status_started = Instant::now();
        let statuses = if numbers.is_empty() {
            Vec::new()
        } else {
            self.pull_request_statuses(context, numbers)?
        };
        let fetch_github_status_us = duration_us(fetch_github_status_started.elapsed());

        let (trunk, fetch_trunk_status_us) = if fetch_trunk {
            let fetch_trunk_status_started = Instant::now();
            let workspace = self.status_workspace_facts(context)?;
            let trunk = self.stack_trunk_status_report(context, workspace)?;
            (
                Some(trunk),
                Some(duration_us(fetch_trunk_status_started.elapsed())),
            )
        } else {
            (None, None)
        };

        Ok(StackStatusFetches {
            statuses,
            trunk,
            fetch_github_status_us,
            fetch_trunk_status_us,
        })
    }

    /// Searches open pull requests requesting review from or already reviewed by the authenticated viewer.
    fn review_requests(
        &self,
        token_source: &TokenSource,
    ) -> Result<PullRequestReviewRequests, WorkflowError>;

    /// Returns cached public display names for GitHub logins, refreshing stale entries best-effort.
    fn github_user_display_names(
        &self,
        _token_source: &TokenSource,
        _logins: &[String],
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// Loads batched read-only GitHub status facts for an arbitrary repository.
    fn pull_request_statuses_for_repository(
        &self,
        token_source: &TokenSource,
        repository: &GitHubRepository,
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
                    self.find_pull_request_for_head(context, branch)
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

fn record_fetch_trace_step(span: &mut PerfSpan, step: FetchTraceStep) {
    let error = step.error.clone();
    span.record_step_us(
        step.name,
        step.duration_us,
        step.attrs.into_iter().map(fetch_trace_attr_to_perf),
        error.as_ref(),
    );
}

fn fetch_trace_attr_to_perf(attr: FetchTraceAttr) -> PerfAttr {
    perf_attr(attr.key, fetch_trace_value_to_perf(attr.value))
}

fn fetch_trace_value_to_perf(value: FetchTraceValue) -> PerfValue {
    match value {
        FetchTraceValue::String(value) => PerfValue::String(value),
        FetchTraceValue::U64(value) => PerfValue::U64(value),
        FetchTraceValue::I64(value) => PerfValue::I64(value),
        FetchTraceValue::Bool(value) => PerfValue::Bool(value),
    }
}

fn fetch_outcome_attrs(fetch: &FetchOutcome) -> Vec<PerfAttr> {
    vec![
        perf_attr("branch", &fetch.branch),
        perf_attr("changed_remote_bookmarks", fetch.changed_remote_bookmarks),
        perf_attr("changed_remote_tags", fetch.changed_remote_tags),
        perf_attr("abandoned_commits", fetch.abandoned_commits),
        perf_attr("rebased_trunk_children", fetch.rebased_trunk_children),
        perf_attr("rebased_descendants", fetch.rebased_descendants),
        perf_attr("skipped_trunk_children", fetch.skipped_trunk_children),
        perf_attr("current_repaired", fetch.current_repaired),
        perf_attr("rebased_commit_count", fetch.rebased_commits.len()),
    ]
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

    async fn pull_request_statuses_chunk_traced(
        &self,
        span: &mut PerfSpan,
        repository: &GitHubRepository,
        chunk: &[u64],
        chunk_index: usize,
        chunk_count: usize,
        retry_count: &mut usize,
    ) -> Result<Vec<PullRequestStatusRecord>, GitHubError>
    where
        C: GitHubClient,
    {
        for attempt in 1..=PULL_REQUEST_STATUS_MAX_ATTEMPTS {
            let started = Instant::now();
            let result = self.inner.pull_request_statuses(repository, chunk).await;
            let duration_us = duration_us(started.elapsed());
            let base_attrs = || {
                pull_request_status_chunk_attrs(
                    chunk,
                    chunk_index,
                    chunk_count,
                    attempt,
                    PULL_REQUEST_STATUS_MAX_ATTEMPTS,
                )
            };
            match result {
                Ok(statuses) => {
                    let mut attrs = base_attrs();
                    attrs.push(perf_attr("status_count", statuses.len()));
                    attrs.push(perf_attr("retry", attempt > 1));
                    span.record_step_us(
                        "pull_request_statuses.chunk",
                        duration_us,
                        attrs,
                        Option::<&GitHubError>::None,
                    );
                    return Ok(statuses);
                }
                Err(error) => {
                    let transient_error_kind = transient_github_error_kind(&error);
                    let will_retry = transient_error_kind.is_some()
                        && attempt < PULL_REQUEST_STATUS_MAX_ATTEMPTS;
                    let mut attrs = base_attrs();
                    attrs.push(perf_attr("transient_error", transient_error_kind.is_some()));
                    if let Some(kind) = transient_error_kind {
                        attrs.push(perf_attr("transient_error_kind", kind));
                    }
                    attrs.push(perf_attr("will_retry", will_retry));
                    span.record_step_us(
                        "pull_request_statuses.chunk",
                        duration_us,
                        attrs,
                        Some(&error),
                    );
                    if will_retry {
                        *retry_count += 1;
                        tokio::time::sleep(Duration::from_millis(
                            PULL_REQUEST_STATUS_RETRY_DELAY_MS,
                        ))
                        .await;
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        unreachable!("pull request status retry loop always returns")
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

const PULL_REQUEST_STATUS_TRACE_CHUNK_SIZE: usize = 10;
const PULL_REQUEST_STATUS_MAX_ATTEMPTS: usize = 2;
const PULL_REQUEST_STATUS_RETRY_DELAY_MS: u64 = 250;

fn pull_request_status_chunk_attrs(
    numbers: &[u64],
    chunk_index: usize,
    chunk_count: usize,
    attempt: usize,
    max_attempts: usize,
) -> Vec<PerfAttr> {
    let mut attrs = vec![
        perf_attr("chunk_index", chunk_index),
        perf_attr("chunk_count", chunk_count),
        perf_attr("attempt", attempt),
        perf_attr("max_attempts", max_attempts),
        perf_attr("number_count", numbers.len()),
    ];
    if let Some(first) = numbers.first() {
        attrs.push(perf_attr("first_number", *first));
    }
    if let Some(last) = numbers.last() {
        attrs.push(perf_attr("last_number", *last));
    }
    attrs
}

fn transient_github_error_kind(error: &GitHubError) -> Option<&'static str> {
    match error {
        GitHubError::Api { source, .. } => transient_octocrab_error_kind(source),
        GitHubError::ApiResponse { status, .. } if *status == 429 => Some("rate_limit"),
        GitHubError::ApiResponse { status, .. } if (500..=599).contains(status) => Some("server"),
        _ => None,
    }
}

fn transient_octocrab_error_kind(error: &octocrab::Error) -> Option<&'static str> {
    match error {
        octocrab::Error::Hyper { .. } => Some("hyper"),
        octocrab::Error::Http { .. } => Some("http"),
        octocrab::Error::Service { .. } => Some("service"),
        octocrab::Error::Graphql { .. } => Some("graphql"),
        octocrab::Error::GitHub { source, .. } if source.status_code.as_u16() == 429 => {
            Some("rate_limit")
        }
        octocrab::Error::GitHub { source, .. } if source.status_code.is_server_error() => {
            Some("server")
        }
        _ => None,
    }
}

fn unique_display_name_logins(logins: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    logins
        .iter()
        .map(|login| login.trim())
        .filter(|login| !login.is_empty())
        .filter(|login| seen.insert((*login).to_owned()))
        .map(str::to_owned)
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

    async fn branch_head_sha(
        &self,
        repository: &GitHubRepository,
        branch: &str,
    ) -> Result<String, GitHubError> {
        let span = self.start_span(
            "github.branch_head_sha",
            Some(repository),
            [perf_attr("branch", branch)],
        );
        let result = self.inner.branch_head_sha(repository, branch).await;
        let attrs = result
            .as_ref()
            .map(|sha| vec![perf_attr("head_sha", sha)])
            .unwrap_or_default();
        self.finish(span, result, attrs)
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
        let attrs = result
            .as_ref()
            .map(compare_commits_result_attrs)
            .unwrap_or_default();
        self.finish(span, result, attrs)
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

    async fn authored_open_pull_requests(
        &self,
        repository: &GitHubRepository,
        author: &str,
    ) -> Result<Vec<PullRequestRecord>, GitHubError> {
        let span = self.start_span(
            "github.authored_open_pull_requests",
            Some(repository),
            [perf_attr("author", author)],
        );
        let result = self
            .inner
            .authored_open_pull_requests(repository, author)
            .await;
        let attrs = result
            .as_ref()
            .map(|pull_requests| vec![perf_attr("pull_request_count", pull_requests.len())])
            .unwrap_or_default();
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
        let requested_numbers = unique_pull_request_numbers(numbers);
        let chunk_count = pull_request_status_chunk_count(requested_numbers.len());
        let mut span = self.start_span(
            "github.pull_request_statuses",
            Some(repository),
            [
                perf_attr("number_count", numbers.len()),
                perf_attr("unique_number_count", requested_numbers.len()),
                perf_attr("chunk_count", chunk_count),
                perf_attr("chunk_size", PULL_REQUEST_STATUS_TRACE_CHUNK_SIZE),
            ],
        );
        let mut statuses = Vec::new();
        let mut retry_count = 0_usize;
        for (chunk_index, chunk) in requested_numbers
            .chunks(PULL_REQUEST_STATUS_TRACE_CHUNK_SIZE)
            .enumerate()
        {
            let chunk_statuses = self
                .pull_request_statuses_chunk_traced(
                    &mut span,
                    repository,
                    chunk,
                    chunk_index,
                    chunk_count,
                    &mut retry_count,
                )
                .await;
            match chunk_statuses {
                Ok(chunk_statuses) => statuses.extend(chunk_statuses),
                Err(error) => {
                    return self.finish(
                        span,
                        Err(error),
                        [
                            perf_attr("retry_count", retry_count),
                            perf_attr("status_count", statuses.len()),
                            perf_attr("completed_chunk_count", chunk_index),
                        ],
                    );
                }
            }
        }

        self.finish(
            span,
            Ok(statuses.clone()),
            [
                perf_attr("retry_count", retry_count),
                perf_attr("status_count", statuses.len()),
                perf_attr("completed_chunk_count", chunk_count),
            ],
        )
    }

    async fn review_requests(&self) -> Result<PullRequestReviewRequests, GitHubError> {
        let span = self.start_span("github.review_requests", None, Vec::new());
        let result = self.inner.review_requests().await;
        let attrs = result
            .as_ref()
            .map(|inbox| {
                let repository_count = inbox
                    .requests
                    .iter()
                    .map(|request| &request.repository)
                    .collect::<BTreeSet<_>>()
                    .len();
                vec![
                    perf_attr("request_count", inbox.requests.len()),
                    perf_attr("repository_count", repository_count),
                    perf_attr("viewer", &inbox.viewer.login),
                ]
            })
            .unwrap_or_default();
        self.finish(span, result, attrs)
    }

    async fn user_profiles(
        &self,
        logins: &[String],
    ) -> Result<Vec<GitHubUserProfile>, GitHubError> {
        let span = self.start_span(
            "github.user_profiles",
            None,
            [perf_attr("login_count", logins.len())],
        );
        let result = self.inner.user_profiles(logins).await;
        let attrs = result
            .as_ref()
            .map(|profiles| vec![perf_attr("profile_count", profiles.len())])
            .unwrap_or_default();
        self.finish(span, result, attrs)
    }

    async fn pull_request_suggested_reviewers(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<Vec<String>, GitHubError> {
        let span = self.start_span(
            "github.pull_request_suggested_reviewers",
            Some(repository),
            [perf_attr("number", number)],
        );
        let result = self
            .inner
            .pull_request_suggested_reviewers(repository, number)
            .await;
        let attrs = result
            .as_ref()
            .map(|reviewers| vec![perf_attr("reviewer_count", reviewers.len())])
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
        number_count.div_ceil(PULL_REQUEST_STATUS_TRACE_CHUNK_SIZE)
    }
}

fn compare_commits_result_attrs(comparison: &CommitComparison) -> Vec<PerfAttr> {
    vec![
        perf_attr(
            "comparison_status",
            comparison_status_label(comparison.status),
        ),
        perf_attr("ahead_by", comparison.ahead_by),
        perf_attr("behind_by", comparison.behind_by),
        perf_attr(
            "identical",
            comparison.status == ComparisonStatus::Identical,
        ),
    ]
}

fn comparison_status_label(status: ComparisonStatus) -> &'static str {
    match status {
        ComparisonStatus::Ahead => "ahead",
        ComparisonStatus::Behind => "behind",
        ComparisonStatus::Diverged => "diverged",
        ComparisonStatus::Identical => "identical",
        ComparisonStatus::Unknown => "unknown",
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

    fn stack_trunk_status_report(
        &self,
        context: &RepositoryContext,
        workspace: StatusWorkspaceFacts,
    ) -> Result<RemoteStatusReport, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;

            domain::stack_trunk_status_report(context, workspace, &github).await
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

    fn authored_open_pull_requests(
        &self,
        context: &RepositoryContext,
        author: &str,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = self.traced_github_client(context)?;
            Ok(github
                .authored_open_pull_requests(&context.origin.github, author)
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

    fn stack_status_fetches(
        &self,
        context: &RepositoryContext,
        numbers: &[u64],
        fetch_trunk: bool,
    ) -> Result<StackStatusFetches, CommandError> {
        self.github_runtime.block_on(async {
            let github = self
                .traced_github_client(context)
                .map_err(WorkflowError::from)?;
            let fetch_statuses = async {
                let started = Instant::now();
                let statuses = if numbers.is_empty() {
                    Vec::new()
                } else {
                    github
                        .pull_request_statuses(&context.origin.github, numbers)
                        .await
                        .map_err(WorkflowError::from)?
                };
                Ok::<_, CommandError>((statuses, duration_us(started.elapsed())))
            };
            let fetch_trunk_status = async {
                if !fetch_trunk {
                    return Ok::<_, CommandError>((None, None));
                }

                let started = Instant::now();
                let workspace = JjWorkspace::load(context.workspace_root.clone())?.status_facts(
                    context
                        .github_remotes
                        .iter()
                        .map(|remote| remote.name.as_str()),
                )?;
                let trunk = domain::stack_trunk_status_report(context, workspace, &github).await?;
                Ok((Some(trunk), Some(duration_us(started.elapsed()))))
            };

            let ((statuses, fetch_github_status_us), (trunk, fetch_trunk_status_us)) =
                tokio::try_join!(fetch_statuses, fetch_trunk_status)?;
            Ok(StackStatusFetches {
                statuses,
                trunk,
                fetch_github_status_us,
                fetch_trunk_status_us,
            })
        })
    }

    fn review_requests(
        &self,
        token_source: &TokenSource,
    ) -> Result<PullRequestReviewRequests, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = OctocrabGitHubClient::from_token_source(token_source, self.environment)?;
            let github = TracedGitHubClient {
                inner: github,
                perf: PerfLog::from_environment(self.environment),
                repo: "review".to_owned(),
                cache: Arc::new(Mutex::new(GitHubFactCache::default())),
            };
            let inbox = github.review_requests().await?;
            if inbox.viewer.login.is_empty() {
                return Err(WorkflowError::MissingGitHubLogin);
            }
            Ok(inbox)
        })
    }

    fn github_user_display_names(
        &self,
        token_source: &TokenSource,
        logins: &[String],
    ) -> BTreeMap<String, String> {
        let logins = unique_display_name_logins(logins);
        if logins.is_empty() {
            return BTreeMap::new();
        }

        let now = Utc::now();
        let mut cache = read_github_user_name_cache(self.environment).unwrap_or_default();
        let mut display_names = BTreeMap::new();
        let mut stale_logins = Vec::new();
        for login in logins {
            match cache.fresh_name(&login, now) {
                Some(Some(name)) => {
                    display_names.insert(login, name);
                }
                Some(None) => {}
                None => stale_logins.push(login),
            }
        }

        if stale_logins.is_empty() {
            return display_names;
        }

        let profiles = self.github_runtime.block_on(async {
            let github = OctocrabGitHubClient::from_token_source(token_source, self.environment)?;
            let github = TracedGitHubClient {
                inner: github,
                perf: PerfLog::from_environment(self.environment),
                repo: "users".to_owned(),
                cache: Arc::clone(&self.github_cache),
            };
            github.user_profiles(&stale_logins).await
        });

        match profiles {
            Ok(profiles) => {
                for profile in profiles {
                    cache.upsert(&profile.login, profile.name.as_deref(), now);
                    if let Some(name) = profile.name {
                        display_names.insert(profile.login, name);
                    }
                }
                let _ = write_github_user_name_cache(self.environment, &cache);
            }
            Err(_) => {
                for login in stale_logins {
                    if let Some(Some(name)) = cache.cached_name(&login) {
                        display_names.insert(login, name);
                    }
                }
            }
        }

        display_names
    }

    fn pull_request_statuses_for_repository(
        &self,
        token_source: &TokenSource,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let github = OctocrabGitHubClient::from_token_source(token_source, self.environment)?;
            let github = TracedGitHubClient {
                inner: github,
                perf: PerfLog::from_environment(self.environment),
                repo: repository.slug(),
                cache: Arc::new(Mutex::new(GitHubFactCache::default())),
            };
            Ok(github.pull_request_statuses(repository, numbers).await?)
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
        let mut span = PerfLog::from_environment(self.environment).start(
            "jj.fetch_origin",
            [
                perf_attr("repo", context.origin.github.slug()),
                perf_attr(
                    "workspace_root",
                    context.workspace_root.display().to_string(),
                ),
                perf_attr("pid", u64::from(std::process::id())),
            ],
        );
        let result = (|| {
            let mut workspace = JjWorkspace::load(context.workspace_root.clone())?;
            span.set([perf_attr("jj_workspace", workspace.workspace_name())]);
            let mut trace = |step| record_fetch_trace_step(&mut span, step);
            workspace.fetch_origin_with_trace(&mut trace)
        })();
        if let Ok(fetch) = &result {
            span.set(fetch_outcome_attrs(fetch));
        } else if let Err(error) = &result {
            span.record_error(error);
        }
        span.end();
        result
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
        maintained_stack_status_report(
            &context,
            &repository_environment,
            &metadata,
            statuses,
            || fetch_trunk(&context),
        )
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
    let prepared_result =
        prepare_global_stack_status(repository.root.clone(), environment.clone()).await;
    if let Ok(prepared) = &prepared_result {
        record_global_stack_status_preparation_steps(span, &prepared.metrics);
    }
    let (context, metadata) = match prepared_result {
        Ok(prepared) => match prepared.data {
            Some(data) => {
                let data = *data;
                (data.context, data.metadata)
            }
            None => return None,
        },
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
    let status_facts_task = spawn_global_stack_status_facts_load(
        context.workspace_root.clone(),
        context
            .github_remotes
            .iter()
            .map(|remote| remote.name.clone())
            .collect(),
    );
    let discover_step = span.start_step("discover_missing_pull_requests", Vec::new());
    let discovered_pull_requests = async {
        let mut pull_requests = Vec::new();
        for branch in stack_status_missing_pull_request_branches_from_metadata(&metadata) {
            let head = PullRequestHead::same_repository(&context.origin.github.owner, &branch);
            if let Some(pull_request) = github
                .find_pull_request_for_head(&context.origin.github, &head)
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

    let fetch_statuses = async {
        let started = Instant::now();
        let result = if numbers.is_empty() {
            Ok(Vec::new())
        } else {
            github
                .pull_request_statuses(&context.origin.github, &numbers)
                .await
                .map_err(WorkflowError::from)
                .map_err(CommandError::from)
                .map_err(|error| error.to_string())
        };
        (result, duration_us(started.elapsed()))
    };
    let fetch_trunk = async {
        let started = Instant::now();
        let status_facts_result = await_global_stack_status_facts_load(status_facts_task).await;
        let status_facts_metrics = status_facts_result
            .as_ref()
            .ok()
            .map(|facts| facts.metrics.clone());
        let result = match status_facts_result {
            Ok(status_facts) => {
                domain::stack_trunk_status_report(&context, status_facts.status_workspace, &github)
                    .await
                    .map(Some)
                    .map_err(CommandError::from)
                    .map_err(|error| error.to_string())
            }
            Err(error) => Err(error),
        };
        (result, duration_us(started.elapsed()), status_facts_metrics)
    };
    let (
        (statuses_result, fetch_github_status_us),
        (trunk_result, fetch_trunk_status_us, status_facts_metrics),
    ) = tokio::join!(fetch_statuses, fetch_trunk);

    if !numbers.is_empty() {
        span.record_step_us(
            "fetch_github_status",
            fetch_github_status_us,
            [
                perf_attr("pr_count", numbers.len()),
                perf_attr(
                    "status_count",
                    statuses_result
                        .as_ref()
                        .map_or(0, |statuses| statuses.len()),
                ),
            ],
            statuses_result.as_ref().err(),
        );
    }
    if let Some(metrics) = &status_facts_metrics {
        record_global_stack_status_facts_steps(span, metrics);
    }
    span.record_step_us(
        "fetch_trunk_status",
        fetch_trunk_status_us,
        stack_trunk_status_attrs(trunk_result.as_ref().ok().and_then(Option::as_ref)),
        trunk_result.as_ref().err(),
    );

    let result = match discovered_pull_requests.and(statuses_result) {
        Ok(statuses) => {
            let maintain_step = span.start_step(
                "maintain_stack_metadata",
                [perf_attr("status_count", statuses.len())],
            );
            let maintained =
                maintain_stack_status_metadata(&context, token_environment, &metadata, &statuses);
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
                Ok(Some(maintained)) => match trunk_result {
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
                },
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

struct GlobalStackStatusPreparation {
    data: Option<Box<GlobalStackStatusPreparationData>>,
    metrics: GlobalStackStatusPreparationMetrics,
}

struct GlobalStackStatusPreparationData {
    context: RepositoryContext,
    metadata: StackMetadata,
}

struct GlobalStackStatusFactsLoad {
    status_workspace: StatusWorkspaceFacts,
    metrics: GlobalStackStatusFactsMetrics,
}

#[derive(Debug, Clone, Default)]
struct GlobalStackStatusPreparationMetrics {
    metadata_node_count: usize,
    read_stack_metadata_us: u64,
    discover_repository_context_us: u64,
}

#[derive(Debug, Clone, Default)]
struct GlobalStackStatusFactsMetrics {
    load_jj_workspace_us: u64,
    load_status_facts_us: u64,
    status_facts: StatusWorkspaceMetrics,
}

async fn prepare_global_stack_status(
    root: PathBuf,
    environment: RuntimeEnvironment,
) -> Result<GlobalStackStatusPreparation, String> {
    tokio::task::spawn_blocking(move || {
        let mut metrics = GlobalStackStatusPreparationMetrics::default();
        let metadata = measure_global_stack_status_preparation_step(
            &mut metrics.read_stack_metadata_us,
            || read_stack_metadata(&root).map_err(|error| error.to_string()),
        )?;
        metrics.metadata_node_count = metadata.nodes.len();
        if metadata.nodes.is_empty() {
            return Ok(GlobalStackStatusPreparation {
                data: None,
                metrics,
            });
        }

        let environment = environment.with_current_dir(&root);
        let context = measure_global_stack_status_preparation_step(
            &mut metrics.discover_repository_context_us,
            || RepositoryContext::discover(&environment).map_err(|error| error.to_string()),
        )?;
        Ok(GlobalStackStatusPreparation {
            data: Some(Box::new(GlobalStackStatusPreparationData {
                context,
                metadata,
            })),
            metrics,
        })
    })
    .await
    .map_err(|error| format!("stack status worker failed: {error}"))?
}

fn spawn_global_stack_status_facts_load(
    workspace_root: PathBuf,
    remote_names: Vec<String>,
) -> tokio::task::JoinHandle<Result<GlobalStackStatusFactsLoad, String>> {
    tokio::task::spawn_blocking(move || {
        let mut metrics = GlobalStackStatusFactsMetrics::default();
        let workspace = measure_global_stack_status_preparation_step(
            &mut metrics.load_jj_workspace_us,
            || {
                JjWorkspace::load(workspace_root)
                    .map_err(CommandError::from)
                    .map_err(|error| error.to_string())
            },
        )?;
        let status_facts = measure_global_stack_status_preparation_step(
            &mut metrics.load_status_facts_us,
            || {
                workspace
                    .status_facts_with_metrics(remote_names.iter().map(String::as_str))
                    .map_err(CommandError::from)
                    .map_err(|error| error.to_string())
            },
        )?;
        metrics.status_facts = status_facts.metrics;
        Ok(GlobalStackStatusFactsLoad {
            status_workspace: status_facts.facts,
            metrics,
        })
    })
}

async fn await_global_stack_status_facts_load(
    task: tokio::task::JoinHandle<Result<GlobalStackStatusFactsLoad, String>>,
) -> Result<GlobalStackStatusFactsLoad, String> {
    task.await
        .map_err(|error| format!("stack status worker failed: {error}"))?
}

fn measure_global_stack_status_preparation_step<T>(
    duration_slot_us: &mut u64,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let started = Instant::now();
    let result = operation();
    *duration_slot_us = duration_us(started.elapsed());
    result
}

fn record_global_stack_status_preparation_steps(
    span: &mut PerfSpan,
    metrics: &GlobalStackStatusPreparationMetrics,
) {
    record_successful_step(
        span,
        "read_stack_metadata",
        metrics.read_stack_metadata_us,
        [perf_attr(
            "metadata_node_count",
            metrics.metadata_node_count,
        )],
    );
    if metrics.metadata_node_count == 0 {
        return;
    }
    record_successful_step(
        span,
        "discover_repository_context",
        metrics.discover_repository_context_us,
        Vec::new(),
    );
}

fn stack_trunk_status_attrs(trunk: Option<&RemoteStatusReport>) -> Vec<PerfAttr> {
    trunk
        .map(|trunk| {
            vec![
                perf_attr("state", trunk.comparison.label()),
                perf_attr("counts_exact", trunk.comparison.counts_exact),
                perf_attr("local_ahead_by", trunk.local_ahead_by),
                perf_attr("github_ahead_by", trunk.comparison.github_ahead_by),
                perf_attr("github_behind_by", trunk.comparison.github_behind_by),
            ]
        })
        .unwrap_or_default()
}

fn record_global_stack_status_facts_steps(
    span: &mut PerfSpan,
    metrics: &GlobalStackStatusFactsMetrics,
) {
    record_successful_step(
        span,
        "load_jj_workspace",
        metrics.load_jj_workspace_us,
        Vec::new(),
    );
    record_successful_step(
        span,
        "load_status_facts",
        metrics.load_status_facts_us,
        [perf_attr(
            "remote_count",
            metrics.status_facts.remotes.len(),
        )],
    );
    record_successful_step(
        span,
        "status_facts.current_commit",
        metrics.status_facts.current_commit_us,
        Vec::new(),
    );
    for remote in &metrics.status_facts.remotes {
        let attrs = || {
            vec![
                perf_attr("remote", &remote.remote),
                perf_attr("branch", &remote.branch),
                perf_attr("stack_path_len", remote.stack_path_len),
                perf_attr("non_empty_count", remote.non_empty_count),
                perf_attr("fast_path", remote.trunk.fast_path),
                perf_attr(
                    "preferred_branch_check_count",
                    remote.trunk.preferred_branch_check_count,
                ),
                perf_attr("remote_bookmark_count", remote.trunk.remote_bookmark_count),
                perf_attr("normal_bookmark_count", remote.trunk.normal_bookmark_count),
                perf_attr(
                    "conflicted_bookmark_count",
                    remote.trunk.conflicted_bookmark_count,
                ),
                perf_attr("ancestor_check_count", remote.trunk.ancestor_check_count),
                perf_attr("candidate_count", remote.trunk.candidate_count),
            ]
        };
        record_successful_step(
            span,
            "status_facts.resolve_trunk",
            remote.resolve_trunk_us,
            attrs(),
        );
        record_successful_step(
            span,
            "status_facts.resolve_trunk.scan_remote_bookmarks",
            remote.trunk.scan_remote_bookmarks_us,
            attrs(),
        );
        record_successful_step(
            span,
            "status_facts.resolve_trunk.select_candidate",
            remote.trunk.select_candidate_us,
            attrs(),
        );
        record_successful_step(
            span,
            "status_facts.resolve_trunk.load_commit",
            remote.trunk.load_trunk_commit_us,
            attrs(),
        );
        record_successful_step(
            span,
            "status_facts.linear_stack_path",
            remote.linear_stack_path_us,
            attrs(),
        );
        record_successful_step(
            span,
            "status_facts.count_non_empty_commits",
            remote.count_non_empty_commits_us,
            attrs(),
        );
    }
}

fn record_successful_step(
    span: &mut PerfSpan,
    name: &str,
    duration_us: u64,
    attrs: impl IntoIterator<Item = PerfAttr>,
) {
    span.record_step_us(name, duration_us, attrs, Option::<&String>::None);
}

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().try_into().unwrap_or(u64::MAX)
}

fn maintained_stack_status_report(
    context: &RepositoryContext,
    environment: &RuntimeEnvironment,
    metadata: &StackMetadata,
    statuses: Vec<PullRequestStatusRecord>,
    fetch_trunk: impl FnOnce() -> Result<Option<RemoteStatusReport>, String>,
) -> Result<Option<PullRequestStackStatusReport>, String> {
    let Some(maintained) =
        maintain_stack_status_metadata(context, environment, metadata, &statuses)?
    else {
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
    context: &RepositoryContext,
    environment: &RuntimeEnvironment,
    metadata: &StackMetadata,
    statuses: &[PullRequestStatusRecord],
) -> Result<Option<StackMetadata>, String> {
    let maintained = StackStatusMetadataMaintainer::new(context, environment)
        .maintain(metadata, statuses)
        .map_err(|error| error.to_string())?
        .metadata;
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

        async fn branch_head_sha(
            &self,
            _repository: &GitHubRepository,
            _branch: &str,
        ) -> Result<String, GitHubError> {
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
    fn compare_commits_perf_attrs_include_result_state() {
        let attrs = compare_commits_result_attrs(&CommitComparison {
            status: ComparisonStatus::Diverged,
            ahead_by: 3,
            behind_by: 5,
        });

        assert_eq!(
            attrs,
            vec![
                perf_attr("comparison_status", "diverged"),
                perf_attr("ahead_by", 3_i64),
                perf_attr("behind_by", 5_i64),
                perf_attr("identical", false),
            ]
        );
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
