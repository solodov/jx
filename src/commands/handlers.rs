use super::*;

pub(super) fn handle_request(
    request: CommandRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: PromptHandlers<'_>,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let stdout = match request {
        CommandRequest::Log => services.workspace_log()?,
        CommandRequest::Status => {
            let status = services.workspace_status(environment.current_dir(), output.color)?;
            render_workspace_status(&status)
        }
        CommandRequest::Diff(request) => {
            let config = WorkflowConfig::discover(environment)?;
            let options = diff_options(&config, request)?;
            services.current_diff(environment.current_dir(), &options)?
        }
        CommandRequest::Clone(request) => {
            let config = WorkflowConfig::discover_for_clone(environment)?;
            let plan = config.layout.clone_plan(
                &request.repository,
                request.destination.as_deref(),
                environment,
            )?;
            progress.status(&format!("Cloning {}", clone_link(&plan)));
            services.clone_repository(environment.current_dir(), &plan)?;
            progress.finish();
            render_clone(&plan, &display_path(&plan.destination, environment))
        }
        CommandRequest::Work(request) => {
            handle_work(request, environment, services, progress, &prompts)?
        }
        CommandRequest::Shell(request) => handle_shell(request, environment)?,
        CommandRequest::RemoteStatus(request) => {
            handle_remote_status(request, environment, services, progress, output)?
        }
        CommandRequest::Fetch(request) => {
            handle_fetch(request, environment, services, progress, output)?
        }
        CommandRequest::RebaseOnTrunk(request) => {
            let context = RepositoryContext::discover(environment)?;
            progress.status("Rebasing onto trunk…");
            let outcome = services.rebase_on_trunk(&context, &request.sources)?;
            progress.finish();
            let report = domain::rebase_on_trunk_report(&context, outcome);
            render_rebase_on_trunk(&report, environment.current_dir(), output.color)?
        }
        CommandRequest::Push(request) => {
            let context = RepositoryContext::discover(environment)?;
            if request.tracked {
                progress.status("Pushing tracked bookmarks…");
                let outcome = services.push_tracked(&context)?;
                progress.finish();
                let report = domain::tracked_push_report(&context, outcome);
                render_tracked_push(&report, environment.current_dir(), output.color)?
            } else {
                progress.status("Planning push…");
                let workspace =
                    services.push_workspace_facts(&context, request.revision.as_deref())?;
                let plan = domain::push_plan(&context, workspace, request.revision.as_deref())?;
                progress.finish();

                let bookmark_update = if plan.bookmark.action == BookmarkAction::Create {
                    if !prompts.push_confirmer.confirm_push(&plan)? {
                        return Ok(CommandResult {
                            stdout: "cancelled\n".to_owned(),
                        });
                    }

                    progress.status("Creating bookmark…");
                    services.ensure_bookmark(
                        &context,
                        &plan.bookmark.branch,
                        &plan.target_commit_id,
                    )?
                } else {
                    BookmarkUpdate {
                        branch: plan.bookmark.branch.clone(),
                        created: false,
                    }
                };

                progress.status("Pushing bookmark…");
                let push = services.push_bookmark(&context, &plan.bookmark.branch)?;
                progress.finish();
                let report = domain::push_report(&context, plan, bookmark_update, push);
                render_push(&report, environment.current_dir(), output.color)?
            }
        }
        CommandRequest::Sync => handle_sync(environment, services, progress, &prompts, output)?,
        CommandRequest::Workflow {
            command,
            task_id,
            commit,
            labels,
            reviewers,
            draft,
        } => {
            let context = RepositoryContext::discover(environment)?;
            match command {
                WorkflowCommand::Check => {
                    let _ = labels;
                    let workspace = services.workspace_facts(&context, None)?;
                    let report = services.check_readiness(&context, workspace)?;
                    render_check(&report, environment.current_dir(), output.color)?
                }
                WorkflowCommand::PullRequest => {
                    progress.status("Planning pull request…");
                    let status = services
                        .workspace_status(environment.current_dir(), io::stderr().is_terminal())?;
                    let workspace = services.workspace_facts(&context, commit.as_deref())?;
                    let mut plan =
                        services.pull_request_plan(&context, workspace, task_id, labels, draft)?;
                    progress.finish();
                    prompts.pull_request_previewer.show_preview(&plan, &status);
                    plan.reviewers = prompts
                        .reviewer_selector
                        .select_reviewers(&plan.reviewer_candidates, &reviewers)?;

                    if !prompts.pull_request_confirmer.confirm_pull_request(&plan)? {
                        return Ok(CommandResult {
                            stdout: "cancelled\n".to_owned(),
                        });
                    }

                    progress.status("Creating bookmark…");
                    let bookmark_update = services.ensure_bookmark(
                        &context,
                        &plan.bookmark.branch,
                        &plan.target_commit_id,
                    )?;
                    progress.status("Pushing branch…");
                    let push = services.push_bookmark(&context, &plan.bookmark.branch)?;
                    progress.status("Publishing pull request…");
                    let report =
                        services.publish_pull_request(&context, plan, bookmark_update, push)?;
                    progress.finish();
                    render_pull_request(&report)
                }
            }
        }
    };

    Ok(CommandResult { stdout })
}

