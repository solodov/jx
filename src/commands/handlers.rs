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
            handle_work(request, environment, services, progress, &prompts, output)?
        }
        CommandRequest::Stack(request) => {
            return handle_stack(request, environment, services, progress, &prompts, output);
        }
        CommandRequest::Shell(request) => handle_shell(request, environment)?,
        CommandRequest::Open(request) => handle_open(request, environment, services)?,
        CommandRequest::Review(request) => {
            handle_review(request, environment, services, progress, output)?
        }
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
        CommandRequest::Push(request) => {
            let context = RepositoryContext::discover(environment)?;
            if request.tracked {
                let changed_files = services.changed_files_for_tracked_push(&context)?;
                run_repo_checks(&context, services, RepoCheckTrigger::Push, &changed_files)?;
                progress.status("Pushing tracked bookmarks…");
                let outcome = services.push_tracked(&context)?;
                progress.finish();
                let report = domain::tracked_push_report(&context, outcome);
                render_tracked_push(&report, environment.current_dir(), output.color)?
            } else {
                progress.status("Planning push…");
                let workspace =
                    services.push_workspace_facts(&context, request.revision.as_deref())?;
                run_repo_checks(
                    &context,
                    services,
                    RepoCheckTrigger::Push,
                    &workspace.changed_files,
                )?;
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
        CommandRequest::Check => {
            let context = RepositoryContext::discover(environment)?;
            let workspace = services.workspace_facts(&context, None)?;
            let report = services.check_readiness(&context, workspace)?;
            render_check(&report, environment.current_dir(), output.color)?
        }
    };

    Ok(CommandResult::success(stdout))
}

