use super::*;

/// Validates local and GitHub assumptions before a publishing command mutates state.
pub async fn check_readiness(
    context: &RepositoryContext,
    workspace: WorkspaceFacts,
    github: &dyn GitHubClient,
) -> Result<CheckReport, WorkflowError> {
    let user = github.authenticated_user().await?;
    let access = github.repository_access(&context.origin.github).await?;
    ensure_repository_access(&context.origin.github, &access)?;
    let bookmark = plan_bookmark(BookmarkPlanRequest {
        github_login: &user.login,
        task_id: None,
        workspace: &workspace,
    })?;

    Ok(CheckReport {
        repository: repository_summary(context),
        workspace: CheckWorkspaceSummary {
            trunk_branch: workspace.origin_branch,
            trunk_short_commit_id: workspace.trunk.short_commit_id,
            current_short_commit_id: workspace.target_change.short_commit_id,
            current_is_empty: workspace.target_change.is_empty,
            stack_index: workspace.stack_index,
        },
        github: GitHubReadiness {
            login: user.login,
            default_branch: access.default_branch,
            can_push: access.can_push,
        },
        bookmark,
    })
}

/// Plans the PR bookmark intent without jj mutation.
pub async fn bookmark_report(
    context: &RepositoryContext,
    workspace: WorkspaceFacts,
    github: &dyn GitHubClient,
    task_id: Option<String>,
) -> Result<BookmarkReport, WorkflowError> {
    let user = github.authenticated_user().await?;
    let access = github.repository_access(&context.origin.github).await?;
    ensure_repository_access(&context.origin.github, &access)?;
    let bookmark = plan_bookmark(BookmarkPlanRequest {
        github_login: &user.login,
        task_id: task_id.as_deref(),
        workspace: &workspace,
    })?;

    Ok(BookmarkReport {
        repository: repository_summary(context),
        task_id,
        bookmark,
    })
}

fn ensure_repository_access(
    repository: &GitHubRepository,
    access: &RepositoryAccess,
) -> Result<(), WorkflowError> {
    let slug = repository.slug();

    if !access.can_read {
        return Err(WorkflowError::MissingReadAccess { repository: slug });
    }

    if !access.can_push {
        return Err(WorkflowError::MissingPushAccess { repository: slug });
    }

    Ok(())
}
