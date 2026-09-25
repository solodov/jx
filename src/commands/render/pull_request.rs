use super::*;

pub(in crate::commands) fn render_pull_request(report: &PullRequestReport) -> String {
    format!(
        "{} {}\n",
        pull_request_action(report.action),
        linked_pull_request_text(&report.repository.github_url, &report.pull_request)
    )
}

/// Wraps shared commit content from the publish plan in PR headers and workflow metadata.
pub(in crate::commands) fn render_pull_request_preview(
    plan: &PullRequestPlan,
    current_dir: &Path,
    prepare_effects: &[PullRequestEventEffect],
    output: OutputMode,
) -> Result<String, JjError> {
    let mut header = vec![pull_request_preview_header(plan)];
    header.extend(
        prepare_effects
            .iter()
            .filter_map(pull_request_prepare_event_summary),
    );
    let header = header
        .into_iter()
        .map(|line| style_log_line(&line, output.color))
        .collect::<Vec<_>>()
        .join("\n");

    let mut description = plan.title.clone();
    if !plan.body.is_empty() {
        description.push_str("\n\n");
        description.push_str(&plan.body);
    }
    let mut blocks = vec![header];
    let content = render_commit_content(&description, &plan.change_lines, current_dir, output)?;
    if !content.is_empty() {
        blocks.push(content);
    }

    let mut metadata = vec![pull_request_reviewer_preview(plan)];
    if !plan.labels.is_empty() {
        metadata.push(format!("Labels: {}", plan.labels.join(", ")));
    }
    blocks.push(metadata.join("\n"));

    Ok(format!("{}\n", blocks.join("\n\n")))
}

/// Shows the effective reviewers and distinguishes preserved requests from explicit draft additions.
fn pull_request_reviewer_preview(plan: &PullRequestPlan) -> String {
    let reviewers = plan.effective_reviewers();
    let mut names = reviewers.users.clone();
    names.extend(reviewers.teams.iter().map(|team| format!("{team} (team)")));
    let summary = if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    };
    let note = if plan.reviewers.is_empty() && !reviewers.is_empty() {
        " (unchanged)"
    } else if plan.draft && !plan.reviewers.is_empty() {
        " (explicit draft request)"
    } else if plan.draft {
        " (draft)"
    } else {
        ""
    };
    format!("Reviewers: {summary}{note}")
}

const LOG_LINE_STYLE: &str = "\x1b[2m\x1b[38;5;244m";
const RESET_STYLE: &str = "\x1b[0m";

pub(in crate::commands) fn style_log_line(line: &str, color: bool) -> String {
    if color {
        format!("{LOG_LINE_STYLE}{line}{RESET_STYLE}")
    } else {
        line.to_owned()
    }
}

fn pull_request_preview_header(plan: &PullRequestPlan) -> String {
    let head = linked_bookmark_text(&plan.repository.github_url, &plan.bookmark.branch);
    let base = pull_request_preview_base(plan);
    match &plan.existing_pull_request {
        Some(existing) => {
            let verb = match (existing.draft, plan.draft) {
                (true, false) => "Updating and marking ready",
                (false, true) => "Updating and marking draft",
                (_, true) => "Updating draft",
                _ => "Updating",
            };
            format!(
                "{verb} {}: {head} → {base}",
                linked_pull_request_text(&plan.repository.github_url, existing)
            )
        }
        None => {
            let verb = if plan.draft {
                "Creating draft"
            } else {
                "Creating"
            };
            format!("{verb}: {head} → {base}")
        }
    }
}

fn pull_request_preview_base(plan: &PullRequestPlan) -> String {
    plan.base_pull_request.as_ref().map_or_else(
        || {
            osc8_link(
                &branch_url(&plan.repository.github_url, &plan.base),
                &plan.base,
            )
        },
        |pull_request| linked_pull_request_text(&plan.repository.github_url, pull_request),
    )
}

fn pull_request_prepare_event_summary(effect: &PullRequestEventEffect) -> Option<String> {
    match &effect.kind {
        PullRequestEventEffectKind::UpdatedTitle { .. } => Some(format!(
            "Event[{}]: Added task ID to the title",
            pull_request_event_display_name(effect)
        )),
        PullRequestEventEffectKind::AddLabels { .. }
        | PullRequestEventEffectKind::LabelsAlreadyPresent { .. }
        | PullRequestEventEffectKind::OpenPullRequest { .. }
        | PullRequestEventEffectKind::TitleAlready { .. } => None,
    }
}

pub(in crate::commands) fn pull_request_event_display_name(
    effect: &PullRequestEventEffect,
) -> &str {
    effect
        .handler_id
        .as_deref()
        .unwrap_or_else(|| effect.event.label())
}

pub(in crate::commands) fn pull_request_event_effect_is_default_visible(
    effect: &PullRequestEventEffect,
) -> bool {
    matches!(
        &effect.kind,
        PullRequestEventEffectKind::AddLabels { .. }
            | PullRequestEventEffectKind::OpenPullRequest { .. }
            | PullRequestEventEffectKind::UpdatedTitle { .. }
    )
}

/// Builds the final confirmation prompt from planned create/update and draft state.
pub(in crate::commands) fn pull_request_confirmation_prompt(plan: &PullRequestPlan) -> String {
    match &plan.existing_pull_request {
        Some(existing) => match (existing.draft, plan.draft) {
            (true, false) => "Update and mark ready?".to_owned(),
            (false, true) => "Update and mark draft?".to_owned(),
            (_, true) => "Update draft?".to_owned(),
            _ => "Update?".to_owned(),
        },
        None if plan.draft => "Create draft?".to_owned(),
        None => "Create?".to_owned(),
    }
}

/// Builds the confirmation prompt for creating an otherwise missing push bookmark.
pub(in crate::commands) fn push_confirmation_prompt(plan: &PushPlan) -> String {
    format!(
        "Create bookmark `{}` at {} and push?",
        plan.bookmark.branch, plan.target_short_commit_id
    )
}

/// Builds the confirmation prompt before forgetting and deleting a managed workspace.
pub(in crate::commands) fn workspace_remove_confirmation_prompt(
    workspace: &WorkspaceEntry,
    display_root: &str,
) -> String {
    format!("Delete workspace `{}` at {display_root}?", workspace.name)
}
