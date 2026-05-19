use super::*;
use futures::{stream, StreamExt};

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

    /// Finds the most recent GitHub pull request for a same-repository bookmark head.
    fn find_pull_request_for_head(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError>;

    /// Opens a URL in the platform default browser.
    fn open_url(&self, url: &str) -> io::Result<()>;

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

    /// Ensures the selected PR bookmark points at the selected jj commit.
    fn ensure_bookmark(
        &self,
        context: &RepositoryContext,
        branch: &str,
        target_commit_id: &str,
    ) -> Result<BookmarkUpdate, JjError>;

    /// Pushes the selected bookmark through the jj Git transport boundary.
    fn push_bookmark(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<PushOutcome, JjError>;

    /// Optionally prepares sync by advancing the local trunk bookmark to current work.
    fn advance_trunk_for_sync(
        &self,
        context: &RepositoryContext,
    ) -> Result<AdvanceTrunkOutcome, JjError>;

    /// Pushes all tracked fixed-origin bookmarks, including deleted bookmarks.
    fn push_tracked(&self, context: &RepositoryContext) -> Result<TrackedPushOutcome, JjError>;

    /// Syncs PR descriptions for tracked bookmark updates and returns PR metadata rendered by sync.
    fn sync_pull_requests(
        &self,
        context: &RepositoryContext,
        push: &TrackedPushOutcome,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError>;

    /// Builds PR metadata and bookmark intent before mutation.
    fn pull_request_plan(
        &self,
        context: &RepositoryContext,
        workspace: WorkspaceFacts,
        task_id: Option<String>,
        labels: Vec<String>,
        draft: bool,
    ) -> Result<PullRequestPlan, WorkflowError>;

    /// Creates or updates the GitHub PR after local bookmark state has been pushed.
    fn publish_pull_request(
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
        })
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
            let github =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)?;
            let head = PullRequestHead::same_repository(&context.origin.github.owner, branch);

            Ok(github
                .find_authored_open_pull_request_for_head(&context.origin.github, &head, author)
                .await?)
        })
    }

    fn find_pull_request_for_head(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let github =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)?;
            let head = PullRequestHead::same_repository(&context.origin.github.owner, branch);

            Ok(github
                .find_pull_request_for_head(&context.origin.github, &head)
                .await?)
        })
    }

    fn open_url(&self, url: &str) -> io::Result<()> {
        open_url_in_browser(url)
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

    fn ensure_bookmark(
        &self,
        context: &RepositoryContext,
        branch: &str,
        target_commit_id: &str,
    ) -> Result<BookmarkUpdate, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.ensure_bookmark(branch, target_commit_id)
    }

    fn push_bookmark(
        &self,
        context: &RepositoryContext,
        branch: &str,
    ) -> Result<PushOutcome, JjError> {
        JjWorkspace::load(context.workspace_root.clone())?.push_bookmark(branch)
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

    fn sync_pull_requests(
        &self,
        context: &RepositoryContext,
        push: &TrackedPushOutcome,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError> {
        self.github_runtime.block_on(async {
            let Ok(github) =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)
            else {
                return Ok(Vec::new());
            };

            // Keep fetch/push usable when GitHub is unavailable, but surface failures once PR
            // description updates start so stale GitHub text is not silently accepted.
            domain::sync_pull_requests(context, push, &github).await
        })
    }

    fn pull_request_plan(
        &self,
        context: &RepositoryContext,
        workspace: WorkspaceFacts,
        task_id: Option<String>,
        labels: Vec<String>,
        draft: bool,
    ) -> Result<PullRequestPlan, WorkflowError> {
        self.github_runtime.block_on(async {
            let github =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)?;

            domain::pull_request_plan(context, workspace, &github, task_id, labels, draft).await
        })
    }

    fn publish_pull_request(
        &self,
        context: &RepositoryContext,
        plan: PullRequestPlan,
        bookmark_update: BookmarkUpdate,
        push: PushOutcome,
    ) -> Result<PullRequestReport, WorkflowError> {
        self.github_runtime.block_on(async {
            let github =
                OctocrabGitHubClient::from_token_source(&context.token_source, self.environment)?;

            domain::publish_pull_request(context, plan, bookmark_update, push, &github).await
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
