use super::*;

pub(in crate::commands) fn render_pull_request(report: &PullRequestReport) -> String {
    format!(
        "{} {}\n",
        pull_request_action(report.action),
        linked_pull_request_text(&report.repository.github_url, &report.pull_request)
    )
}

/// Renders the approval-focused PR preview before any publishing mutation.
#[cfg(test)]
pub(in crate::commands) fn render_pull_request_preview(
    plan: &PullRequestPlan,
    status: &WorkspaceStatus,
    prepare_effects: &[PullRequestEventEffect],
) -> String {
    render_pull_request_preview_with_style(plan, status, prepare_effects, false)
}

/// Renders the PR preview with optional log-line styling for interactive terminals.
pub(in crate::commands) fn render_pull_request_preview_with_style(
    plan: &PullRequestPlan,
    status: &WorkspaceStatus,
    prepare_effects: &[PullRequestEventEffect],
    color: bool,
) -> String {
    render_pull_request_preview_with_style_for_width(
        plan,
        status,
        prepare_effects,
        color,
        termimad::terminal_size().0.into(),
    )
}

#[cfg(test)]
pub(in crate::commands) fn render_pull_request_preview_for_width(
    plan: &PullRequestPlan,
    status: &WorkspaceStatus,
    prepare_effects: &[PullRequestEventEffect],
    terminal_width: usize,
) -> String {
    render_pull_request_preview_with_style_for_width(
        plan,
        status,
        prepare_effects,
        false,
        terminal_width,
    )
}

fn render_pull_request_preview_with_style_for_width(
    plan: &PullRequestPlan,
    status: &WorkspaceStatus,
    prepare_effects: &[PullRequestEventEffect],
    color: bool,
    terminal_width: usize,
) -> String {
    let mut header = vec![pull_request_preview_header(plan)];
    header.extend(
        prepare_effects
            .iter()
            .filter_map(pull_request_prepare_event_summary),
    );
    let header = header
        .into_iter()
        .map(|line| style_log_line(&line, color))
        .collect::<Vec<_>>()
        .join("\n");

    let content_width = terminal_width.saturating_sub(PREVIEW_CONTENT_INDENT.len());
    let mut blocks = vec![header];
    blocks.push(indent_non_empty_lines(
        &render_pull_request_description_preview(plan, content_width),
    ));

    let mut change_lines = status.change_lines.clone();
    change_lines.extend(status.extra_lines.clone());
    if !change_lines.is_empty() {
        blocks.push(indent_non_empty_lines(&change_lines.join("\n")));
    }

    let mut metadata = Vec::new();
    if !plan.labels.is_empty() {
        metadata.push(format!("Labels: {}", plan.labels.join(", ")));
    }
    if !metadata.is_empty() {
        blocks.push(metadata.join("\n"));
    }

    format!("{}\n", blocks.join("\n\n"))
}

const PREVIEW_CONTENT_INDENT: &str = "  ";
const LOG_LINE_STYLE: &str = "\x1b[2m\x1b[38;5;244m";
const RESET_STYLE: &str = "\x1b[0m";

pub(in crate::commands) fn style_log_line(line: &str, color: bool) -> String {
    if color {
        format!("{LOG_LINE_STYLE}{line}{RESET_STYLE}")
    } else {
        line.to_owned()
    }
}

fn indent_non_empty_lines(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{PREVIEW_CONTENT_INDENT}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn pull_request_preview_header(plan: &PullRequestPlan) -> String {
    let head = linked_bookmark_text(&plan.repository.github_url, &plan.bookmark.branch);
    let base = pull_request_preview_base(plan);
    match &plan.existing_pull_request {
        Some(existing) => {
            let verb = if existing.draft {
                "Updating draft"
            } else {
                "Updating"
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

fn render_pull_request_description_preview(plan: &PullRequestPlan, width: usize) -> String {
    let mut description = plan.title.clone();
    if !plan.body.is_empty() {
        description.push_str("\n\n");
        description.push_str(&plan.body);
    }
    render_status_description(&description, width)
        .trim_end()
        .to_owned()
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
    let (verb, draft) = match &plan.existing_pull_request {
        Some(existing) => ("Update", existing.draft),
        None => ("Create", plan.draft),
    };
    if draft {
        format!("{verb} draft?")
    } else {
        format!("{verb}?")
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
) -> String {
    format!(
        "Delete workspace `{}` at {}?",
        workspace.name,
        workspace.root.display()
    )
}
