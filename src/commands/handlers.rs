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
        CommandRequest::Open(request) => handle_open(request, environment, services, &prompts)?,
        CommandRequest::PreviousCommit => {
            services.previous_commit_log(environment.current_dir())?
        }
        CommandRequest::NextCommit => services.next_commit_log(environment.current_dir())?,
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
                        return Ok(CommandResult::success("cancelled\n".to_owned()));
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
        CommandRequest::Sync(request) => {
            return handle_sync(request, environment, services, progress, &prompts, output);
        }
        CommandRequest::Workflow {
            command,
            task_id,
            no_task_id,
            commit,
            labels,
            reviewers,
            draft,
            no_event_handlers,
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
                    let task_id = match (task_id, no_task_id) {
                        (Some(task_id), _) => Some(task_id),
                        (None, true) => None,
                        (None, false) => read_workspace_metadata(&context.workspace_root)?.task_id,
                    };
                    progress.status("Planning pull request…");
                    let publish_options = PullRequestPublishOptions {
                        event_handlers: !no_event_handlers,
                    };
                    let mut selected_revision = commit;
                    let mut workspace =
                        services.workspace_facts(&context, selected_revision.as_deref())?;
                    let mut prepare_effects = Vec::new();
                    let prepare_report = domain::prepare_pull_request_change(
                        &context,
                        &workspace,
                        task_id.as_deref(),
                        publish_options,
                    );
                    if prepare_report.changed {
                        let rewrite = services.rewrite_commit_description(
                            &context,
                            &workspace.target_change.commit_id,
                            &prepare_report.description,
                        )?;
                        selected_revision = Some(rewrite.commit_id);
                        workspace =
                            services.workspace_facts(&context, selected_revision.as_deref())?;
                    }
                    prepare_effects.extend(prepare_report.event_effects);
                    let status = services
                        .workspace_status(environment.current_dir(), io::stderr().is_terminal())?;
                    let mut plan =
                        services.pull_request_plan(&context, workspace, task_id, labels, draft)?;
                    progress.finish();
                    prompts
                        .pull_request_previewer
                        .show_preview(&plan, &status, &prepare_effects);
                    plan.reviewers = prompts
                        .reviewer_selector
                        .select_reviewers(&plan.reviewer_candidates, &reviewers)?;

                    if !prompts.pull_request_confirmer.confirm_pull_request(&plan)? {
                        return Ok(CommandResult::success("cancelled\n".to_owned()));
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
                    let report = services.publish_pull_request(
                        &context,
                        plan,
                        bookmark_update,
                        push,
                        publish_options,
                    )?;
                    progress.finish();
                    render_pull_request_with_effects(&report, services, output.color)?
                }
            }
        }
    };

    Ok(CommandResult::success(stdout))
}

fn render_pull_request_with_effects(
    report: &PullRequestReport,
    services: &dyn CommandServices,
    color: bool,
) -> Result<String, CommandError> {
    let mut output = render_pull_request(report);
    let pull_request =
        linked_pull_request_text(&report.repository.github_url, &report.pull_request);

    for effect in &report.event_effects {
        if !pull_request_event_effect_is_default_visible(effect) {
            continue;
        }

        let summary = match &effect.kind {
            PullRequestEventEffectKind::AddLabels { labels } => added_labels_summary(labels),
            PullRequestEventEffectKind::LabelsAlreadyPresent { .. } => continue,
            PullRequestEventEffectKind::OpenPullRequest { url } => match services.open_url(url) {
                Ok(()) => format!("opened {pull_request}"),
                Err(error) => format!("could not open {pull_request}: {error}"),
            },
            PullRequestEventEffectKind::TitleAlready { .. } => continue,
            PullRequestEventEffectKind::UpdatedTitle { .. } => {
                "added task ID to the title".to_owned()
            }
        };
        let line = format!(
            "Event[{}]: {summary}",
            pull_request_event_display_name(effect)
        );
        output.push_str(&style_log_line(&line, color));
        output.push('\n');
    }
    Ok(output)
}

