use super::*;
use crate::domain::ReviewRequestState;
use crate::github::PullRequestReviewerMention;

const JX_DISMISSAL_LABEL_COLOR: &str = "5319e7";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct ReviewRequestsView {
    pub(in crate::commands) viewer: String,
    pub(in crate::commands) repositories: Vec<ReviewRequestRepositoryView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct ReviewRequestRepositoryView {
    pub(in crate::commands) repository: GitHubRepository,
    pub(in crate::commands) layout_key: Option<String>,
    pub(in crate::commands) root: Option<PathBuf>,
    pub(in crate::commands) display_root: Option<String>,
    pub(in crate::commands) rows: Vec<ReviewRequestRowView>,
    pub(in crate::commands) external: bool,
    pub(in crate::commands) review_wait_threshold_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct ReviewRequestRowView {
    pub(in crate::commands) status: PullRequestStatusRecord,
    pub(in crate::commands) state: ReviewRequestState,
    pub(in crate::commands) viewer_signal: ReviewRequestViewerSignal,
    pub(in crate::commands) lag_since_unix: Option<i64>,
    pub(in crate::commands) dismissal: Option<ReviewRequestDismissalView>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::commands) enum ReviewRequestViewerSignal {
    #[default]
    None,
    DismissedApproval,
}

impl ReviewRequestViewerSignal {
    fn label(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::DismissedApproval => Some("dismissed_approval"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::commands) struct ReviewRequestDismissalView {
    pub(in crate::commands) source: String,
    pub(in crate::commands) reason: String,
}

pub(in crate::commands) fn render_review_requests(
    view: &ReviewRequestsView,
    color: bool,
    terminal_width: Option<usize>,
    display_names: &BTreeMap<String, String>,
) -> String {
    let mut output = String::new();
    if view.repositories.is_empty() {
        return output;
    }

    for (index, repository) in view.repositories.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&review_repository_header(repository, color));
        output.push('\n');
        output.push_str(&format!(
            "  {pr:<pr_width$}  Chk  Rev  {:<lag_width$}  Title\n",
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

    output
}

/// Renders review requests as a stable JSON data-provider surface for external selectors.
pub(in crate::commands) fn render_review_requests_json(
    view: &ReviewRequestsView,
    display_names: &BTreeMap<String, String>,
) -> String {
    let output = ReviewRequestsJson {
        command: "review",
        version: 1,
        viewer: ReviewViewerJson::new(&view.viewer, display_names),
        display_names: display_names.clone(),
        pull_requests: view
            .repositories
            .iter()
            .flat_map(|repository| {
                repository
                    .rows
                    .iter()
                    .map(move |row| ReviewPullRequestJson::from_row(repository, row))
            })
            .collect(),
    };
    let mut rendered = serde_json::to_string_pretty(&output)
        .expect("review JSON contains only serializable values");
    rendered.push('\n');
    rendered
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewRequestsJson {
    command: &'static str,
    version: u8,
    viewer: ReviewViewerJson,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    display_names: BTreeMap<String, String>,
    pull_requests: Vec<ReviewPullRequestJson>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewViewerJson {
    login: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
}

impl ReviewViewerJson {
    fn new(login: &str, display_names: &BTreeMap<String, String>) -> Self {
        Self {
            login: login.to_owned(),
            display_name: display_names.get(login).cloned(),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewPullRequestJson {
    repository: String,
    repository_owner: String,
    repository_name: String,
    repository_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_root: Option<String>,
    external: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    review_wait_threshold_seconds: Option<u64>,
    number: u64,
    url: String,
    title: String,
    branch: String,
    base_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    draft: bool,
    merged: bool,
    closed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    merged_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    closed_at: Option<String>,
    check_status: &'static str,
    merge_status: &'static str,
    review_status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_merge_status: Option<&'static str>,
    request_state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    viewer_review_signal: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lag_since_unix: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dismissal: Option<ReviewDismissalJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labels: Vec<ReviewLabelJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    requested_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    requested_teams: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggested_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    approved_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    changes_requested_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    commented_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    addressed_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reviewer_responses: Vec<ReviewResponseJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reviewer_mentions: Vec<ReviewMentionJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dismissed_users: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    review_activity: Vec<ReviewActivityJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    timeline_events: Vec<ReviewTimelineEventJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_commit_oid: Option<String>,
}

impl ReviewPullRequestJson {
    fn from_row(repository: &ReviewRequestRepositoryView, row: &ReviewRequestRowView) -> Self {
        let status = &row.status;
        Self {
            repository: repository.repository.slug(),
            repository_owner: repository.repository.owner.clone(),
            repository_name: repository.repository.name.clone(),
            repository_url: repository.repository.https_url(),
            key: repository.layout_key.clone(),
            root: repository
                .root
                .as_ref()
                .map(|root| root.display().to_string()),
            display_root: repository.display_root.clone(),
            external: repository.external,
            review_wait_threshold_seconds: repository.review_wait_threshold_seconds,
            number: status.number,
            url: review_request_url(&repository.repository, status),
            title: status.title.clone(),
            branch: status.head_branch.clone(),
            base_branch: status.base_branch.clone(),
            created_at: status.created_at.clone(),
            author: status.author.clone(),
            draft: status.draft,
            merged: status.merged,
            closed: status.closed,
            merged_at: status.merged_at.clone(),
            closed_at: status.closed_at.clone(),
            check_status: status.check_status.label(),
            merge_status: status.merge_status.label(),
            review_status: status.review_status.label(),
            auto_merge_status: review_auto_merge_status_label(status),
            request_state: row.state.label(),
            viewer_review_signal: row.viewer_signal.label(),
            lag_since_unix: row.lag_since_unix,
            dismissal: row.dismissal.as_ref().map(ReviewDismissalJson::from),
            labels: status.labels.iter().map(ReviewLabelJson::from).collect(),
            requested_users: status.requested_reviewers.users.clone(),
            requested_teams: status.requested_reviewers.teams.clone(),
            suggested_users: status.suggested_reviewers.clone(),
            approved_users: status.approved_reviewers.clone(),
            changes_requested_users: status.changes_requested_reviewers.clone(),
            commented_users: status.commented_reviewers.clone(),
            addressed_users: status.addressed_reviewers.clone(),
            reviewer_responses: status
                .reviewer_responses
                .iter()
                .map(ReviewResponseJson::from)
                .collect(),
            reviewer_mentions: status
                .reviewer_mentions
                .iter()
                .map(ReviewMentionJson::from)
                .collect(),
            dismissed_users: status.dismissed_reviewers.clone(),
            review_activity: status
                .review_activity
                .iter()
                .map(ReviewActivityJson::from)
                .collect(),
            timeline_events: status
                .timeline_events
                .iter()
                .map(ReviewTimelineEventJson::from)
                .collect(),
            latest_commit_oid: status.latest_commit_oid.clone(),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewDismissalJson {
    source: String,
    reason: String,
}

impl From<&ReviewRequestDismissalView> for ReviewDismissalJson {
    fn from(dismissal: &ReviewRequestDismissalView) -> Self {
        Self {
            source: dismissal.source.clone(),
            reason: dismissal.reason.clone(),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewLabelJson {
    name: String,
    color: String,
}

impl From<&PullRequestLabel> for ReviewLabelJson {
    fn from(label: &PullRequestLabel) -> Self {
        Self {
            name: label.name.clone(),
            color: label.color.clone(),
        }
    }
}

fn review_auto_merge_status_label(status: &PullRequestStatusRecord) -> Option<&'static str> {
    (!status.auto_merge_status.is_not_configured()).then(|| status.auto_merge_status.label())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewActivityJson {
    reviewer: String,
    reviewed_at: String,
}

impl From<&PullRequestReviewActivity> for ReviewActivityJson {
    fn from(activity: &PullRequestReviewActivity) -> Self {
        Self {
            reviewer: activity.reviewer.clone(),
            reviewed_at: activity.reviewed_at.clone(),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewResponseJson {
    reviewer: String,
    responded_at: String,
}

impl From<&PullRequestReviewerResponse> for ReviewResponseJson {
    fn from(response: &PullRequestReviewerResponse) -> Self {
        Self {
            reviewer: response.reviewer.clone(),
            responded_at: response.responded_at.clone(),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewMentionJson {
    reviewer: String,
    mentioned_at: String,
}

impl From<&PullRequestReviewerMention> for ReviewMentionJson {
    fn from(mention: &PullRequestReviewerMention) -> Self {
        Self {
            reviewer: mention.reviewer.clone(),
            mentioned_at: mention.mentioned_at.clone(),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewTimelineEventJson {
    kind: &'static str,
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reviewer: Option<String>,
}

impl From<&PullRequestTimelineEvent> for ReviewTimelineEventJson {
    fn from(event: &PullRequestTimelineEvent) -> Self {
        Self {
            kind: review_timeline_event_kind_label(event.kind),
            created_at: event.created_at.clone(),
            reviewer: event.reviewer.clone(),
        }
    }
}

fn review_timeline_event_kind_label(kind: PullRequestTimelineEventKind) -> &'static str {
    match kind {
        PullRequestTimelineEventKind::ReadyForReview => "ready_for_review",
        PullRequestTimelineEventKind::ConvertToDraft => "convert_to_draft",
        PullRequestTimelineEventKind::ReviewRequested => "review_requested",
    }
}

pub(in crate::commands) fn review_request_url(
    repository: &GitHubRepository,
    status: &PullRequestStatusRecord,
) -> String {
    status
        .url
        .as_deref()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}/pull/{}", repository.https_url(), status.number))
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
    let on_ice = review_request_is_on_ice(&row.status);
    let active_cell_color = row_color && !row.status.draft && !on_ice;
    let row_style = review_request_row_style(repository, row, color);
    let pr = review_request_pr_cell(&repository.repository, &row.status, row_color);
    let check = pull_request_check_symbol_with_restore(
        Some(&row.status),
        row.status.merged,
        active_cell_color,
        row_style,
    );
    let lag = row.lag_since_unix.map_or_else(
        || {
            pull_request_viewer_review_lag(
                &row.status,
                viewer,
                repository.review_wait_threshold_seconds,
                review_request_state_waits_on_viewer(row.state),
            )
        },
        |since_unix| {
            pull_request_review_lag_since_unix(
                Some(since_unix),
                repository.review_wait_threshold_seconds,
            )
        },
    );
    let state = review_request_state_cell(
        &row.status,
        row.state,
        row.viewer_signal,
        viewer,
        active_cell_color,
        lag.over_threshold,
        row_style,
    );
    let lag = render_review_lag_cell(&lag, color && !on_ice, row_style, false, row.status.draft);
    let title = review_request_title(row, row_color, display_names);
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
    if row_style.is_empty() {
        line
    } else {
        format!("{row_style}{line}{RESET_STYLE}")
    }
}

fn review_request_row_style(
    repository: &ReviewRequestRepositoryView,
    row: &ReviewRequestRowView,
    color: bool,
) -> &'static str {
    if !color {
        ""
    } else if pull_request_has_merge_conflict(&row.status) {
        CONFLICT_STYLE
    } else if review_request_is_on_ice(&row.status) {
        PASTEL_BLUE_STYLE
    } else if repository.external {
        DIM_STYLE
    } else if row.status.draft {
        DRAFT_ROW_STYLE
    } else {
        ""
    }
}

fn review_request_is_on_ice(status: &PullRequestStatusRecord) -> bool {
    status.closed && !status.merged
}

fn review_request_pr_cell(
    repository: &GitHubRepository,
    status: &PullRequestStatusRecord,
    color: bool,
) -> String {
    let label = format!("#{}", status.number);
    let url = review_request_url(repository, status);
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

/// Returns true when a policy-visible non-viewer approval exists while this row still waits on the viewer.
fn review_request_waits_on_viewer_with_peer_approval(
    status: &PullRequestStatusRecord,
    state: ReviewRequestState,
    viewer: &str,
) -> bool {
    review_request_state_waits_on_viewer(state)
        && !status
            .approved_reviewers
            .iter()
            .any(|reviewer| reviewer == viewer)
        && status
            .approved_reviewers
            .iter()
            .any(|reviewer| reviewer != viewer)
}

fn review_request_state_cell(
    status: &PullRequestStatusRecord,
    state: ReviewRequestState,
    viewer_signal: ReviewRequestViewerSignal,
    viewer: &str,
    color: bool,
    review_lag_over_threshold: bool,
    restore_style: &str,
) -> String {
    if viewer_signal == ReviewRequestViewerSignal::DismissedApproval {
        return styled_pull_request_symbol_with_restore(
            "✓",
            PullRequestSymbolStyle::Comment,
            color,
            restore_style,
        );
    }
    if review_request_waits_on_viewer_with_peer_approval(status, state, viewer) {
        return styled_pull_request_symbol_with_restore(
            "✓",
            PullRequestSymbolStyle::Comment,
            color,
            restore_style,
        );
    }
    let (symbol, style) = match state {
        ReviewRequestState::New | ReviewRequestState::Answered | ReviewRequestState::Again => (
            "?",
            pull_request_review_wait_style(review_lag_over_threshold),
        ),
        ReviewRequestState::ChangesRequested => ("!", PullRequestSymbolStyle::Bad),
        ReviewRequestState::Commented => ("!", PullRequestSymbolStyle::Comment),
        ReviewRequestState::Approved => (
            "✓",
            if status
                .commented_reviewers
                .iter()
                .any(|reviewer| reviewer == viewer)
            {
                PullRequestSymbolStyle::Comment
            } else {
                PullRequestSymbolStyle::Good
            },
        ),
    };
    styled_pull_request_symbol_with_restore(symbol, style, color, restore_style)
}

fn review_request_label_chips(row: &ReviewRequestRowView, color: bool) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(dismissal) = &row.dismissal {
        labels.push(PullRequestLabel {
            name: format!("jx:dismissed:{}", dismissal.reason),
            color: JX_DISMISSAL_LABEL_COLOR.to_owned(),
        });
    }
    labels.extend(row.status.labels.iter().cloned());
    pull_request_label_chips(&labels, color, row.status.draft)
}

fn review_request_title(
    row: &ReviewRequestRowView,
    color: bool,
    display_names: &BTreeMap<String, String>,
) -> String {
    let status = &row.status;
    let title = ellipsize_pull_request_title(&status.title);
    let mut parts = vec![pull_request_node_title_with_restore(
        Some(status),
        status.draft,
        &title,
        color,
        "",
    )];
    if review_request_is_on_ice(status) {
        return parts.join(" ");
    }
    let label_chips = review_request_label_chips(row, color);
    if !label_chips.is_empty() {
        parts.push(label_chips.join(pull_request_label_separator(color)));
    }
    if let Some(author) = review_request_author_token(status, color, display_names) {
        parts.push(author);
    }
    parts.join(" ")
}

fn review_request_author_token(
    status: &PullRequestStatusRecord,
    color: bool,
    display_names: &BTreeMap<String, String>,
) -> Option<String> {
    let author = status.author.as_deref()?.trim();
    if author.is_empty() {
        return None;
    }
    let author = display_names
        .get(author)
        .map(String::as_str)
        .unwrap_or(author);
    if color {
        Some(format!("{BOLD_STYLE}{author}{RESET_STYLE}"))
    } else {
        Some(author.to_owned())
    }
}
