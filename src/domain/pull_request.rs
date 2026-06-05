use super::*;

/// Plans PR metadata and bookmark intent before jj or GitHub mutation.
pub async fn pull_request_plan(
    context: &RepositoryContext,
    workspace: WorkspaceFacts,
    github: &dyn GitHubClient,
    task_id: Option<String>,
    labels: Vec<String>,
    readiness: PullRequestReadiness,
) -> Result<PullRequestPlan, WorkflowError> {
    let (title, body) = pull_request_description(&workspace)?;
    // Root PRs target trunk, while stacked PRs target the nearest bookmarked stack
    // ancestor. The jj fact excludes trunk bookmarks so alternate labels on trunk
    // do not become accidental PR bases.
    let nearest_ancestor_bookmark = workspace.nearest_ancestor_bookmark.clone();
    let base = nearest_ancestor_bookmark
        .clone()
        .unwrap_or_else(|| workspace.trunk.branch.clone());
    let base_pull_request = if let Some(branch) = nearest_ancestor_bookmark.as_deref() {
        let head = PullRequestHead::same_repository(&context.origin.github.owner, branch);
        github
            .find_open_pull_request(&context.origin.github, &head)
            .await?
    } else {
        None
    };
    let target_commit_id = workspace.target_change.commit_id.clone();
    let changed_files = workspace.changed_files.clone();
    let change_lines = workspace.change_lines.clone();
    let mut reviewer_candidates = context
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
    let existing_reviewers = existing_pull_request
        .as_ref()
        .map(|pull_request| pull_request.reviewers.clone())
        .unwrap_or_default();
    add_existing_reviewers_to_candidates(&mut reviewer_candidates, &existing_reviewers);
    let reviewers = merge_reviewer_selections(
        reviewer_selection_from_candidates(&reviewer_candidates),
        existing_reviewers,
    );
    let existing_review_activity = existing_review_activity_for_pull_request(
        github,
        &context.origin.github,
        existing_pull_request.as_ref(),
    )
    .await?;
    add_existing_review_activity_to_candidates(
        &mut reviewer_candidates,
        existing_review_activity.as_ref(),
    );
    let draft = readiness.desired_draft(existing_pull_request.as_ref());
    let suggested_reviewers = suggested_reviewers_for_ready_draft(
        github,
        &context.origin.github,
        existing_pull_request.as_ref(),
        draft,
    )
    .await?;
    add_suggested_reviewers_to_candidates(&mut reviewer_candidates, suggested_reviewers);

    Ok(PullRequestPlan {
        repository: bookmark_report.repository,
        task_id: bookmark_report.task_id,
        bookmark: bookmark_report.bookmark,
        target_commit_id,
        title,
        body,
        changed_files,
        change_lines,
        base,
        base_pull_request,
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

fn merge_reviewer_selections(
    left: ReviewerSelection,
    right: ReviewerSelection,
) -> ReviewerSelection {
    ReviewerSelection::new(
        left.users.into_iter().chain(right.users),
        left.teams.into_iter().chain(right.teams),
    )
}

fn add_existing_reviewers_to_candidates(
    candidates: &mut Vec<ReviewerCandidate>,
    reviewers: &ReviewerSelection,
) {
    for login in &reviewers.users {
        add_reviewer_candidate_reason(
            candidates,
            ReviewerTarget::user(login.clone()),
            "already requested",
        );
    }
    for slug in &reviewers.teams {
        add_reviewer_candidate_reason(
            candidates,
            ReviewerTarget::team(slug.clone(), slug.clone()),
            "already requested",
        );
    }
}

async fn existing_review_activity_for_pull_request(
    github: &dyn GitHubClient,
    repository: &GitHubRepository,
    existing_pull_request: Option<&PullRequestRecord>,
) -> Result<Option<PullRequestStatusRecord>, WorkflowError> {
    let Some(existing_pull_request) = existing_pull_request else {
        return Ok(None);
    };

    Ok(github
        .pull_request_statuses(repository, &[existing_pull_request.number])
        .await?
        .into_iter()
        .find(|status| status.number == existing_pull_request.number))
}

fn add_existing_review_activity_to_candidates(
    candidates: &mut Vec<ReviewerCandidate>,
    status: Option<&PullRequestStatusRecord>,
) {
    let Some(status) = status else {
        return;
    };

    for login in &status.approved_reviewers {
        add_reviewer_candidate_reason(
            candidates,
            ReviewerTarget::user(login.clone()),
            "already approved",
        );
    }
    for login in &status.commented_reviewers {
        add_reviewer_candidate_reason(candidates, ReviewerTarget::user(login.clone()), "commented");
    }
    for login in &status.addressed_reviewers {
        add_reviewer_candidate_reason(
            candidates,
            ReviewerTarget::user(login.clone()),
            "comments addressed",
        );
    }
}

async fn suggested_reviewers_for_ready_draft(
    github: &dyn GitHubClient,
    repository: &GitHubRepository,
    existing_pull_request: Option<&PullRequestRecord>,
    draft: bool,
) -> Result<Vec<String>, WorkflowError> {
    let Some(existing_pull_request) = existing_pull_request else {
        return Ok(Vec::new());
    };
    if !existing_pull_request.draft || draft {
        return Ok(Vec::new());
    }

    Ok(github
        .pull_request_suggested_reviewers(repository, existing_pull_request.number)
        .await?)
}

fn add_suggested_reviewers_to_candidates(
    candidates: &mut Vec<ReviewerCandidate>,
    suggested_reviewers: Vec<String>,
) {
    for login in suggested_reviewers {
        add_reviewer_candidate_reason(
            candidates,
            ReviewerTarget::user(login),
            "suggested by GitHub",
        );
    }
}

fn add_reviewer_candidate_reason(
    candidates: &mut Vec<ReviewerCandidate>,
    target: ReviewerTarget,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    if let Some(candidate) = candidates
        .iter_mut()
        .find(|candidate| candidate.target.matches_identity(&target))
    {
        if !candidate.reasons.contains(&reason) {
            candidate.reasons.push(reason);
        }
        return;
    }

    candidates.push(ReviewerCandidate::new(target, vec![reason]));
}

/// Applies pre-planning PR handlers to the selected change description.
pub fn prepare_pull_request_change(
    context: &RepositoryContext,
    workspace: &WorkspaceFacts,
    task_id: Option<&str>,
    options: PullRequestPublishOptions,
) -> PullRequestPrepareReport {
    let handlers = if options.event_handlers {
        context
            .config
            .repo
            .event_handlers_for(&context.origin.github)
    } else {
        Vec::new()
    };
    let mut subject = PullRequestPrepareSubject {
        task_id,
        description: workspace.target_change.description.clone(),
    };
    let mut event_effects = Vec::new();

    for handler in handlers
        .iter()
        .filter(|handler| handler.on == RepoEvent::PullRequestPrepare)
    {
        if !pull_request_prepare_query_matches(&handler.when, &subject) {
            continue;
        }

        if matches!(handler.run, RepoEventHandlerRun::PrependTaskId) {
            let Some(task_id) = subject.task_id else {
                continue;
            };
            let Some(outcome) = prepend_task_id_to_description(&subject.description, task_id)
            else {
                continue;
            };
            subject.description = outcome.description;
            event_effects.push(PullRequestEventEffect {
                event: handler.on,
                handler_id: handler.id.clone(),
                kind: if outcome.changed {
                    PullRequestEventEffectKind::UpdatedTitle {
                        title: outcome.title,
                    }
                } else {
                    PullRequestEventEffectKind::TitleAlready {
                        title: outcome.title,
                    }
                },
            });
        }
    }

    PullRequestPrepareReport {
        changed: subject.description != workspace.target_change.description,
        description: subject.description,
        event_effects,
    }
}

pub async fn publish_pull_request(
    context: &RepositoryContext,
    plan: PullRequestPlan,
    bookmark_update: BookmarkUpdate,
    push: PushOutcome,
    options: PullRequestPublishOptions,
    github: &dyn GitHubClient,
) -> Result<PullRequestReport, WorkflowError> {
    let existing = plan.existing_pull_request.clone();
    let (action, mut pull_request) = if let Some(existing) = existing {
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
            body: non_empty_pull_request_body(&plan.body),
            head: plan.head.clone(),
            base: plan.base.clone(),
            draft: plan.draft,
        };
        let pull_request = github
            .create_pull_request(&context.origin.github, request)
            .await?;

        (PullRequestAction::Created, pull_request)
    };

    if action == PullRequestAction::Updated && pull_request.draft != plan.draft {
        pull_request = if plan.draft {
            github
                .convert_pull_request_to_draft(&context.origin.github, pull_request.number)
                .await?
        } else {
            github
                .mark_pull_request_ready(&context.origin.github, pull_request.number)
                .await?
        };
    }

    let event_handlers = if options.event_handlers {
        context
            .config
            .repo
            .event_handlers_for(&context.origin.github)
    } else {
        Vec::new()
    };
    let existing_labels = if action == PullRequestAction::Updated
        && (!event_handlers.is_empty() || !plan.labels.is_empty())
    {
        github
            .pull_request_labels(&context.origin.github, pull_request.number)
            .await?
    } else {
        Vec::new()
    };
    let mut subject = PullRequestEventSubject::new(
        action,
        pull_request.clone(),
        plan.task_id.clone(),
        plan.reviewers.clone(),
        merge_label_sets(existing_labels.clone(), plan.labels.clone()),
    );

    let mut applied_labels = Vec::new();
    let cli_labels = if action == PullRequestAction::Updated {
        missing_labels(&existing_labels, plan.labels.clone())
    } else {
        plan.labels.clone()
    };
    let _ = apply_label_batch(
        context,
        github,
        subject.pull_request.number,
        cli_labels,
        &mut applied_labels,
    )
    .await?;

    let event_effects = run_pull_request_event_handlers(
        context,
        github,
        &event_handlers,
        &mut subject,
        &mut applied_labels,
    )
    .await?;

    let labels = (!applied_labels.is_empty()).then_some(LabelApplyResult {
        labels: applied_labels,
    });

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
        base_pull_request: plan.base_pull_request,
        head: plan.head,
        labels,
        reviewers,
        event_effects,
    })
}