fn added_labels_summary(labels: &[String]) -> String {
    match labels {
        [] => "added labels".to_owned(),
        [label] => format!("added label {label}"),
        labels => format!("added labels {}", labels.join(", ")),
    }
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

    if let Some(repository) = request.repository {
        let repository_environment = repository_environment(&repository, environment)?;
        return fetch_current_repository(&repository_environment, services, progress, output);
    }

    fetch_current_repository(environment, services, progress, output)
}

fn fetch_current_repository(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<String, CommandError> {
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
    let total = repositories.len();
    let mut entries = Vec::new();

    for (index, repository) in repositories.into_iter().enumerate() {
        progress.percentage(&format!("Fetching {}", repository.key), index, total);
        match global_fetch_for_repository(&repository.root, environment, services) {
            Ok(true) => entries.push(GlobalFetchEntry {
                root: repository.root.clone(),
                display_root: display_path(&repository.root, environment),
                result: Ok(()),
            }),
            Ok(false) => {}
            Err(error) => entries.push(GlobalFetchEntry {
                root: repository.root.clone(),
                display_root: display_path(&repository.root, environment),
                result: Err(error.to_string()),
            }),
        }
        progress.percentage(&format!("Fetching {}", repository.key), index + 1, total);
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

fn handle_open(
    request: OpenRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    prompts: &PromptHandlers<'_>,
) -> Result<String, CommandError> {
    let urls = match &request.target {
        OpenTarget::Repository => open_targets(&request, environment)?
            .iter()
            .map(|target| target.repository.https_url())
            .collect::<Vec<_>>(),
        OpenTarget::PullRequest {
            selector,
            interactive,
        } => vec![if *interactive {
            interactive_pull_request_url(environment, services, prompts.pull_request_selector)?
        } else {
            selected_pull_request_url(environment, services, selector.as_deref())?
        }],
        OpenTarget::PullRequests { all } => {
            let targets = open_targets(&request, environment)?;
            vec![pull_requests_url(&targets, *all, services)?]
        }
    };

    if request.print {
        return Ok(render_url_list(&urls));
    }

    for url in &urls {
        services.open_url(url)?;
    }

    Ok(render_opened_urls(&urls))
}

fn selected_pull_request_url(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    selector: Option<&str>,
) -> Result<String, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    for branch in services.pull_request_candidate_bookmarks(&context, selector)? {
        if let Some(pull_request) = services.find_pull_request_for_head(&context, &branch)? {
            return Ok(pull_request_url(
                &context.origin.github.https_url(),
                &pull_request,
            ));
        }
    }

    Err(WorkflowError::MissingPullRequest.into())
}

fn interactive_pull_request_url(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    selector: &dyn PullRequestSelector,
) -> Result<String, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    let branches = services.pull_request_bookmarks(&context)?;
    if branches.is_empty() {
        return Err(WorkflowError::MissingLocalBookmarkPullRequests {
            repository: context.origin.github.slug(),
        }
        .into());
    }

    let author = services.authenticated_login(&context.token_source)?;
    let mut pull_requests = Vec::new();
    let mut seen_numbers = BTreeSet::new();
    for branch in branches {
        let Some(pull_request) =
            services.find_authored_open_pull_request_for_head(&context, &branch, &author)?
        else {
            continue;
        };
        if seen_numbers.insert(pull_request.number) {
            pull_requests.push(pull_request);
        }
    }
    if pull_requests.is_empty() {
        return Err(WorkflowError::MissingLocalBookmarkPullRequests {
            repository: context.origin.github.slug(),
        }
        .into());
    }

    let selected = selector.select_pull_request(&pull_requests)?;
    Ok(pull_request_url(
        &context.origin.github.https_url(),
        &selected,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenTargetInfo {
    repository: GitHubRepository,
    token_source: Option<TokenSource>,
}

fn open_targets(
    request: &OpenRequest,
    environment: &RuntimeEnvironment,
) -> Result<Vec<OpenTargetInfo>, CommandError> {
    if let Some(repository) = &request.repository {
        return Ok(vec![open_target_for_repository_key(
            repository,
            environment,
        )?]);
    }
    if !request.repo_filters.is_empty() {
        let config = WorkflowConfig::discover_global(environment)?;
        let repositories = global_work_repositories(&config, environment)?;
        return filter_work_repositories(&repositories, &request.repo_filters)?
            .iter()
            .map(|repository| open_target_for_root(&repository.root, environment))
            .collect();
    }

    Ok(vec![open_target_for_environment(environment)?])
}

fn open_target_for_repository_key(
    key: &str,
    environment: &RuntimeEnvironment,
) -> Result<OpenTargetInfo, CommandError> {
    let config = WorkflowConfig::discover_global(environment)?;
    let repositories = global_work_repositories(&config, environment)?;
    let repository = resolve_work_repository(&repositories, key)?;
    open_target_for_root(&repository.root, environment)
}

fn open_target_for_root(
    root: &Path,
    environment: &RuntimeEnvironment,
) -> Result<OpenTargetInfo, CommandError> {
    open_target_for_environment(&environment.with_current_dir(root))
}

fn open_target_for_environment(
    environment: &RuntimeEnvironment,
) -> Result<OpenTargetInfo, CommandError> {
    match LocalRepositoryContext::discover(environment) {
        Ok(context) => open_target_for_local_context(context, environment),
        Err(RepositoryError::WorkspaceNotFound) => {
            let config = WorkflowConfig::discover_for_uninitialized(environment)?;
            let workspace_root = uninitialized_layout_workspace_root(&config.layout, environment)?;
            let identity = config
                .layout
                .identity_for_workspace_root(&workspace_root, environment)?;
            Ok(OpenTargetInfo {
                repository: identity.github_repository(),
                token_source: None,
            })
        }
        Err(error) => Err(error.into()),
    }
}

fn open_target_for_local_context(
    context: LocalRepositoryContext,
    environment: &RuntimeEnvironment,
) -> Result<OpenTargetInfo, CommandError> {
    let repository = context
        .remotes
        .iter()
        .find(|remote| remote.name == crate::repository::ORIGIN_REMOTE_NAME)
        .and_then(|remote| GitHubRepository::parse(&remote.url).ok())
        .map(Ok)
        .unwrap_or_else(|| {
            context
                .config
                .layout
                .identity_for_workspace_root(&context.workspace_root, environment)
                .map(|identity| identity.github_repository())
        })?;

    Ok(OpenTargetInfo {
        repository,
        token_source: Some(context.token_source),
    })
}

fn pull_requests_url(
    targets: &[OpenTargetInfo],
    all: bool,
    services: &dyn CommandServices,
) -> Result<String, CommandError> {
    let login = if all {
        None
    } else {
        let token_source = targets
            .iter()
            .find_map(|target| target.token_source.as_ref())
            .ok_or(RepositoryError::WorkspaceNotFound)?;
        Some(services.authenticated_login(token_source)?)
    };
    let mut query = vec!["is:pr".to_owned(), "is:open".to_owned()];
    if let Some(login) = login {
        query.push(format!("author:{login}"));
    }

    if let [target] = targets {
        let query = encode_url_query(&query.join(" "));
        return Ok(format!("{}/pulls?q={query}", target.repository.https_url()));
    }

    query.extend(
        targets
            .iter()
            .map(|target| format!("repo:{}", target.repository.slug())),
    );
    Ok(format!(
        "https://github.com/pulls?q={}",
        encode_url_query(&query.join(" "))
    ))
}

fn encode_url_query(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            b' ' => "+".to_owned(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn render_url_list(urls: &[String]) -> String {
    urls.iter().map(|url| format!("{url}\n")).collect()
}

fn render_opened_urls(urls: &[String]) -> String {
    urls.iter().map(|url| format!("Opened: {url}\n")).collect()
}

fn handle_remote_status(
    request: RemoteStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<String, CommandError> {
    if let Some(repository) = &request.repository {
        let repository_environment = repository_environment(repository, environment)?;
        return remote_status_current_repository(
            &request,
            &repository_environment,
            services,
            progress,
            output,
        );
    }

    if request.all || request.changed || !request.repo_filters.is_empty() {
        return handle_global_remote_status(request, environment, services, progress, output);
    }

    remote_status_current_repository(&request, environment, services, progress, output)
}

fn remote_status_current_repository(
    request: &RemoteStatusRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<String, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    progress.status("Loading remote status…");
    let workspace = services.status_workspace_facts(&context)?;
    progress.status("Checking GitHub remotes…");
    let repository = context.origin.github.clone();
    let root = context.workspace_root.clone();
    let report = services.remote_status_report(&context, workspace)?;
    progress.finish();
    if request.changed && !status_report_has_changes(&report) {
        return if request.format == RemoteStatusFormat::Json {
            Ok(render_status_json(&[]))
        } else {
            Ok(String::new())
        };
    }

    if request.format == RemoteStatusFormat::Json {
        return Ok(render_status_json(&[GlobalStatusEntry {
            key: request.repository.clone(),
            root,
            display_root: display_path(environment.current_dir(), environment),
            repository: Some(repository),
            result: Ok(report),
        }]));
    }

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

    if !repositories.is_empty() {
        progress.percentage("Checking remote status", 0, repositories.len());
    }
    let entries =
        services.global_remote_status_entries(&repositories, &request, environment, progress);
    progress.finish();
    if request.format == RemoteStatusFormat::Json {
        Ok(render_status_json(&entries))
    } else {
        render_global_status(
            &entries,
            repositories.len(),
            environment.current_dir(),
            output.color,
        )
        .map_err(Into::into)
    }
}

pub(super) fn status_report_has_changes(report: &StatusReport) -> bool {
    report.remotes.iter().any(|remote| {
        remote.comparison.state != domain::StatusState::UpToDate || remote.local_ahead_by > 0
    }) || report
        .fork
        .as_ref()
        .is_some_and(|fork| fork.comparison.state != domain::ForkStatusState::Synced)
}

fn repository_environment(
    repository: &str,
    environment: &RuntimeEnvironment,
) -> Result<RuntimeEnvironment, CommandError> {
    let config = WorkflowConfig::discover_global(environment)?;
    let repositories = global_work_repositories(&config, environment)?;
    let repository = resolve_work_repository(&repositories, repository)?;

    Ok(environment.with_current_dir(repository.root))
}

pub(super) fn display_path(path: &Path, environment: &RuntimeEnvironment) -> String {
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
            let plan = plan_work_add(&request, &context, environment)?;
            let options = plan.workspace_options();
            progress.status("Adding workspace…");
            services.add_workspace(&plan.primary_checkout_root, &options)?;
            apply_work_add_setup(&plan)?;
            progress.finish();
            let mut output = render_work_add(&plan);
            if request.shell_cd_target {
                output.push_str(SHELL_CD_TARGET_PREFIX);
                output.push_str(&plan.destination.display().to_string());
                output.push('\n');
            }
            Ok(output)
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
            if request.workspaces {
                let context = LocalRepositoryContext::discover(environment)?;
                let identity = workspace_identity(&context, environment)?;
                let workspaces = services.workspace_entries(environment.current_dir())?;
                let workspaces =
                    deletable_workspace_entries(&context, &identity, &workspaces, environment)?;
                let workspaces = filter_workspace_entries_by_prefix(&workspaces, &request.prefix);
                return Ok(render_workspace_name_complete(&workspaces));
            }

            let config = WorkflowConfig::discover_global(environment)?;
            if request.navigation {
                let workspaces = current_navigation_workspaces(environment, services)?;
                let locations = navigation_work_locations(&config, environment, &workspaces)?;
                let locations = filter_work_locations_by_prefix(&locations, &request.prefix);
                Ok(render_work_complete(&locations))
            } else if request.repositories {
                let repositories = global_work_repositories(&config, environment)?;
                let repositories =
                    filter_work_repositories_by_prefix(&repositories, &request.prefix);
                Ok(render_work_repository_complete(&repositories))
            } else {
                let locations = global_work_locations(&config, environment)?;
                let locations = filter_work_locations_by_prefix(&locations, &request.prefix);
                Ok(render_work_complete(&locations))
            }
        }
        WorkRequest::Root(request) => {
            let config = WorkflowConfig::discover_global(environment)?;
            let locations = if request.navigation {
                let workspaces = current_navigation_workspaces(environment, services)?;
                navigation_work_locations(&config, environment, &workspaces)?
            } else {
                global_work_locations(&config, environment)?
            };
            let root = if request.navigation {
                resolve_navigation_work_location(&locations, &request.key, environment)?
            } else {
                resolve_work_location(&locations, &request.key)?
            };
            Ok(render_work_root(&root))
        }
        WorkRequest::Trunk(request) => {
            let context = LocalRepositoryContext::discover(environment)?;
            let identity = workspace_identity(&context, environment)?;
            let trunk = context
                .config
                .layout
                .project_destination(&identity, environment)?;
            if request.shell_cd_target {
                Ok(shell_cd_target(&trunk))
            } else {
                Ok(render_work_root(&trunk))
            }
        }
        WorkRequest::Delete(request) => {
            let context = LocalRepositoryContext::discover(environment)?;
            let identity = workspace_identity(&context, environment)?;
            let removal = if let Some(name) = &request.name {
                validate_workspace_name(name)?;
                let workspaces = services.workspace_entries(environment.current_dir())?;
                removable_workspace(&context, &identity, &workspaces, Some(name), environment)?
            } else {
                let workspace = services.current_workspace_entry(environment.current_dir())?;
                removable_workspace(&context, &identity, &[workspace], None, environment)?
            };

            if !prompts
                .workspace_remove_confirmer
                .confirm_workspace_remove(&removal.workspace)?
            {
                return Ok("cancelled\n".to_owned());
            }

            progress.status("Deleting workspace…");
            services.remove_workspace(
                &removal.operation_dir,
                &WorkspaceRemoveOptions {
                    name: removal.workspace.name.clone(),
                    root: removal.workspace.root.clone(),
                    cleanup_root: context
                        .config
                        .layout
                        .workspace_storage_root(&identity, environment)?,
                },
            )?;
            progress.finish();
            let mut output = render_work_delete(&removal.workspace);
            if request.shell_cd_target {
                if let Some(target) = removal.cd_target {
                    output.push_str(SHELL_CD_TARGET_PREFIX);
                    output.push_str(&target.display().to_string());
                    output.push('\n');
                }
            }
            Ok(output)
        }
    }
}

fn shell_cd_target(path: &Path) -> String {
    format!("{SHELL_CD_TARGET_PREFIX}{}\n", path.display())
}

fn current_navigation_workspaces(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<Vec<WorkspaceEntry>, CommandError> {
    if has_jj_workspace_ancestor(environment.current_dir()) {
        services
            .workspace_entries(environment.current_dir())
            .map_err(CommandError::from)
    } else {
        Ok(Vec::new())
    }
}

fn has_jj_workspace_ancestor(start: &Path) -> bool {
    start
        .ancestors()
        .any(|candidate| candidate.join(".jj").is_dir())
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovableWorkspace {
    workspace: WorkspaceEntry,
    operation_dir: PathBuf,
    cd_target: Option<PathBuf>,
}

fn removable_workspace(
    context: &LocalRepositoryContext,
    identity: &RepositoryIdentity,
    workspaces: &[WorkspaceEntry],
    name: Option<&str>,
    environment: &RuntimeEnvironment,
) -> Result<RemovableWorkspace, RepositoryError> {
    let workspace = match name {
        Some(name) => workspaces
            .iter()
            .find(|workspace| workspace.name == name)
            .cloned()
            .ok_or_else(|| RepositoryError::WorkspaceNameNotFound {
                name: name.to_owned(),
            })?,
        None => workspaces
            .iter()
            .find(|workspace| workspace.is_current)
            .cloned()
            .ok_or(RepositoryError::CurrentWorkspaceNotFound)?,
    };

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

    let cd_target = workspace.is_current.then_some(primary.clone());
    Ok(RemovableWorkspace {
        workspace,
        operation_dir: cd_target
            .as_ref()
            .cloned()
            .unwrap_or_else(|| environment.current_dir().to_path_buf()),
        cd_target,
    })
}

fn handle_sync(
    request: SyncRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    if request.all {
        return handle_global_sync(environment, services, progress, output);
    }

    if request.repo || request.revision.is_none() {
        return sync_current_repository(environment, services, progress, prompts, output);
    }

    sync_selected_revision(
        request.revision.as_deref(),
        environment,
        services,
        progress,
        prompts,
        output,
    )
}

fn handle_global_sync(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let config = WorkflowConfig::discover_global(environment)?;
    let repositories = global_work_repositories(&config, environment)?;
    let total = repositories.len();
    let mut entries = Vec::new();

    for (index, repository) in repositories.into_iter().enumerate() {
        progress.percentage(&format!("Syncing {}", repository.key), index, total);
        entries.push(GlobalSyncEntry {
            root: repository.root.clone(),
            display_root: display_path(&repository.root, environment),
            outcome: global_sync_for_repository(&repository.root, environment, services),
        });
        progress.percentage(&format!("Syncing {}", repository.key), index + 1, total);
    }

    progress.finish();
    let exit_code = if global_sync_has_conflicts(&entries) {
        1
    } else {
        0
    };
    let stdout = render_global_sync(&entries, environment.current_dir(), output.color)?;
    Ok(CommandResult::with_exit_code(stdout, exit_code))
}

fn global_sync_for_repository(
    root: &Path,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> GlobalSyncOutcome {
    match try_global_sync_for_repository(root, environment, services) {
        Ok(outcome) => outcome,
        Err(error) => GlobalSyncOutcome::Error(error.to_string()),
    }
}

fn try_global_sync_for_repository(
    root: &Path,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<GlobalSyncOutcome, CommandError> {
    let repository_environment = environment.with_current_dir(root);
    let context = match RepositoryContext::discover(&repository_environment) {
        Ok(context) => context,
        Err(error @ (RepositoryError::MissingOrigin | RepositoryError::OriginNotGitHub { .. })) => {
            return Ok(GlobalSyncOutcome::Skipped(
                GlobalSyncSkipReason::SetupNeeded(error.to_string()),
            ));
        }
        Err(error) => return Err(error.into()),
    };

    match services.origin_can_push(&context) {
        Ok(true) => {}
        Ok(false) => {
            return Ok(GlobalSyncOutcome::Skipped(
                GlobalSyncSkipReason::ReadOnlyOrigin,
            ));
        }
        Err(error) => {
            return Ok(GlobalSyncOutcome::Skipped(
                GlobalSyncSkipReason::PushAccessUnavailable(error.to_string()),
            ));
        }
    }

    let origin_status = origin_remote_status(&context, services)?;
    let pull = origin_status.comparison.github_ahead_by;
    let push = origin_status.comparison.github_behind_by + origin_status.local_ahead_by;
    match (pull > 0, push > 0) {
        (true, true) => {
            return Ok(GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::Diverged {
                pull,
                push,
            }));
        }
        (true, false) => {
            if !services.global_fetch_ready(&context)? {
                return Ok(GlobalSyncOutcome::Skipped(
                    GlobalSyncSkipReason::PullNeeded { commits: pull },
                ));
            }
        }
        (false, _) => {}
    }

    global_sync_existing_origin(context, services, origin_status.local_ahead_by)
}

fn origin_remote_status(
    context: &RepositoryContext,
    services: &dyn CommandServices,
) -> Result<RemoteStatusReport, CommandError> {
    let context = origin_only_status_context(context);
    let workspace = services.status_workspace_facts(&context)?;
    let report = services.status_report(&context, workspace)?;
    report
        .remotes
        .into_iter()
        .find(|remote| remote.name == crate::repository::ORIGIN_REMOTE_NAME)
        .ok_or_else(|| {
            WorkflowError::MissingStatusRemote {
                remote: crate::repository::ORIGIN_REMOTE_NAME.to_owned(),
            }
            .into()
        })
}

fn origin_only_status_context(context: &RepositoryContext) -> RepositoryContext {
    let mut context = context.clone();
    context
        .github_remotes
        .retain(|remote| remote.name == crate::repository::ORIGIN_REMOTE_NAME);
    if context.github_remotes.is_empty() {
        context.github_remotes.push(GitHubRemote {
            name: crate::repository::ORIGIN_REMOTE_NAME.to_owned(),
            url: context.origin.url.clone(),
            github: context.origin.github.clone(),
        });
    }
    context
}

fn global_sync_existing_origin(
    context: RepositoryContext,
    services: &dyn CommandServices,
    local_ahead_by: i64,
) -> Result<GlobalSyncOutcome, CommandError> {
    let fetch = services.fetch_origin(&context)?;
    let mut changed = fetch_outcome_changed(&fetch);
    if context
        .config
        .repo
        .advance_trunk_enabled_for(&context.origin.github)
    {
        let advance = services.advance_trunk_for_sync(&context)?;
        changed |= advance_trunk_changed(&advance);
    }
    let push = services.push_syncable_tracked(&context)?;
    changed |= tracked_push_changed(&push.pushed);
    let _ = services.sync_pull_requests(&context, &push.pushed)?;

    if let Some(detail) = sync_conflict_detail(&fetch, &push) {
        Ok(GlobalSyncOutcome::SyncedWithConflicts { detail })
    } else if changed {
        Ok(GlobalSyncOutcome::Synced)
    } else if local_ahead_by > 0 {
        Ok(GlobalSyncOutcome::Skipped(
            GlobalSyncSkipReason::LocalWork {
                changes: local_ahead_by,
            },
        ))
    } else {
        Ok(GlobalSyncOutcome::Skipped(GlobalSyncSkipReason::UpToDate))
    }
}

fn fetch_outcome_changed(fetch: &FetchOutcome) -> bool {
    fetch.changed_remote_bookmarks > 0
        || fetch.changed_remote_tags > 0
        || fetch.abandoned_commits > 0
        || fetch.rebased_trunk_children > 0
        || fetch.rebased_descendants > 0
        || fetch.current_repaired
        || !fetch.rebased_commits.is_empty()
}

fn advance_trunk_changed(advance: &AdvanceTrunkOutcome) -> bool {
    advance.old_short_commit_id != advance.new_short_commit_id || advance.current_updated
}

fn tracked_push_changed(push: &TrackedPushOutcome) -> bool {
    push.pushed_refs > 0
        || !push.pushed_commits.is_empty()
        || push
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.old_short_commit_id != bookmark.new_short_commit_id)
}

fn sync_report_has_conflicts(report: &SyncReport) -> bool {
    fetch_has_conflicts(&report.fetch) || !report.skipped_conflicted_bookmarks.is_empty()
}

fn global_sync_has_conflicts(entries: &[GlobalSyncEntry]) -> bool {
    entries.iter().any(|entry| {
        matches!(
            &entry.outcome,
            GlobalSyncOutcome::SyncedWithConflicts { .. }
        )
    })
}

fn sync_conflict_detail(fetch: &FetchOutcome, push: &SyncPushOutcome) -> Option<String> {
    let mut parts = Vec::new();
    let rebased_conflicts = fetch
        .rebased_commits
        .iter()
        .filter(|commit| commit.has_conflict)
        .map(|commit| commit.new_short_commit_id.as_str())
        .collect::<Vec<_>>();
    if !rebased_conflicts.is_empty() {
        parts.push(format!(
            "conflicted rebases: {}",
            rebased_conflicts.join(", ")
        ));
    }

    let skipped_bookmarks = push
        .skipped_conflicted_bookmarks
        .iter()
        .map(|bookmark| bookmark.branch.as_str())
        .collect::<Vec<_>>();
    if !skipped_bookmarks.is_empty() {
        parts.push(format!(
            "skipped bookmarks: {}",
            skipped_bookmarks.join(", ")
        ));
    }

    (!parts.is_empty()).then(|| parts.join("; "))
}

fn fetch_has_conflicts(fetch: &FetchOutcome) -> bool {
    fetch
        .rebased_commits
        .iter()
        .any(|commit| commit.has_conflict)
}

fn sync_selected_revision(
    revision: Option<&str>,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let context = match RepositoryContext::discover(environment) {
        Ok(context) => context,
        Err(RepositoryError::WorkspaceNotFound) if revision.is_none() => {
            return sync_current_repository(environment, services, progress, prompts, output);
        }
        Err(error) => return Err(error.into()),
    };
    progress.status("Fetching origin…");
    let fetch = services.fetch_origin(&context)?;
    progress.status("Pushing selected bookmark…");
    let push = services.push_syncable_revision(&context, revision)?;
    progress.status("Syncing pull request description…");
    let pull_requests = services.sync_pull_requests(&context, &push.pushed)?;
    progress.finish();
    let report = domain::sync_report(&context, fetch, push, pull_requests);
    let exit_code = if sync_report_has_conflicts(&report) {
        1
    } else {
        0
    };
    let stdout = render_sync(&report, environment.current_dir(), output.color)?;
    Ok(CommandResult::with_exit_code(stdout, exit_code))
}

fn sync_current_repository(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let local_context = match LocalRepositoryContext::discover(environment) {
        Ok(context) => context,
        Err(RepositoryError::WorkspaceNotFound) => {
            let Some(context) =
                initialize_layout_repository_for_sync(environment, services, progress, prompts)?
            else {
                return Ok(CommandResult::success("cancelled\n".to_owned()));
            };
            context
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
    prompts: &PromptHandlers<'_>,
) -> Result<Option<LocalRepositoryContext>, CommandError> {
    let config = WorkflowConfig::discover_for_uninitialized(environment)?;
    let workspace_root = uninitialized_layout_workspace_root(&config.layout, environment)?;

    if !prompts
        .repository_initialization_confirmer
        .confirm_repository_initialization(&workspace_root)?
    {
        return Ok(None);
    }

    progress.status("Initializing jj repository…");
    services.init_repository(&workspace_root)?;
    progress.finish();

    Ok(Some(LocalRepositoryContext::discover(environment)?))
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
) -> Result<CommandResult, CommandError> {
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

    let stdout = sync_missing_origin(
        local_context,
        environment,
        services,
        progress,
        prompts,
        output,
    )?;
    Ok(CommandResult::success(stdout))
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
) -> Result<CommandResult, CommandError> {
    progress.status("Fetching origin…");
    let fetch = services.fetch_origin(&context)?;
    if context
        .config
        .repo
        .advance_trunk_enabled_for(&context.origin.github)
    {
        progress.status("Advancing trunk bookmark…");
        services.advance_trunk_for_sync(&context)?;
    }
    progress.status("Pushing tracked bookmarks…");
    let push = services.push_syncable_tracked(&context)?;
    progress.status("Syncing pull request descriptions…");
    let pull_requests = services.sync_pull_requests(&context, &push.pushed)?;
    progress.finish();
    let report = domain::sync_report(&context, fetch, push, pull_requests);
    let exit_code = if sync_report_has_conflicts(&report) {
        1
    } else {
        0
    };
    let stdout = render_sync(&report, environment.current_dir(), output.color)?;
    Ok(CommandResult::with_exit_code(stdout, exit_code))
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
