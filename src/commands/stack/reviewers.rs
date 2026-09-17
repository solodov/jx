use super::*;

/// Applies shared selection only to ready PRs, allowing explicit additions to one targeted draft.
pub(super) fn select_publish_reviewers(
    plans: &mut [PullRequestPlan],
    intent_positions: &BTreeSet<usize>,
    request: &StackPublishRequest,
    selector: &dyn ReviewerSelector,
    span: &mut PerfSpan,
) -> Result<(), ReviewerSelectionError> {
    let ready_positions = intent_positions
        .iter()
        .copied()
        .filter(|position| !plans[*position].draft)
        .collect::<BTreeSet<_>>();
    let explicit_draft = !request.apply_to_stack
        && !request.revisions.is_empty()
        && !request.reviewers.is_empty()
        && plans.len() == 1
        && plans[0].draft
        && intent_positions.contains(&0);

    if !explicit_draft && plans.iter().any(|plan| plan.draft) && io::stderr().is_terminal() {
        if ready_positions.is_empty() {
            eprintln!("No ready PRs in reviewer scope; reviewer selection skipped. Draft reviewers unchanged.");
        } else {
            eprintln!("Reviewers apply to ready PRs only; draft reviewers remain unchanged.");
        }
    }

    let candidates = reviewer_candidates(plans, &ready_positions);
    let preselected = preselected_reviewers(plans, &ready_positions, &request.reviewers);
    let selected = span.measure(
        "reviewer_selection",
        [
            perf_attr("candidate_count", candidates.len()),
            perf_attr("reviewer_arg_count", request.reviewers.len()),
            perf_attr("preselected_reviewer_count", preselected.len()),
            perf_attr("ready_pr_count", ready_positions.len()),
            perf_attr("explicit_draft", explicit_draft),
        ],
        || {
            if ready_positions.is_empty() {
                Ok(ReviewerSelection::default())
            } else {
                selector.select_reviewers(&candidates, &preselected)
            }
        },
    )?;

    for (position, plan) in plans.iter_mut().enumerate() {
        plan.reviewers = if ready_positions.contains(&position) {
            selected.clone()
        } else if explicit_draft {
            let explicit = selection_from_targets(&request.reviewers);
            let existing = plan
                .existing_pull_request
                .as_ref()
                .map(|pull_request| pull_request.reviewers.clone())
                .unwrap_or_default();
            ReviewerSelection::new(
                existing.users.into_iter().chain(explicit.users),
                existing.teams.into_iter().chain(explicit.teams),
            )
        } else {
            ReviewerSelection::default()
        };
    }
    Ok(())
}

fn reviewer_candidates(
    plans: &[PullRequestPlan],
    positions: &BTreeSet<usize>,
) -> Vec<ReviewerCandidate> {
    let mut candidates: Vec<ReviewerCandidate> = Vec::new();
    for candidate in plans
        .iter()
        .enumerate()
        .filter(|(position, _)| positions.contains(position))
        .flat_map(|(_, plan)| plan.reviewer_candidates.iter().cloned())
    {
        if let Some(existing) = candidates
            .iter_mut()
            .find(|existing| existing.target.matches_identity(&candidate.target))
        {
            for reason in candidate.reasons {
                if !existing.reasons.contains(&reason) {
                    existing.reasons.push(reason);
                }
            }
        } else {
            candidates.push(candidate);
        }
    }
    candidates
}

fn preselected_reviewers(
    plans: &[PullRequestPlan],
    positions: &BTreeSet<usize>,
    cli_reviewers: &[ReviewerTarget],
) -> Vec<ReviewerTarget> {
    let mut reviewers = Vec::new();
    for reviewer in cli_reviewers {
        push_reviewer_target(&mut reviewers, reviewer.clone());
    }
    for plan in plans
        .iter()
        .enumerate()
        .filter(|(position, _)| positions.contains(position))
        .map(|(_, plan)| plan)
    {
        if let Some(existing) = &plan.existing_pull_request {
            for user in &existing.reviewers.users {
                push_reviewer_target(&mut reviewers, ReviewerTarget::user(user.clone()));
            }
            for team in &existing.reviewers.teams {
                push_reviewer_target(
                    &mut reviewers,
                    ReviewerTarget::team(team.clone(), team.clone()),
                );
            }
        }
        for candidate in &plan.reviewer_candidates {
            if reviewer_candidate_keeps_existing_review_selection(candidate) {
                push_reviewer_target(&mut reviewers, candidate.target.clone());
            }
        }
    }
    reviewers
}

/// Returns whether prior PR activity should keep a reviewer checked after GitHub clears a request.
fn reviewer_candidate_keeps_existing_review_selection(candidate: &ReviewerCandidate) -> bool {
    candidate.reasons.iter().any(|reason| {
        matches!(
            reason.as_str(),
            "already requested" | "already approved" | "commented" | "comments addressed"
        )
    })
}

fn push_reviewer_target(reviewers: &mut Vec<ReviewerTarget>, reviewer: ReviewerTarget) {
    if !reviewers
        .iter()
        .any(|existing| existing.matches_identity(&reviewer))
    {
        reviewers.push(reviewer);
    }
}