/// Updates only non-code pull-request metadata when the branch already matches GitHub.
pub async fn publish_pull_request_metadata_only(
    context: &RepositoryContext,
    plan: PullRequestPlan,
    bookmark_update: BookmarkUpdate,
    push: PushOutcome,
    github: &dyn GitHubClient,
) -> Result<PullRequestReport, WorkflowError> {
    let mut pull_request = plan
        .existing_pull_request
        .clone()
        .ok_or(WorkflowError::MissingPullRequest)?;

    if pull_request.draft != plan.draft {
        pull_request = if plan.draft {
            github
                .convert_pull_request_to_draft(&context.origin.github, pull_request.number)
                .await?
        } else {
            github
                .mark_pull_request_ready(&context.origin.github, pull_request.number)
                .await?
        };
    }

    let existing_labels = if plan.labels.is_empty() {
        Vec::new()
    } else {
        github
            .pull_request_labels(&context.origin.github, pull_request.number)
            .await?
    };
    let mut applied_labels = Vec::new();
    let cli_labels = missing_labels(&existing_labels, plan.labels.clone());
    let _ = apply_label_batch(
        context,
        github,
        pull_request.number,
        cli_labels,
        &mut applied_labels,
    )
    .await?;
    let labels = (!applied_labels.is_empty()).then_some(LabelApplyResult {
        labels: applied_labels,
    });

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
        action: PullRequestAction::Updated,
        pull_request,
        base: plan.base,
        base_pull_request: plan.base_pull_request,
        head: plan.head,
        labels,
        reviewers,
        event_effects: Vec::new(),
    })
}