pub(super) fn render_pull_request_with_effects(
    report: &PullRequestReport,
    stack_update: &PullRequestStackPublishUpdate,
    services: &dyn CommandServices,
    color: bool,
) -> Result<String, CommandError> {
    let mut output = render_pull_request(report);
    let pull_request =
        linked_pull_request_text(&report.repository.github_url, &report.pull_request);

    if !stack_update.is_empty() {
        let pull_requests = stack_update
            .pull_requests
            .iter()
            .map(|pull_request| {
                linked_pull_request_text(&report.repository.github_url, pull_request)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let line = format!("Stack: refreshed stack context on {pull_requests}");
        output.push_str(&style_log_line(&line, color));
        output.push('\n');
    }

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

pub(super) fn added_labels_summary(labels: &[String]) -> String {
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
) -> Result<String, CommandError> {
    let urls = match &request.target {
        OpenTarget::Repository => open_targets(&request, environment)?
            .iter()
            .map(|target| target.repository.https_url())
            .collect::<Vec<_>>(),
        OpenTarget::PullRequest { selector } => {
            vec![selected_pull_request_url(
                environment,
                services,
                selector.as_deref(),
            )?]
        }
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
    output_mode: OutputMode,
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
                Ok(render_work_list(&workspaces, output_mode.color))
            }
        }
        WorkRequest::Complete(request) => handle_work_complete(request, environment, services),
        WorkRequest::Root(request) => handle_work_root(request, environment, services),
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

            let hooks = context.config.repo.hooks_for(
                &identity.github_repository(),
                RepoHookEvent::WorkspaceDeleteBefore,
            );
            let hook_effects = if hooks.is_empty() {
                Vec::new()
            } else {
                progress.status("Running workspace delete hooks…");
                run_repo_hooks(
                    environment,
                    services,
                    &identity.github_repository(),
                    &removal.workspace,
                    RepoHookEvent::WorkspaceDeleteBefore,
                    hooks,
                )?
            };

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
            append_repo_hook_effects(
                &mut output,
                &hook_effects,
                output_mode.color || request.shell_cd_target,
            );
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

fn handle_work_root(
    request: WorkRootRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<String, CommandError> {
    let mut span = PerfLog::from_environment(environment).start(
        "work.root",
        [
            perf_attr("navigation", request.navigation),
            perf_attr("query_len", request.key.len()),
        ],
    );
    let result = handle_work_root_traced(request, environment, services, &mut span);
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn handle_work_root_traced(
    request: WorkRootRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    span: &mut PerfSpan,
) -> Result<String, CommandError> {
    let config = span.measure("discover_global_config", Vec::new(), || {
        WorkflowConfig::discover_global(environment)
    })?;
    let locations = if request.navigation {
        let workspaces = span.measure_with_result_attrs(
            "load_current_workspaces",
            Vec::new(),
            || current_navigation_workspaces(environment, services),
            |result| count_result_attrs(result, "current_workspace_count"),
        )?;
        span.set([perf_attr("current_workspace_count", workspaces.len())]);
        let local_root = span.measure(
            "resolve_local_navigation_target",
            [perf_attr("current_workspace_count", workspaces.len())],
            || {
                resolve_local_navigation_work_location(
                    &config,
                    environment,
                    &workspaces,
                    &request.key,
                )
                .map_err(CommandError::from)
            },
        )?;
        span.set([perf_attr("local_resolution", local_root.is_some())]);
        if let Some(root) = local_root {
            return span.measure("render", Vec::new(), || {
                Ok::<_, CommandError>(render_work_root(&root))
            });
        }

        let global = span.measure_with_result_attrs(
            "discover_global_work_locations",
            Vec::new(),
            || global_work_locations(&config, environment),
            |result| count_result_attrs(result, "global_location_count"),
        )?;
        span.set([perf_attr("global_location_count", global.len())]);
        span.measure_with_result_attrs(
            "compose_navigation_locations",
            [
                perf_attr("current_workspace_count", workspaces.len()),
                perf_attr("global_location_count", global.len()),
            ],
            || navigation_work_locations_from_global(&config, environment, &workspaces, global),
            |result| count_result_attrs(result, "location_count"),
        )?
    } else {
        let global = span.measure_with_result_attrs(
            "discover_global_work_locations",
            Vec::new(),
            || global_work_locations(&config, environment),
            |result| count_result_attrs(result, "global_location_count"),
        )?;
        span.set([perf_attr("global_location_count", global.len())]);
        global
    };
    span.set([perf_attr("location_count", locations.len())]);

    let root = if request.navigation {
        span.measure(
            "resolve_navigation_target",
            [perf_attr("location_count", locations.len())],
            || {
                resolve_navigation_work_location(&locations, &request.key, environment)
                    .map_err(CommandError::from)
            },
        )?
    } else {
        span.measure(
            "resolve_work_location",
            [perf_attr("location_count", locations.len())],
            || resolve_work_location(&locations, &request.key).map_err(CommandError::from),
        )?
    };

    span.measure("render", Vec::new(), || {
        Ok::<_, CommandError>(render_work_root(&root))
    })
}

fn handle_work_complete(
    request: WorkCompleteRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<String, CommandError> {
    let mut span = PerfLog::from_environment(environment).start(
        "work.complete",
        [
            perf_attr("mode", work_complete_mode(&request)),
            perf_attr("format", work_complete_format_label(request.format)),
            perf_attr("has_prefix", !request.prefix.is_empty()),
        ],
    );
    let result = handle_work_complete_traced(request, environment, services, &mut span);
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn handle_work_complete_traced(
    request: WorkCompleteRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    span: &mut PerfSpan,
) -> Result<String, CommandError> {
    if request.workspaces {
        let context = span.measure("discover_local_context", Vec::new(), || {
            LocalRepositoryContext::discover(environment)
        })?;
        let identity = span.measure("resolve_workspace_identity", Vec::new(), || {
            workspace_identity(&context, environment)
        })?;
        let workspaces = span.measure_with_result_attrs(
            "load_workspace_entries",
            Vec::new(),
            || {
                services
                    .workspace_entries(environment.current_dir())
                    .map_err(CommandError::from)
            },
            |result| count_result_attrs(result, "workspace_count"),
        )?;
        let workspaces = span.measure_with_result_attrs(
            "select_deletable_workspaces",
            [perf_attr("workspace_count", workspaces.len())],
            || deletable_workspace_entries(&context, &identity, &workspaces, environment),
            |result| count_result_attrs(result, "deletable_workspace_count"),
        )?;
        let workspaces = span.measure_with_result_attrs(
            "filter_candidates",
            [perf_attr("prefix_len", request.prefix.len())],
            || {
                Ok::<_, CommandError>(filter_workspace_entries_by_prefix(
                    &workspaces,
                    &request.prefix,
                ))
            },
            |result| count_result_attrs(result, "candidate_count"),
        )?;
        span.set([perf_attr("candidate_count", workspaces.len())]);
        return span.measure("render", [perf_attr("format", "simple")], || {
            Ok::<_, CommandError>(render_workspace_name_complete(&workspaces))
        });
    }

    let config = span.measure("discover_global_config", Vec::new(), || {
        WorkflowConfig::discover_global(environment)
    })?;
    if request.navigation {
        let workspaces = span.measure_with_result_attrs(
            "load_current_workspaces",
            Vec::new(),
            || current_navigation_workspaces(environment, services),
            |result| count_result_attrs(result, "current_workspace_count"),
        )?;
        let global = span.measure_with_result_attrs(
            "discover_global_work_locations",
            Vec::new(),
            || global_work_locations(&config, environment),
            |result| count_result_attrs(result, "global_location_count"),
        )?;
        span.set([
            perf_attr("current_workspace_count", workspaces.len()),
            perf_attr("global_location_count", global.len()),
        ]);
        let locations = span.measure_with_result_attrs(
            "compose_navigation_locations",
            [
                perf_attr("current_workspace_count", workspaces.len()),
                perf_attr("global_location_count", global.len()),
            ],
            || navigation_work_locations_from_global(&config, environment, &workspaces, global),
            |result| count_result_attrs(result, "location_count"),
        )?;
        let locations = span.measure_with_result_attrs(
            "filter_candidates",
            [perf_attr("prefix_len", request.prefix.len())],
            || Ok::<_, CommandError>(filter_work_locations_by_prefix(&locations, &request.prefix)),
            |result| count_result_attrs(result, "candidate_count"),
        )?;
        span.set([perf_attr("candidate_count", locations.len())]);
        return span.measure(
            "render",
            [perf_attr(
                "format",
                work_complete_format_label(request.format),
            )],
            || Ok::<_, CommandError>(render_work_complete(&locations, request.format)),
        );
    }

    if request.repositories {
        let repositories = span.measure_with_result_attrs(
            "discover_global_repositories",
            Vec::new(),
            || global_work_repositories(&config, environment),
            |result| count_result_attrs(result, "repository_count"),
        )?;
        let repositories = span.measure_with_result_attrs(
            "filter_candidates",
            [perf_attr("prefix_len", request.prefix.len())],
            || {
                Ok::<_, CommandError>(filter_work_repositories_by_prefix(
                    &repositories,
                    &request.prefix,
                ))
            },
            |result| count_result_attrs(result, "candidate_count"),
        )?;
        span.set([perf_attr("candidate_count", repositories.len())]);
        return span.measure("render", [perf_attr("format", "simple")], || {
            Ok::<_, CommandError>(render_work_repository_complete(&repositories))
        });
    }

    let locations = span.measure_with_result_attrs(
        "discover_global_work_locations",
        Vec::new(),
        || global_work_locations(&config, environment),
        |result| count_result_attrs(result, "global_location_count"),
    )?;
    span.set([perf_attr("global_location_count", locations.len())]);
    let locations = span.measure_with_result_attrs(
        "filter_candidates",
        [perf_attr("prefix_len", request.prefix.len())],
        || Ok::<_, CommandError>(filter_work_locations_by_prefix(&locations, &request.prefix)),
        |result| count_result_attrs(result, "candidate_count"),
    )?;
    span.set([perf_attr("candidate_count", locations.len())]);
    span.measure(
        "render",
        [perf_attr(
            "format",
            work_complete_format_label(request.format),
        )],
        || Ok::<_, CommandError>(render_work_complete(&locations, request.format)),
    )
}

fn count_result_attrs<T, E>(result: &Result<Vec<T>, E>, key: &str) -> Vec<PerfAttr> {
    result
        .as_ref()
        .map(|items| vec![perf_attr(key, items.len())])
        .unwrap_or_default()
}

fn work_complete_mode(request: &WorkCompleteRequest) -> &'static str {
    if request.workspaces {
        "workspaces"
    } else if request.navigation {
        "navigation"
    } else if request.repositories {
        "repositories"
    } else {
        "locations"
    }
}

fn work_complete_format_label(format: WorkCompleteFormat) -> &'static str {
    match format {
        WorkCompleteFormat::Simple => "simple",
        WorkCompleteFormat::Picker => "picker",
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
            .current_workspace_entry(environment.current_dir())
            .map(|workspace| vec![workspace])
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
        ShellRequest::Title => {
            let config = WorkflowConfig::discover_global(environment)?;
            Ok(format!("{}\n", shell_title_context(&config, environment)?))
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
        Some(name) => resolve_workspace_entry_by_fragment(workspaces, name)?,
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
        return handle_global_sync(
            &request.repo_filters,
            request.sync_push_options,
            environment,
            services,
            progress,
            output,
        );
    }

    if request.stack {
        return sync_current_stack(
            request.sync_push_options,
            environment,
            services,
            progress,
            output,
        );
    }

    if request.repo || request.revision.is_none() {
        return sync_current_repository(
            request.sync_push_options,
            environment,
            services,
            progress,
            prompts,
            output,
        );
    }

    sync_selected_revision(
        request.revision.as_deref(),
        request.sync_push_options,
        environment,
        services,
        progress,
        prompts,
        output,
    )
}

fn handle_global_sync(
    repo_filters: &[String],
    sync_push_options: SyncPushOptions,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let config = WorkflowConfig::discover_global(environment)?;
    let repositories = global_work_repositories(&config, environment)?;
    let repositories = filter_work_repositories(&repositories, repo_filters)?;
    let total = repositories.len();
    let mut entries = Vec::new();

    for (index, repository) in repositories.into_iter().enumerate() {
        progress.percentage(&format!("Syncing {}", repository.key), index, total);
        entries.push(GlobalSyncEntry {
            root: repository.root.clone(),
            display_root: display_path(&repository.root, environment),
            outcome: global_sync_for_repository(
                &repository.root,
                sync_push_options,
                environment,
                services,
            ),
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
    sync_push_options: SyncPushOptions,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> GlobalSyncOutcome {
    match try_global_sync_for_repository(root, sync_push_options, environment, services) {
        Ok(outcome) => outcome,
        Err(error) => GlobalSyncOutcome::Error(error.to_string()),
    }
}

fn try_global_sync_for_repository(
    root: &Path,
    sync_push_options: SyncPushOptions,
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

    run_sync_repo_checks(&context, services, || {
        services.changed_files_for_tracked_push(&context)
    })?;

    global_sync_existing_origin(
        context,
        sync_push_options,
        &repository_environment,
        services,
        origin_status.local_ahead_by,
    )
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
    sync_push_options: SyncPushOptions,
    environment: &RuntimeEnvironment,
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
    let push = services.push_syncable_tracked(&context, sync_push_options)?;
    changed |= tracked_push_changed(&push.pushed) || !push.skipped_same_tree_bookmarks.is_empty();
    let manager =
        PullRequestStackManager::new(&context, services, PerfLog::disabled(), environment);
    let _ = manager.sync_pull_requests(&push.pushed)?;

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

fn record_sync_result(span: &mut PerfSpan, result: &Result<CommandResult, CommandError>) {
    match result {
        Ok(result) => {
            span.set([perf_attr("exit_code", u64::from(result.exit_code))]);
            if result.exit_code != 0 {
                span.record_error(format!("exit code {}", result.exit_code));
            }
        }
        Err(error) => {
            span.set([perf_attr("exit_code", 1_u64)]);
            span.record_error(error);
        }
    }
}

fn fetch_result_attrs(result: &Result<FetchOutcome, JjError>) -> Vec<PerfAttr> {
    match result {
        Ok(fetch) => vec![
            perf_attr("changed_remote_bookmarks", fetch.changed_remote_bookmarks),
            perf_attr("changed_remote_tags", fetch.changed_remote_tags),
            perf_attr("abandoned_commits", fetch.abandoned_commits),
            perf_attr("rebased_trunk_children", fetch.rebased_trunk_children),
            perf_attr("rebased_descendants", fetch.rebased_descendants),
            perf_attr("skipped_trunk_children", fetch.skipped_trunk_children),
            perf_attr("current_repaired", fetch.current_repaired),
            perf_attr("rebased_commit_count", fetch.rebased_commits.len()),
            perf_attr(
                "conflicted_rebased_commit_count",
                fetch_conflict_count(fetch),
            ),
            perf_attr("empty_rebased_commit_count", fetch_empty_count(fetch)),
        ],
        Err(_) => Vec::new(),
    }
}

fn fetch_conflict_count(fetch: &FetchOutcome) -> usize {
    fetch
        .rebased_commits
        .iter()
        .filter(|commit| commit.has_conflict)
        .count()
}

fn fetch_empty_count(fetch: &FetchOutcome) -> usize {
    fetch
        .rebased_commits
        .iter()
        .filter(|commit| commit.is_empty)
        .count()
}

fn advance_trunk_result_attrs(result: &Result<AdvanceTrunkOutcome, JjError>) -> Vec<PerfAttr> {
    match result {
        Ok(advance) => vec![
            perf_attr("branch", &advance.branch),
            perf_attr("current_updated", advance.current_updated),
            perf_attr(
                "changed",
                advance.old_short_commit_id != advance.new_short_commit_id,
            ),
        ],
        Err(_) => Vec::new(),
    }
}

fn sync_selection_result_attrs(
    result: &Result<PullRequestStackSyncSelection, CommandError>,
) -> Vec<PerfAttr> {
    match result {
        Ok(selection) => vec![
            perf_attr("branch_count", selection.branches.len()),
            perf_attr("metadata_node_count", selection.metadata.nodes.len()),
        ],
        Err(_) => Vec::new(),
    }
}

fn sync_push_outcome_result_attrs<E>(result: &Result<SyncPushOutcome, E>) -> Vec<PerfAttr> {
    match result {
        Ok(push) => vec![
            perf_attr("pushed_ref_count", push.pushed.pushed_refs),
            perf_attr("bookmark_count", push.pushed.bookmarks.len()),
            perf_attr("pushed_commit_count", push.pushed.pushed_commits.len()),
            perf_attr(
                "skipped_conflicted_count",
                push.skipped_conflicted_bookmarks.len(),
            ),
            perf_attr(
                "skipped_same_tree_count",
                push.skipped_same_tree_bookmarks.len(),
            ),
        ],
        Err(_) => Vec::new(),
    }
}

fn sync_push_metrics_result_attrs(
    result: &Result<SyncPushMetricsOutcome, JjError>,
) -> Vec<PerfAttr> {
    match result {
        Ok(outcome) => sync_push_metric_attrs(&outcome.metrics),
        Err(_) => Vec::new(),
    }
}

fn pull_request_records_result_attrs<E>(
    result: &Result<Vec<PullRequestRecord>, E>,
) -> Vec<PerfAttr> {
    match result {
        Ok(pull_requests) => vec![perf_attr("pull_request_count", pull_requests.len())],
        Err(_) => Vec::new(),
    }
}

fn record_sync_push_metrics(span: &mut PerfSpan, metrics: &SyncPushMetrics) {
    let attrs = sync_push_metric_attrs(metrics);
    record_sync_push_metric_step(
        span,
        "tracked_origin_bookmark_updates",
        metrics.tracked_origin_bookmark_updates_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(
        span,
        "split_conflicted_updates",
        metrics.split_conflicted_updates_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(
        span,
        "push_tracked_updates",
        metrics.push_tracked_updates_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(
        span,
        "tracked_push_trunk",
        metrics.tracked_push_trunk_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(
        span,
        "pushed_bookmark_summaries",
        metrics.pushed_bookmark_summaries_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(
        span,
        "pushed_commits_for_updates",
        metrics.pushed_commits_for_updates_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(
        span,
        "git_push_refs",
        metrics.git_push_refs_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(
        span,
        "export_git_refs",
        metrics.export_git_refs_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(
        span,
        "commit_transaction",
        metrics.commit_transaction_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(
        span,
        "unchanged_tracked_bookmark_summaries",
        metrics.unchanged_tracked_bookmark_summaries_us,
        attrs.clone(),
    );
    record_sync_push_metric_step(span, "total", metrics.total_us, attrs);
}

fn sync_push_metric_attrs(metrics: &SyncPushMetrics) -> Vec<PerfAttr> {
    vec![
        perf_attr("tracked_update_count", metrics.tracked_update_count),
        perf_attr("pushable_update_count", metrics.pushable_update_count),
        perf_attr("skipped_conflicted_count", metrics.skipped_conflicted_count),
        perf_attr("skipped_same_tree_count", metrics.skipped_same_tree_count),
        perf_attr(
            "adopted_remote_head_count",
            metrics.adopted_remote_head_count,
        ),
        perf_attr("pushed_ref_count", metrics.pushed_ref_count),
        perf_attr("pushed_bookmark_count", metrics.pushed_bookmark_count),
        perf_attr("unchanged_bookmark_count", metrics.unchanged_bookmark_count),
        perf_attr("pushed_commit_count", metrics.pushed_commit_count),
        perf_attr("jj_total_us", metrics.total_us),
    ]
}

fn record_sync_push_metric_step(
    span: &mut PerfSpan,
    phase: &str,
    duration_us: u64,
    attrs: Vec<PerfAttr>,
) {
    span.record_step_us(
        format!("push_syncable_tracked.{phase}"),
        duration_us,
        attrs,
        None::<&CommandError>,
    );
}

fn sync_current_stack(
    sync_push_options: SyncPushOptions,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    let mut span = PerfLog::from_environment(environment).start(
        "sync.current_stack",
        [perf_attr("repo", context.origin.github.slug())],
    );
    let result = sync_current_stack_traced(
        sync_push_options,
        context,
        environment,
        services,
        progress,
        output,
        &mut span,
    );
    record_sync_result(&mut span, &result);
    span.end();
    result
}

fn sync_current_stack_traced(
    sync_push_options: SyncPushOptions,
    context: RepositoryContext,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
    span: &mut PerfSpan,
) -> Result<CommandResult, CommandError> {
    let manager = PullRequestStackManager::new(
        &context,
        services,
        PerfLog::from_environment(environment),
        environment,
    );
    progress.status("Selecting stack bookmarks…");
    let selection = span.measure_with_result_attrs(
        "select_stack_bookmarks",
        Vec::new(),
        || sync_stack_selection(&manager),
        sync_selection_result_attrs,
    )?;
    run_sync_repo_checks(&context, services, || {
        services.changed_files_for_bookmarks(&context, &selection.branches)
    })?;
    progress.status("Fetching origin…");
    let fetch = span.measure_with_result_attrs(
        "fetch_origin",
        Vec::new(),
        || services.fetch_origin(&context),
        fetch_result_attrs,
    )?;
    progress.status("Pushing stack bookmarks…");
    let push = span.measure_with_result_attrs(
        "push_stack_bookmarks",
        [perf_attr("branch_count", selection.branches.len())],
        || push_syncable_stack_branches(&context, services, &selection.branches, sync_push_options),
        sync_push_outcome_result_attrs,
    )?;
    progress.status("Syncing pull request descriptions…");
    let pull_requests = span.measure_with_result_attrs(
        "sync_pull_requests",
        [perf_attr("bookmark_count", push.pushed.bookmarks.len())],
        || manager.sync_pull_requests_with_metadata(&push.pushed, &selection.metadata),
        pull_request_records_result_attrs,
    )?;
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

fn sync_stack_selection(
    manager: &PullRequestStackManager<'_>,
) -> Result<PullRequestStackSyncSelection, CommandError> {
    let selection = manager.sync_selection_for_selector(None)?;

    if selection.branches.is_empty() {
        Err(WorkflowError::MissingPullRequest.into())
    } else {
        Ok(selection)
    }
}

pub(super) fn push_syncable_stack_branches(
    context: &RepositoryContext,
    services: &dyn CommandServices,
    branches: &[String],
    sync_push_options: SyncPushOptions,
) -> Result<SyncPushOutcome, CommandError> {
    let mut pushed = TrackedPushOutcome {
        pushed_refs: 0,
        bookmarks: Vec::new(),
        pushed_commits: Vec::new(),
    };
    let mut skipped_conflicted_bookmarks = Vec::new();
    let mut skipped_same_tree_bookmarks = Vec::new();
    let mut seen_bookmarks = BTreeSet::new();
    let mut seen_commits = BTreeSet::new();

    for branch in branches {
        let next = services.push_syncable_revision(context, Some(branch), sync_push_options)?;
        pushed.pushed_refs += next.pushed.pushed_refs;
        pushed.bookmarks.extend(
            next.pushed
                .bookmarks
                .into_iter()
                .filter(|bookmark| seen_bookmarks.insert(bookmark.branch.clone())),
        );
        pushed.pushed_commits.extend(
            next.pushed
                .pushed_commits
                .into_iter()
                .filter(|commit| seen_commits.insert(commit.short_commit_id.clone())),
        );
        skipped_conflicted_bookmarks.extend(next.skipped_conflicted_bookmarks);
        skipped_same_tree_bookmarks.extend(next.skipped_same_tree_bookmarks);
    }

    Ok(SyncPushOutcome {
        pushed,
        skipped_conflicted_bookmarks,
        skipped_same_tree_bookmarks,
    })
}

fn sync_selected_revision(
    revision: Option<&str>,
    sync_push_options: SyncPushOptions,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let context = match RepositoryContext::discover(environment) {
        Ok(context) => context,
        Err(RepositoryError::WorkspaceNotFound) if revision.is_none() => {
            return sync_current_repository(
                sync_push_options,
                environment,
                services,
                progress,
                prompts,
                output,
            );
        }
        Err(error) => return Err(error.into()),
    };
    let mut span = PerfLog::from_environment(environment).start(
        "sync.selected_revision",
        [
            perf_attr("repo", context.origin.github.slug()),
            perf_attr("has_revision", revision.is_some()),
        ],
    );
    let selection = SelectedSyncPush {
        revision,
        options: sync_push_options,
    };
    let result = sync_selected_revision_traced(
        selection,
        context,
        environment,
        services,
        progress,
        output,
        &mut span,
    );
    record_sync_result(&mut span, &result);
    span.end();
    result
}

#[derive(Debug, Clone, Copy)]
struct SelectedSyncPush<'a> {
    revision: Option<&'a str>,
    options: SyncPushOptions,
}

fn sync_selected_revision_traced(
    selection: SelectedSyncPush<'_>,
    context: RepositoryContext,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
    span: &mut PerfSpan,
) -> Result<CommandResult, CommandError> {
    let workspace = services.workspace_facts(&context, selection.revision)?;
    run_sync_repo_checks(&context, services, || Ok(workspace.changed_files.clone()))?;
    progress.status("Fetching origin…");
    let fetch = span.measure_with_result_attrs(
        "fetch_origin",
        Vec::new(),
        || services.fetch_origin(&context),
        fetch_result_attrs,
    )?;
    progress.status("Pushing selected bookmark…");
    let push = span.measure_with_result_attrs(
        "push_syncable_revision",
        Vec::new(),
        || services.push_syncable_revision(&context, selection.revision, selection.options),
        sync_push_outcome_result_attrs,
    )?;
    progress.status("Syncing pull request description…");
    let manager = PullRequestStackManager::new(
        &context,
        services,
        PerfLog::from_environment(environment),
        environment,
    );
    let pull_requests = span.measure_with_result_attrs(
        "sync_pull_requests",
        [perf_attr("bookmark_count", push.pushed.bookmarks.len())],
        || manager.sync_pull_requests(&push.pushed),
        pull_request_records_result_attrs,
    )?;
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
    sync_push_options: SyncPushOptions,
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
        sync_push_options,
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
    sync_push_options: SyncPushOptions,
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
            sync_push_options,
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
    sync_push_options: SyncPushOptions,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let mut span = PerfLog::from_environment(environment).start(
        "sync.current_repository",
        [perf_attr("repo", context.origin.github.slug())],
    );
    let result = sync_existing_origin_traced(
        context,
        sync_push_options,
        environment,
        services,
        progress,
        output,
        &mut span,
    );
    record_sync_result(&mut span, &result);
    span.end();
    result
}

fn sync_existing_origin_traced(
    context: RepositoryContext,
    sync_push_options: SyncPushOptions,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
    span: &mut PerfSpan,
) -> Result<CommandResult, CommandError> {
    run_sync_repo_checks(&context, services, || {
        services.changed_files_for_tracked_push(&context)
    })?;
    progress.status("Fetching origin…");
    let fetch = span.measure_with_result_attrs(
        "fetch_origin",
        Vec::new(),
        || services.fetch_origin(&context),
        fetch_result_attrs,
    )?;
    let advance_trunk = context
        .config
        .repo
        .advance_trunk_enabled_for(&context.origin.github);
    span.set([perf_attr("advance_trunk", advance_trunk)]);
    if advance_trunk {
        progress.status("Advancing trunk bookmark…");
        span.measure_with_result_attrs(
            "advance_trunk",
            Vec::new(),
            || services.advance_trunk_for_sync(&context),
            advance_trunk_result_attrs,
        )?;
    }
    progress.status("Pushing tracked bookmarks…");
    let push_step = span.start_step("push_syncable_tracked", Vec::new());
    let push_result = services.push_syncable_tracked_with_metrics(&context, sync_push_options);
    if let Ok(outcome) = &push_result {
        record_sync_push_metrics(span, &outcome.metrics);
    }
    span.finish_step(
        push_step,
        sync_push_metrics_result_attrs(&push_result),
        push_result.as_ref().err(),
    );
    let push = push_result?.outcome;
    progress.status("Syncing pull request descriptions…");
    let manager = PullRequestStackManager::new(
        &context,
        services,
        PerfLog::from_environment(environment),
        environment,
    );
    let pull_requests = span.measure_with_result_attrs(
        "sync_pull_requests",
        [perf_attr("bookmark_count", push.pushed.bookmarks.len())],
        || manager.sync_pull_requests(&push.pushed),
        pull_request_records_result_attrs,
    )?;
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
