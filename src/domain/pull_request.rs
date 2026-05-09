use super::*;

/// Plans PR metadata and bookmark intent before jj or GitHub mutation.
pub async fn pull_request_plan(
    context: &RepositoryContext,
    workspace: WorkspaceFacts,
    github: &dyn GitHubClient,
    task_id: Option<String>,
    labels: Vec<String>,
    draft: bool,
) -> Result<PullRequestPlan, WorkflowError> {
    let (title, body) = pull_request_description(&workspace)?;
    let base = workspace
        .nearest_ancestor_bookmark
        .clone()
        .unwrap_or_else(|| workspace.origin_branch.clone());
    let target_commit_id = workspace.target_change.commit_id.clone();
    let changed_files = workspace.changed_files.clone();
    let reviewer_candidates = context
        .config
        .repo
        .reviewer_candidates_for(&context.origin.github, &workspace.changed_files);
    let bookmark_report = bookmark_report(context, workspace, github, task_id).await?;
    let head = PullRequestHead::same_repository(
        &context.origin.github.owner,
        &bookmark_report.bookmark.branch,
    );
    let existing_pull_request = github
        .find_open_pull_request(&context.origin.github, &head)
        .await?;
    let reviewers = reviewer_selection_from_candidates(&reviewer_candidates);

    Ok(PullRequestPlan {
        repository: bookmark_report.repository,
        task_id: bookmark_report.task_id,
        bookmark: bookmark_report.bookmark,
        target_commit_id,
        title,
        body,
        changed_files,
        base,
        head,
        labels,
        draft,
        existing_pull_request,
        reviewer_candidates,
        reviewers,
    })
}

/// Creates or updates a PR after the selected bookmark has been pushed.
fn reviewer_selection_from_candidates(candidates: &[ReviewerCandidate]) -> ReviewerSelection {
    let mut users = Vec::new();
    let mut teams = Vec::new();
    for candidate in candidates {
        match &candidate.target {
            ReviewerTarget::User { login } => users.push(login.clone()),
            ReviewerTarget::Team { slug, .. } => teams.push(slug.clone()),
        }
    }

    ReviewerSelection::new(users, teams)
}

pub async fn publish_pull_request(
    context: &RepositoryContext,
    plan: PullRequestPlan,
    bookmark_update: BookmarkUpdate,
    push: PushOutcome,
    github: &dyn GitHubClient,
) -> Result<PullRequestReport, WorkflowError> {
    let existing = plan.existing_pull_request.clone();
    let (action, pull_request) = if let Some(existing) = existing {
        let request = PullRequestUpdate {
            title: Some(plan.title.clone()),
            body: Some(plan.body.clone()),
            base: Some(plan.base.clone()),
        };
        let pull_request = github
            .update_pull_request(&context.origin.github, existing.number, request)
            .await?;

        (PullRequestAction::Updated, pull_request)
    } else {
        let request = PullRequestCreate {
            title: plan.title.clone(),
            body: Some(plan.body.clone()),
            head: plan.head.clone(),
            base: plan.base.clone(),
            draft: plan.draft,
        };
        let pull_request = github
            .create_pull_request(&context.origin.github, request)
            .await?;

        (PullRequestAction::Created, pull_request)
    };

    let labels = if plan.labels.is_empty() {
        None
    } else {
        Some(
            github
                .add_labels(
                    &context.origin.github,
                    pull_request.number,
                    plan.labels.clone(),
                )
                .await?,
        )
    };

    let reviewers = if plan.reviewers.is_empty() {
        None
    } else {
        Some(
            github
                .sync_reviewers(
                    &context.origin.github,
                    pull_request.number,
                    plan.reviewers.clone(),
                )
                .await?,
        )
    };

    Ok(PullRequestReport {
        repository: plan.repository,
        task_id: plan.task_id,
        bookmark: plan.bookmark,
        bookmark_update,
        push,
        action,
        pull_request,
        base: plan.base,
        head: plan.head,
        labels,
        reviewers,
    })
}

fn pull_request_description(workspace: &WorkspaceFacts) -> Result<(String, String), WorkflowError> {
    if workspace.target_change.is_empty {
        return Err(WorkflowError::EmptyPullRequestChange);
    }

    let body = workspace.target_change.description.trim();
    let Some(title) = body.lines().find_map(|line| {
        let line = line.trim();
        (!line.is_empty()).then_some(line)
    }) else {
        return Err(WorkflowError::MissingPullRequestDescription);
    };

    Ok((title.to_owned(), body.to_owned()))
}