struct PullRequestPrepareSubject<'a> {
    task_id: Option<&'a str>,
    description: String,
}

struct PreparedCommitDescription {
    description: String,
    title: String,
    changed: bool,
}

fn prepend_task_id_to_description(
    description: &str,
    task_id: &str,
) -> Option<PreparedCommitDescription> {
    let line = first_non_empty_line_range(description)?;
    let original_title = description[line.title.clone()].trim();
    let title = canonical_task_title(task_id, original_title);
    let changed = title != original_title;
    let mut updated = String::with_capacity(description.len() + title.len());
    updated.push_str(&description[..line.title.start]);
    updated.push_str(&title);
    updated.push_str(&description[line.title.end..]);

    Some(PreparedCommitDescription {
        description: updated,
        title,
        changed,
    })
}

fn canonical_task_title(task_id: &str, title: &str) -> String {
    let title = title.trim();
    if let Some(title) = strip_task_prefix(title, task_id) {
        let title = title.trim();
        return if title.is_empty() {
            task_id.to_owned()
        } else {
            format!("{task_id}: {title}")
        };
    }

    if title_has_task_reference(title) {
        title.to_owned()
    } else if title.is_empty() {
        task_id.to_owned()
    } else {
        format!("{task_id}: {title}")
    }
}

struct TitleLineRange {
    title: std::ops::Range<usize>,
    after_line: usize,
}

