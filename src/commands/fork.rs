use super::*;

const DEFAULT_FORK_BRANCH: &str = "main";

pub(super) fn handle_fork(
    request: ForkRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    output: OutputMode,
) -> Result<CommandResult, CommandError> {
    match request {
        ForkRequest::Sync(request) => {
            handle_fork_sync(request, environment, services, progress, prompts, output)
        }
    }
}

fn handle_fork_sync(
    request: ForkSyncRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: &PromptHandlers<'_>,
    _output: OutputMode,
) -> Result<CommandResult, CommandError> {
    let context = RepositoryContext::discover(environment)?;
    let source = fork_sync_source(&request, &context, services)?;
    let branch = request
        .branch
        .clone()
        .or_else(|| source.default_branch.clone())
        .unwrap_or_else(|| DEFAULT_FORK_BRANCH.to_owned());
    let source_branch = branch.clone();

    if source.repository == context.origin.github {
        return Err(WorkflowError::ForkSourceMatchesOrigin {
            repository: context.origin.github.slug(),
        }
        .into());
    }
    if same_remote_target(&context.origin.url, &source.remote_url) {
        return Err(WorkflowError::ForkUpstreamMatchesOrigin {
            origin_url: context.origin.url.clone(),
            upstream_url: source.remote_url.clone(),
        }
        .into());
    }

    progress.status("Preparing upstream remote…");
    let upstream = services.ensure_git_remote(
        &context,
        &request.upstream_remote,
        &source.remote_url,
        request.fix_remotes,
        request.remote_setup,
    )?;
    progress.status("Fetching upstream…");
    services.fetch_remote(&context, &request.upstream_remote)?;
    progress.status("Fetching origin…");
    services.fetch_remote(&context, context.origin.name)?;
    progress.status("Planning fork sync…");
    let branch_plan = services.fork_sync_branch_plan(
        &context,
        &branch,
        &request.upstream_remote,
        &source_branch,
    )?;
    let plan = domain::fork_sync_plan(
        &context,
        domain::ForkSyncPlanInput {
            source: source.repository,
            branch,
            source_branch,
            upstream_remote: request.upstream_remote,
            upstream_url: source.remote_url,
            push: request.push,
            branch_plan,
        },
    );
    progress.finish();

    if fork_sync_plan_needs_confirmation(&plan)
        && !prompts.fork_sync_confirmer.confirm_fork_sync(&plan)?
    {
        return Ok(CommandResult::success("cancelled\n".to_owned()));
    }

    progress.status("Updating local fork branch…");
    let outcome = services.apply_fork_sync_branch_plan(&context, &plan.branch_plan)?;
    let push = if should_push_fork_sync(&plan, &outcome) {
        progress.status("Pushing fork branch…");
        Some(services.push_bookmark(&context, &plan.branch)?)
    } else {
        None
    };
    progress.finish();

    let conflicts = fork_sync_outcome_has_conflicts(&outcome);
    let report = domain::fork_sync_report(plan, upstream, outcome, push);
    let exit_code = if conflicts { 1 } else { 0 };
    Ok(CommandResult::with_exit_code(
        render_fork_sync(&report),
        exit_code,
    ))
}

fn should_push_fork_sync(plan: &ForkSyncPlan, outcome: &ForkSyncBranchOutcome) -> bool {
    plan.push
        && !fork_sync_outcome_has_conflicts(outcome)
        && (!matches!(outcome.operation, ForkSyncBranchOutcomeKind::AlreadySynced)
            || plan.branch_plan.push_needed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForkSyncSource {
    repository: GitHubRepository,
    default_branch: Option<String>,
    remote_url: String,
}

fn fork_sync_source(
    request: &ForkSyncRequest,
    context: &RepositoryContext,
    services: &dyn CommandServices,
) -> Result<ForkSyncSource, CommandError> {
    if let Some(remote_url) = &request.upstream_url {
        let repository = GitHubRepository::parse(remote_url).map_err(|_| {
            WorkflowError::InvalidForkUpstreamUrl {
                url: remote_url.clone(),
            }
        })?;
        return Ok(ForkSyncSource {
            repository,
            default_branch: None,
            remote_url: remote_url.clone(),
        });
    }

    let fork = services
        .repository_fork(context)?
        .ok_or_else(|| WorkflowError::NotGitHubFork {
            repository: context.origin.github.slug(),
        })?;
    let remote_url = fork.source.ssh_url();

    Ok(ForkSyncSource {
        repository: fork.source,
        default_branch: fork.source_default_branch,
        remote_url,
    })
}
