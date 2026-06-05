use super::*;
use crate::domain::ReviewRequestState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct ReviewRequestsView {
    pub(in crate::commands) viewer: String,
    pub(in crate::commands) repositories: Vec<ReviewRequestRepositoryView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct ReviewRequestRepositoryView {
    pub(in crate::commands) repository: GitHubRepository,
    pub(in crate::commands) layout_key: Option<String>,
    pub(in crate::commands) display_root: Option<String>,
    pub(in crate::commands) rows: Vec<ReviewRequestRowView>,
    pub(in crate::commands) external: bool,
    pub(in crate::commands) review_wait_threshold_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct ReviewRequestRowView {
    pub(in crate::commands) status: PullRequestStatusRecord,
    pub(in crate::commands) state: ReviewRequestState,
}

pub(in crate::commands) fn render_review_requests(
    view: &ReviewRequestsView,
    color: bool,
    terminal_width: Option<usize>,
    display_names: &BTreeMap<String, String>,
) -> String {
    let mut output = String::new();
    let pull_request_count = view
        .repositories
        .iter()
        .map(|repository| repository.rows.len())
        .sum::<usize>();
    output.push_str(&format!(
        "Review requests for {viewer}: {pull_request_count} pull request{} across {repository_count} repositor{}\n",
        if pull_request_count == 1 { "" } else { "s" },
        if view.repositories.len() == 1 { "y" } else { "ies" },
        viewer = pull_request_user_display_name(&view.viewer, display_names),
        repository_count = view.repositories.len(),
    ));
    if view.repositories.is_empty() {
        return output;
    }
    output.push('\n');

    for (index, repository) in view.repositories.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&review_repository_header(repository, color));
        output.push('\n');
        output.push_str(&format!(
            "  {pr:<pr_width$}  Chk  Req  {:<lag_width$}  Title\n",
            "Lag",
            pr = "PR",
            pr_width = PULL_REQUEST_STATUS_PR_WIDTH,
            lag_width = REVIEW_LAG_WIDTH,
        ));
        for row in &repository.rows {
            output.push_str(&review_request_row(
                repository,
                row,
                &view.viewer,
                color,
                terminal_width,
                display_names,
            ));
            output.push('\n');
        }
    }

    output.push('\n');
    output.push_str(&review_requests_legend(color));
    output
}

fn review_repository_header(repository: &ReviewRequestRepositoryView, color: bool) -> String {
    let slug = repository.repository.slug();
    let label = repository.layout_key.as_deref().unwrap_or(&slug);
    let link = osc8_link(&repository.repository.https_url(), label);
    let suffix = repository
        .display_root
        .as_ref()
        .map(|root| format!("  {root}"))
        .unwrap_or_else(|| format!("  {slug}"));
    if repository.external && color {
        format!("{DIM_STYLE}{link}{suffix}{RESET_STYLE}")
    } else if color {
        format!("\x1b[1m{link}{RESET_STYLE}{suffix}")
    } else {
        format!("{label}{suffix}")
    }
}

fn review_request_row(
    repository: &ReviewRequestRepositoryView,
    row: &ReviewRequestRowView,
    viewer: &str,
    color: bool,
    terminal_width: Option<usize>,
    display_names: &BTreeMap<String, String>,
) -> String {
    let row_color = color && !repository.external;
    let active_cell_color = row_color && !row.status.draft;
    let pr = review_request_pr_cell(&repository.repository, &row.status, row_color);
    let check = pull_request_check_symbol(Some(&row.status), row.status.merged, active_cell_color);
    let lag = pull_request_viewer_review_lag(
        &row.status,
        viewer,
        repository.review_wait_threshold_seconds,
        review_request_state_waits_on_viewer(row.state),
    );
    let state = review_request_state_cell(row.state, active_cell_color, lag.over_threshold);
    let restore_style = if repository.external && color {
        DIM_STYLE
    } else if row.status.draft && color {
        DRAFT_ROW_STYLE
    } else {
        ""
    };
    let lag = render_review_lag_cell(&lag, color, restore_style, false, row.status.draft);
    let title = review_request_title(&row.status, viewer, row_color, display_names);
    let pr_padding = " ".repeat(
        PULL_REQUEST_STATUS_PR_WIDTH.saturating_sub(format!("#{}", row.status.number).len()),
    );
    let line = format!(
        "  {pr}{pr_padding}  {check}    {state}    {lag}  {title}",
        pr = pr,
        pr_padding = pr_padding,
        check = check,
        state = state,
        lag = lag,
        title = title,
    );
    let line = ellipsize_rendered_line(&line, terminal_width);
    if repository.external && color {
        format!("{DIM_STYLE}{line}{RESET_STYLE}")
    } else if row.status.draft && color {
        format!("{DRAFT_ROW_STYLE}{line}{RESET_STYLE}")
    } else {
        line
    }
}

fn review_request_pr_cell(
    repository: &GitHubRepository,
    status: &PullRequestStatusRecord,
    color: bool,
) -> String {
    let label = format!("#{}", status.number);
    let url = status
        .url
        .as_deref()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}/pull/{}", repository.https_url(), status.number));
    if color {
        osc8_link(&url, &label)
    } else {
        label
    }
}

fn review_request_state_waits_on_viewer(state: ReviewRequestState) -> bool {
    matches!(
        state,
        ReviewRequestState::New | ReviewRequestState::Answered | ReviewRequestState::Again
    )
}

fn review_request_state_cell(
    state: ReviewRequestState,
    color: bool,
    review_lag_over_threshold: bool,
) -> String {
    let (symbol, style) = match state {
        ReviewRequestState::New | ReviewRequestState::Answered | ReviewRequestState::Again => (
            "◷",
            if review_lag_over_threshold {
                PullRequestSymbolStyle::Bad
            } else {
                PullRequestSymbolStyle::Info
            },
        ),
        ReviewRequestState::Commented => ("!", PullRequestSymbolStyle::Warn),
        ReviewRequestState::Approved => ("✓", PullRequestSymbolStyle::Good),
    };
    styled_pull_request_symbol(symbol, style, color)
}

fn review_request_title(
    status: &PullRequestStatusRecord,
    viewer: &str,
    color: bool,
    display_names: &BTreeMap<String, String>,
) -> String {
    let marker = if status.draft { "◌" } else { "◯" };
    let title = ellipsize_pull_request_title(&status.title);
    let mut parts = vec![format!("{marker} {title}")];
    let labels = pull_request_label_chips(&status.labels, color, status.draft);
    if !labels.is_empty() {
        parts.push(labels.join(pull_request_label_separator(color)));
    }
    let mut reviewer_tokens = Vec::new();
    reviewer_tokens.extend(pull_request_reviewer_activity_tokens(
        status,
        viewer,
        color && !status.draft,
        display_names,
    ));
    if !reviewer_tokens.is_empty() {
        parts.push(reviewer_tokens.join(", "));
    }
    parts.join(" ")
}

fn review_requests_legend(color: bool) -> String {
    let lines = [
        "Legend:",
        "  Title: ◯ ready, ◌ draft; labels/reviewer activity follow title",
        "  Chk: ✓ passing, ✗ failing, ◷ pending, — none/unknown",
        "  Req: ◷ requested, ! commented, ✓ approved; Lag: waiting on you or since your review",
    ];
    let legend = lines.join("\n") + "\n";
    if color {
        format!("{DRAFT_ROW_STYLE}{legend}{RESET_STYLE}")
    } else {
        legend
    }
}