fn first_non_empty_line_range(description: &str) -> Option<TitleLineRange> {
    let mut offset = 0;
    for line in description.split_inclusive('\n') {
        let line_without_ending = line.trim_end_matches(['\r', '\n']);
        if !line_without_ending.trim().is_empty() {
            let leading = line_without_ending.len() - line_without_ending.trim_start().len();
            let trailing = line_without_ending.len() - line_without_ending.trim_end().len();
            return Some(TitleLineRange {
                title: offset + leading..offset + line_without_ending.len() - trailing,
                after_line: offset + line.len(),
            });
        }
        offset += line.len();
    }

    None
}

fn strip_task_prefix<'a>(title: &'a str, task_id: &str) -> Option<&'a str> {
    strip_bracketed_task_prefix(title, task_id).or_else(|| strip_bare_task_prefix(title, task_id))
}

fn strip_bracketed_task_prefix<'a>(title: &'a str, task_id: &str) -> Option<&'a str> {
    let rest = title.strip_prefix('[')?;
    let (candidate, rest) = rest.split_once(']')?;
    is_task_prefix(candidate, task_id).then(|| trim_task_prefix_separator(rest))
}

fn strip_bare_task_prefix<'a>(title: &'a str, task_id: &str) -> Option<&'a str> {
    let prefix_len = title
        .char_indices()
        .take_while(|(_, character)| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
        })
        .map(|(index, character)| index + character.len_utf8())
        .last()?;
    let candidate = &title[..prefix_len];
    let rest = &title[prefix_len..];
    if rest.is_empty() && !candidate.eq_ignore_ascii_case(task_id) {
        return None;
    }
    is_task_prefix(candidate, task_id).then(|| trim_task_prefix_separator(rest))
}

fn is_task_prefix(candidate: &str, task_id: &str) -> bool {
    candidate.eq_ignore_ascii_case(task_id)
}

/// Returns a leading work item identifier from a title, such as `ABC-123`.
pub fn work_id_from_title_prefix(title: &str) -> Option<String> {
    let title = title.trim();
    let bracketed = title
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map(|(candidate, _)| candidate.trim());
    if let Some(candidate) = bracketed.filter(|candidate| is_task_reference(candidate)) {
        return Some(candidate.to_owned());
    }

    let prefix_len = title
        .char_indices()
        .take_while(|(_, character)| is_task_token_character(*character))
        .map(|(index, character)| index + character.len_utf8())
        .last()?;
    let candidate = &title[..prefix_len];
    is_task_reference(candidate).then(|| candidate.to_owned())
}

/// Returns related work IDs from a commit description, task context, and explicit fixes.
pub fn pull_request_work_ids_from_description(
    description: &str,
    task_id: Option<&str>,
    fixes: &[String],
) -> Vec<String> {
    let title = pull_request_description_from_text(description)
        .map(|(title, _)| title)
        .unwrap_or_default();
    pull_request_work_ids(&title, task_id, fixes)
}