fn handle_fetch(
    request: FetchRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<String, CommandError> {
    if request.all {
        return handle_global_fetch(environment, services, progress, output);
    }

    let context = RepositoryContext::discover(environment)?;
    progress.status("Fetching origin…");
    let outcome = services.fetch_origin(&context)?;
    progress.finish();
    let report = domain::fetch_report(&context, outcome);
    render_fetch(&report, environment.current_dir(), output.color).map_err(Into::into)
}

fn handle_global_fetch(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<String, CommandError> {
    let config = WorkflowConfig::discover_global(environment)?;
    let repositories = global_work_repositories(&config, environment)?;
    let mut entries = Vec::new();

    for repository in repositories {
        progress.status(&format!("Fetching {}…", repository.key));
        match global_fetch_for_repository(&repository.root, environment, services) {
            Ok(true) => entries.push(GlobalFetchEntry {
                display_root: display_path(&repository.root, environment),
                result: Ok(()),
            }),
            Ok(false) => {}
            Err(error) => entries.push(GlobalFetchEntry {
                display_root: display_path(&repository.root, environment),
                result: Err(error.to_string()),
            }),
        }
    }

    progress.finish();
    render_global_fetch(&entries, environment.current_dir(), output.color).map_err(Into::into)
}

fn global_fetch_for_repository(
    root: &Path,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<bool, CommandError> {
    let environment = environment.with_current_dir(root);
    let context = match RepositoryContext::discover(&environment) {
        Ok(context) => context,
        Err(_) => return Ok(false),
    };
    if !services.global_fetch_ready(&context)? {
        return Ok(false);
    }

    services.fetch_origin(&context)?;
    Ok(true)
}

fn handle_remote_status(
    request: RemoteStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<String, CommandError> {
    if request.all || request.changed || !request.repo_filters.is_empty() {
        return handle_global_remote_status(request, environment, services, progress, output);
    }

    let context = RepositoryContext::discover(environment)?;
    progress.status("Loading remote status…");
    let workspace = services.status_workspace_facts(&context)?;
    progress.status("Checking GitHub remotes…");
    let report = services.status_report(&context, workspace)?;
    progress.finish();
    render_status(&report, environment.current_dir(), output.color).map_err(Into::into)
}

fn handle_global_remote_status(
    request: RemoteStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<String, CommandError> {
    let config = WorkflowConfig::discover_global(environment)?;
    let repositories = global_work_repositories(&config, environment)?;
    let repositories = filter_work_repositories(&repositories, &request.repo_filters)?;
    let mut entries = Vec::new();

    for repository in repositories {
        progress.status(&format!("Checking {}…", repository.key));
        let result = global_remote_status_for_repository(&repository.root, environment, services)
            .map_err(|error| error.to_string());
        if request.changed
            && result
                .as_ref()
                .is_ok_and(|report| !status_report_has_changes(report))
        {
            continue;
        }
        entries.push(GlobalStatusEntry {
            display_root: display_path(&repository.root, environment),
            result,
        });
    }

    progress.finish();
    render_global_status(&entries, environment.current_dir(), output.color).map_err(Into::into)
}

fn global_remote_status_for_repository(
    root: &Path,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<StatusReport, CommandError> {
    let environment = environment.with_current_dir(root);
    let context = RepositoryContext::discover(&environment)?;
    let workspace = services.status_workspace_facts(&context)?;
    Ok(services.status_report(&context, workspace)?)
}

fn status_report_has_changes(report: &StatusReport) -> bool {
    report.remotes.iter().any(|remote| {
        remote.comparison.state != domain::StatusState::UpToDate || remote.local_ahead_by > 0
    })
}

fn display_path(path: &Path, environment: &RuntimeEnvironment) -> String {
    if let Some(home) = environment.home_dir() {
        if path == home {
            return "~".to_owned();
        }
        if let Ok(relative) = path.strip_prefix(home) {
            return format!("~/{}", relative.display());
        }
    }

    path.display().to_string()
}

fn handle_work(
    request: WorkRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
) -> Result<String, CommandError> {
    match request {
        WorkRequest::Add(request) => {
            let context = LocalRepositoryContext::discover(environment)?;
            validate_workspace_name(&request.name)?;
            let identity = workspace_identity(&context, environment)?;
            let options = WorkspaceAddOptions {
                destination: context.config.layout.workspace_destination(
                    &identity,
                    &request.name,
                    environment,
                )?,
                name: request.name,
                revision: request.revision,
            };
            if options.destination.exists() {
                return Err(RepositoryError::WorkspacePathExists {
                    path: options.destination,
                }
                .into());
            }

            progress.status("Adding workspace…");
            services.add_workspace(environment.current_dir(), &options)?;
            progress.finish();
            Ok(render_work_add(&options))
        }
        WorkRequest::List(request) => {
            if request.all || !request.prefix.is_empty() {
                let config = WorkflowConfig::discover_global(environment)?;
                let locations = global_work_locations(&config, environment)?;
                let locations = filter_work_locations_by_prefix(&locations, &request.prefix);
                Ok(render_global_work_list(&locations))
            } else {
                let workspaces = services.workspace_entries(environment.current_dir())?;
                Ok(render_work_list(&workspaces))
            }
        }
        WorkRequest::Complete(request) => {
            let config = WorkflowConfig::discover_global(environment)?;
            let locations = global_work_locations(&config, environment)?;
            let locations = filter_work_locations_by_prefix(&locations, &request.prefix);
            Ok(render_work_complete(&locations))
        }
        WorkRequest::Root(request) => {
            let config = WorkflowConfig::discover_global(environment)?;
            let locations = global_work_locations(&config, environment)?;
            let root = resolve_work_location(&locations, &request.key)?;
            Ok(render_work_root(&root))
        }
        WorkRequest::Remove(request) => {
            let context = LocalRepositoryContext::discover(environment)?;
            validate_workspace_name(&request.name)?;
            let identity = workspace_identity(&context, environment)?;
            let workspaces = services.workspace_entries(environment.current_dir())?;
            let workspace =
                removable_workspace(&context, &identity, &workspaces, &request.name, environment)?;

            if !prompts
                .workspace_remove_confirmer
                .confirm_workspace_remove(&workspace)?
            {
                return Ok("cancelled\n".to_owned());
            }

            progress.status("Removing workspace…");
            services.remove_workspace(
                environment.current_dir(),
                &WorkspaceRemoveOptions {
                    name: workspace.name.clone(),
                    root: workspace.root.clone(),
                    cleanup_root: context
                        .config
                        .layout
                        .workspace_storage_root(&identity, environment)?,
                },
            )?;
            progress.finish();
            Ok(render_work_remove(&workspace))
        }
    }
}

fn handle_shell(
    request: ShellRequest,
    environment: &RuntimeEnvironment,
) -> Result<String, CommandError> {
    match request {
        ShellRequest::Init(request) => {
            let config = WorkflowConfig::discover_global(environment)?;
            Ok(shell_init_script(request.shell, &config.shell))
        }
    }
}

fn workspace_identity(
    context: &LocalRepositoryContext,
    environment: &RuntimeEnvironment,
) -> Result<RepositoryIdentity, RepositoryError> {
    if let Some(remote) = context
        .remotes
        .iter()
        .find(|remote| remote.name == crate::repository::ORIGIN_REMOTE_NAME)
    {
        if let Ok(identity) = context.config.layout.identity_for_remote_url(&remote.url) {
            return Ok(identity);
        }
    }

    context
        .config
        .layout
        .identity_for_workspace_root(&context.workspace_root, environment)
}

fn removable_workspace(
    context: &LocalRepositoryContext,
    identity: &RepositoryIdentity,
    workspaces: &[WorkspaceEntry],
    name: &str,
    environment: &RuntimeEnvironment,
) -> Result<WorkspaceEntry, RepositoryError> {
    let workspace = workspaces
        .iter()
        .find(|workspace| workspace.name == name)
        .cloned()
        .ok_or_else(|| RepositoryError::WorkspaceNameNotFound {
            name: name.to_owned(),
        })?;

    if workspace.is_current {
        return Err(RepositoryError::RefuseRemoveCurrentWorkspace {
            name: workspace.name,
        });
    }

    let primary = context
        .config
        .layout
        .project_destination(identity, environment)?;
    if workspace.root == primary {
        return Err(RepositoryError::RefuseRemovePrimaryWorkspace {
            name: workspace.name,
            path: workspace.root,
        });
    }

    let managed =
        context
            .config
            .layout
            .workspace_destination(identity, &workspace.name, environment)?;
    if workspace.root != managed {
        return Err(RepositoryError::RefuseRemoveUnmanagedWorkspace {
            name: workspace.name,
            path: workspace.root,
            workspace_dir: context.config.layout.workspace_dir.clone(),
        });
    }

    Ok(workspace)
}

fn handle_sync(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<String, CommandError> {
    let local_context = match LocalRepositoryContext::discover(environment) {
        Ok(context) => context,
        Err(RepositoryError::WorkspaceNotFound) => {
            initialize_layout_repository_for_sync(environment, services, progress)?
        }
        Err(error) => return Err(error.into()),
    };

    sync_local_context(
        local_context,
        environment,
        services,
        progress,
        prompts,
        output,
    )
}

fn initialize_layout_repository_for_sync(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
) -> Result<LocalRepositoryContext, CommandError> {
    let config = WorkflowConfig::discover_for_uninitialized(environment)?;
    let workspace_root = uninitialized_layout_workspace_root(&config.layout, environment)?;

    progress.status("Initializing jj repository…");
    services.init_repository(&workspace_root)?;
    progress.finish();

    Ok(LocalRepositoryContext::discover(environment)?)
}

fn uninitialized_layout_workspace_root(
    layout: &LayoutConfig,
    environment: &RuntimeEnvironment,
) -> Result<PathBuf, RepositoryError> {
    for candidate in environment.current_dir().ancestors() {
        match layout.identity_for_workspace_root(candidate, environment) {
            Ok(_) => return Ok(candidate.to_path_buf()),
            Err(RepositoryError::LayoutPathNotMatched { .. }) => {}
            Err(error) => return Err(error),
        }
    }

    Err(RepositoryError::LayoutPathNotMatched {
        path: environment.current_dir().to_path_buf(),
    })
}

fn sync_local_context(
    local_context: LocalRepositoryContext,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<String, CommandError> {
    if local_context
        .remotes
        .iter()
        .any(|remote| remote.name == crate::repository::ORIGIN_REMOTE_NAME)
    {
        return sync_existing_origin(
            local_context.into_origin_context()?,
            environment,
            services,
            progress,
            output,
        );
    }
    if local_context.has_remotes() {
        return Err(RepositoryError::MissingOrigin.into());
    }

    sync_missing_origin(
        local_context,
        environment,
        services,
        progress,
        prompts,
        output,
    )
}

fn sync_missing_origin(
    local_context: LocalRepositoryContext,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<String, CommandError> {
    let identity = local_context
        .config
        .layout
        .identity_for_workspace_root(&local_context.workspace_root, environment)?;
    let repository = identity.github_repository();
    let remote_url = local_context
        .config
        .layout
        .remote_url_for_identity(&identity)?;
    let target = services.initial_publish_target(&local_context.workspace_root)?;
    let plan = RepositoryBootstrapPlan {
        repository,
        repository_url: identity.github_repository().https_url(),
        remote_url,
        branch: "main".to_owned(),
        target,
    };

    if !prompts
        .repository_creation_confirmer
        .confirm_repository_creation(&plan)?
    {
        return Ok("cancelled\n".to_owned());
    }

    let target =
        services.prepare_initial_publish_target(&local_context.workspace_root, &plan.target)?;

    progress.status(&format!("Creating private {} repository", plan.remote_url));
    let creation = services.create_repository(&local_context, &plan.repository)?;
    progress.status(&format!(
        "Pushing {} to {}",
        target.short_commit_id, plan.branch
    ));
    let push =
        services.bootstrap_origin_main(&local_context.workspace_root, &plan.remote_url, &target)?;
    progress.finish();

    render_repository_bootstrap(
        &RepositoryBootstrapReport {
            repository_url: creation.html_url,
            remote_url: plan.remote_url,
            push,
        },
        environment.current_dir(),
        output.color,
    )
    .map_err(Into::into)
}

fn sync_existing_origin(
    context: RepositoryContext,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<String, CommandError> {
    progress.status("Fetching origin…");
    let fetch = services.fetch_origin(&context)?;
    if let Err(error) = domain::ensure_fetch_is_pushable(&fetch) {
        progress.finish();
        return Err(error.into());
    }
    if context
        .config
        .repo
        .advance_trunk_enabled_for(&context.origin.github)
    {
        progress.status("Advancing trunk bookmark…");
        services.advance_trunk_for_sync(&context)?;
    }
    progress.status("Pushing tracked bookmarks…");
    let push = services.push_tracked(&context)?;
    progress.status("Loading pull requests…");
    let pull_requests = services.sync_pull_requests(&context, &push)?;
    progress.finish();
    let report = domain::sync_report(&context, fetch, push, pull_requests);
    render_sync(&report, environment.current_dir(), output.color).map_err(Into::into)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RepositoryBootstrapPlan {
    pub(super) repository: GitHubRepository,
    pub(super) repository_url: String,
    pub(super) remote_url: String,
    pub(super) branch: String,
    pub(super) target: InitialPublishTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RepositoryBootstrapReport {
    pub(super) repository_url: String,
    pub(super) remote_url: String,
    pub(super) push: BootstrapPushOutcome,
}