/// Returns the related work IDs to preserve for a pull request node.
pub fn pull_request_work_ids(title: &str, task_id: Option<&str>, fixes: &[String]) -> Vec<String> {
    let mut work_ids = Vec::new();
    if let Some(work_id) = work_id_from_title_prefix(title) {
        push_unique_work_id(&mut work_ids, work_id);
    } else if let Some(task_id) = task_id {
        push_unique_work_id(&mut work_ids, task_id.to_owned());
    }
    for work_id in fixes {
        push_unique_work_id(&mut work_ids, work_id.clone());
    }
    work_ids
}

fn push_unique_work_id(work_ids: &mut Vec<String>, work_id: String) {
    let work_id = work_id.trim();
    if !work_id.is_empty() && !work_ids.iter().any(|existing| existing == work_id) {
        work_ids.push(work_id.to_owned());
    }
}

fn title_has_task_reference(title: &str) -> bool {
    let mut token_start = None;
    for (index, character) in title.char_indices() {
        if is_task_token_character(character) {
            token_start.get_or_insert(index);
            continue;
        }

        if let Some(start) = token_start.take() {
            if is_task_reference(&title[start..index]) {
                return true;
            }
        }
    }

    token_start.is_some_and(|start| is_task_reference(&title[start..]))
}

fn is_task_token_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
}

fn is_task_reference(candidate: &str) -> bool {
    candidate.contains('-')
        && candidate
            .chars()
            .any(|character| character.is_ascii_digit())
        && candidate.chars().all(|character| {
            character.is_ascii_uppercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
}

fn trim_task_prefix_separator(value: &str) -> &str {
    let value = value.trim_start();
    value
        .strip_prefix(':')
        .or_else(|| value.strip_prefix('-'))
        .unwrap_or(value)
        .trim_start()
}

fn pull_request_prepare_query_matches(
    query: &PullRequestEventQuery,
    subject: &PullRequestPrepareSubject<'_>,
) -> bool {
    query.terms.iter().all(|term| {
        let matched = match &term.predicate {
            PullRequestEventPredicate::HasTask => subject.task_id.is_some(),
            PullRequestEventPredicate::Draft
            | PullRequestEventPredicate::Ready
            | PullRequestEventPredicate::HasReviewers
            | PullRequestEventPredicate::Label(_) => false,
        };
        matched != term.negated
    })
}

struct PullRequestEventSubject {
    event: RepoEvent,
    pull_request: PullRequestRecord,
    draft: bool,
    task_id: Option<String>,
    reviewers: ReviewerSelection,
    labels: Vec<String>,
}

impl PullRequestEventSubject {
    fn new(
        action: PullRequestAction,
        pull_request: PullRequestRecord,
        task_id: Option<String>,
        reviewers: ReviewerSelection,
        labels: Vec<String>,
    ) -> Self {
        let event = match action {
            PullRequestAction::Created => RepoEvent::PullRequestCreated,
            PullRequestAction::Updated => RepoEvent::PullRequestUpdated,
        };
        let draft = pull_request.draft;

        Self {
            event,
            pull_request,
            draft,
            task_id,
            reviewers,
            labels,
        }
    }
}

async fn run_pull_request_event_handlers(
    context: &RepositoryContext,
    github: &dyn GitHubClient,
    handlers: &[RepoEventHandler],
    subject: &mut PullRequestEventSubject,
    applied_labels: &mut Vec<String>,
) -> Result<Vec<PullRequestEventEffect>, WorkflowError> {
    let mut effects = Vec::new();
    for handler in handlers {
        if handler.on != subject.event || !pull_request_event_query_matches(&handler.when, subject)
        {
            continue;
        }

        match &handler.run {
            RepoEventHandlerRun::AddLabels { labels } => {
                let added =
                    add_missing_labels(context, github, subject, labels.clone(), applied_labels)
                        .await?;
                let kind = if added.is_empty() {
                    PullRequestEventEffectKind::LabelsAlreadyPresent {
                        labels: labels.clone(),
                    }
                } else {
                    PullRequestEventEffectKind::AddLabels { labels: added }
                };
                effects.push(PullRequestEventEffect {
                    event: handler.on,
                    handler_id: handler.id.clone(),
                    kind,
                });
            }
            RepoEventHandlerRun::OpenPullRequest => {
                effects.push(PullRequestEventEffect {
                    event: handler.on,
                    handler_id: handler.id.clone(),
                    kind: PullRequestEventEffectKind::OpenPullRequest {
                        url: pull_request_url(
                            &context.origin.github.https_url(),
                            &subject.pull_request,
                        ),
                    },
                });
            }
            RepoEventHandlerRun::PrependTaskId => {}
        }
    }

    Ok(effects)
}

async fn add_missing_labels(
    context: &RepositoryContext,
    github: &dyn GitHubClient,
    subject: &mut PullRequestEventSubject,
    labels: Vec<String>,
    applied_labels: &mut Vec<String>,
) -> Result<Vec<String>, WorkflowError> {
    let labels = missing_labels(&subject.labels, labels);
    let added = apply_label_batch(
        context,
        github,
        subject.pull_request.number,
        labels,
        applied_labels,
    )
    .await?;
    subject.labels = merge_label_sets(subject.labels.clone(), added.clone());

    Ok(added)
}

async fn apply_label_batch(
    context: &RepositoryContext,
    github: &dyn GitHubClient,
    pull_request_number: u64,
    labels: Vec<String>,
    applied_labels: &mut Vec<String>,
) -> Result<Vec<String>, WorkflowError> {
    if labels.is_empty() {
        return Ok(Vec::new());
    }

    let result = github
        .add_labels(&context.origin.github, pull_request_number, labels)
        .await?;
    let labels = result.labels;
    *applied_labels = merge_label_sets(applied_labels.clone(), labels.clone());

    Ok(labels)
}

fn pull_request_event_query_matches(
    query: &PullRequestEventQuery,
    subject: &PullRequestEventSubject,
) -> bool {
    query.terms.iter().all(|term| {
        let matched = match &term.predicate {
            PullRequestEventPredicate::Draft => subject.draft,
            PullRequestEventPredicate::Ready => !subject.draft,
            PullRequestEventPredicate::HasReviewers => !subject.reviewers.is_empty(),
            PullRequestEventPredicate::HasTask => subject.task_id.is_some(),
            PullRequestEventPredicate::Label(label) => subject.labels.contains(label),
        };
        matched != term.negated
    })
}

fn missing_labels(existing: &[String], labels: Vec<String>) -> Vec<String> {
    labels
        .into_iter()
        .filter(|label| !existing.contains(label))
        .collect()
}

fn merge_label_sets(left: Vec<String>, right: Vec<String>) -> Vec<String> {
    let mut labels = left;
    for label in right {
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels
}

fn pull_request_url(repository_url: &str, pull_request: &PullRequestRecord) -> String {
    pull_request
        .html_url
        .clone()
        .unwrap_or_else(|| format!("{repository_url}/pull/{}", pull_request.number))
}

fn non_empty_pull_request_body(body: &str) -> Option<String> {
    (!body.is_empty()).then(|| body.to_owned())
}

fn pull_request_description(workspace: &WorkspaceFacts) -> Result<(String, String), WorkflowError> {
    if workspace.target_change.is_empty {
        return Err(WorkflowError::EmptyPullRequestChange);
    }

    pull_request_description_from_text(&workspace.target_change.description)
}

/// Splits a commit description into a PR title and body without duplicating the title.
pub(super) fn pull_request_description_from_text(
    description: &str,
) -> Result<(String, String), WorkflowError> {
    let Some(line) = first_non_empty_line_range(description) else {
        return Err(WorkflowError::MissingPullRequestDescription);
    };

    let title = description[line.title].trim().to_owned();
    let body = trim_body_blank_lines(&description[line.after_line..]).to_owned();

    Ok((title, body))
}

fn trim_body_blank_lines(value: &str) -> &str {
    let mut offset = 0;
    let mut first_non_empty = None;
    let mut last_non_empty_end = 0;

    for line in value.split_inclusive('\n') {
        let line_without_ending = line.trim_end_matches(['\r', '\n']);
        if !line_without_ending.trim().is_empty() {
            first_non_empty.get_or_insert(offset);
            last_non_empty_end = offset + line_without_ending.len();
        }
        offset += line.len();
    }

    first_non_empty.map_or("", |start| &value[start..last_non_empty_end])
}
