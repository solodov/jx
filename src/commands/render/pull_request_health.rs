use super::*;
use crate::github::{
    PullRequestAutoMergeStatus, PullRequestMergeStatus, PullRequestTimelineEventKind,
};

pub(in crate::commands) const BOLD_STYLE: &str = "\x1b[1m";
const BLACK_BOLD_STYLE: &str = "\x1b[1m\x1b[30m";
const BLACK_ITALIC_STYLE: &str = "\x1b[3m\x1b[30m";
pub(in crate::commands) const DIM_STYLE: &str = "\x1b[2m";
pub(in crate::commands) const DRAFT_ROW_STYLE: &str = "\x1b[2m\x1b[38;2;184;184;184m";
pub(in crate::commands) const GREEN_STYLE: &str = "\x1b[32m";
const GREEN_ITALIC_STYLE: &str = "\x1b[3m\x1b[32m";
const MERGED_APPROVED_REVIEWER_STYLE: &str = "\x1b[38;2;118;108;96m";
const ORANGE_STYLE: &str = "\x1b[38;2;194;95;0m";
pub(in crate::commands) const CONFLICT_STYLE: &str = "\x1b[31m";
pub(in crate::commands) const PASTEL_BLUE_STYLE: &str = "\x1b[38;2;130;165;218m";
const RED_BOLD_STYLE: &str = "\x1b[1m\x1b[31m";
const YELLOW_STYLE: &str = "\x1b[33m";
const CYAN_STYLE: &str = "\x1b[36m";
pub(in crate::commands) const RESET_STYLE: &str = "\x1b[0m";
pub(in crate::commands) const PULL_REQUEST_STATUS_PR_WIDTH: usize = 7;
pub(in crate::commands) const PULL_REQUEST_TITLE_MAX_WIDTH: usize = 72;

pub(in crate::commands) fn ellipsize_pull_request_title(title: &str) -> String {
    ellipsize_rendered_line(title, Some(PULL_REQUEST_TITLE_MAX_WIDTH))
}

/// Renders pastel label chips for active PRs and monochrome chips that restore draft row styling.
pub(in crate::commands) fn pull_request_label_chips(
    labels: &[PullRequestLabel],
    color: bool,
    draft: bool,
) -> Vec<String> {
    labels
        .iter()
        .map(|label| pull_request_label_chip(label, color, draft))
        .collect()
}

/// Renders monochrome label chips for completed PRs without applying draft row styling.
pub(in crate::commands) fn muted_pull_request_label_chips(
    labels: &[PullRequestLabel],
    color: bool,
) -> Vec<String> {
    labels
        .iter()
        .map(|label| pull_request_label_chip_with_restore(label, color, true, ""))
        .collect()
}

pub(in crate::commands) fn pull_request_label_separator(color: bool) -> &'static str {
    if color {
        ""
    } else {
        " "
    }
}

fn pull_request_label_chip(label: &PullRequestLabel, color: bool, draft: bool) -> String {
    let restore_style = if draft { DRAFT_ROW_STYLE } else { "" };
    pull_request_label_chip_with_restore(label, color, draft, restore_style)
}

/// Draws each chip at normal intensity, then restores the surrounding row style.
fn pull_request_label_chip_with_restore(
    label: &PullRequestLabel,
    color: bool,
    muted: bool,
    restore_style: &str,
) -> String {
    if !color {
        return plain_pull_request_label_chip(&label.name);
    }
    let (red, green, blue) = if muted {
        (232, 232, 232)
    } else {
        github_label_rgb(&label.color)
            .map(pastel_github_label_rgb)
            .unwrap_or((221, 221, 221))
    };
    // Both palettes retain at least 4.5:1 contrast, including the darkest active background.
    let (text_red, text_green, text_blue) = if muted { (98, 98, 98) } else { (52, 49, 46) };
    let display_name = compact_label_name(&label.name);
    // SGR 22 clears inherited bold/dim intensity so terminal dimming cannot alter contrast.
    format!(
        "\x1b[22m\x1b[48;2;{red};{green};{blue}m\x1b[38;2;{text_red};{text_green};{text_blue}m {display_name} {RESET_STYLE}{restore_style}"
    )
}

fn plain_pull_request_label_chip(name: &str) -> String {
    format!("[{}]", compact_label_name(name))
}

fn compact_label_name(name: &str) -> String {
    let mut compacted = String::with_capacity(name.len());
    let mut chars = name.chars().peekable();
    while let Some(ch) = chars.next() {
        compacted.push(ch);
        if ch == ':' {
            while matches!(chars.peek(), Some(' ' | '\t')) {
                chars.next();
            }
        }
    }
    compacted
}

/// Blends active PR label colors toward warm off-white.
fn pastel_github_label_rgb((red, green, blue): (u8, u8, u8)) -> (u8, u8, u8) {
    const BLEND_TARGET: (u8, u8, u8) = (248, 246, 242);
    (
        blend_color_channel(red, BLEND_TARGET.0, 25),
        blend_color_channel(green, BLEND_TARGET.1, 25),
        blend_color_channel(blue, BLEND_TARGET.2, 25),
    )
}

fn blend_color_channel(source: u8, target: u8, source_weight_percent: u16) -> u8 {
    let target_weight_percent = 100 - source_weight_percent;
    let value =
        u16::from(source) * source_weight_percent + u16::from(target) * target_weight_percent + 50;
    (value / 100) as u8
}

fn github_label_rgb(color: &str) -> Option<(u8, u8, u8)> {
    let color = color.trim().trim_start_matches('#');
    if color.len() != 6 || !color.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some((
        u8::from_str_radix(&color[0..2], 16).ok()?,
        u8::from_str_radix(&color[2..4], 16).ok()?,
        u8::from_str_radix(&color[4..6], 16).ok()?,
    ))
}

pub(in crate::commands) fn pull_request_status_user_logins<'a>(
    statuses: impl IntoIterator<Item = &'a PullRequestStatusRecord>,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut logins = Vec::new();
    for status in statuses {
        for login in pull_request_status_user_login_iter(status) {
            if seen.insert(login.to_owned()) {
                logins.push(login.to_owned());
            }
        }
    }
    logins
}

fn pull_request_status_user_login_iter(
    status: &PullRequestStatusRecord,
) -> impl Iterator<Item = &str> {
    status
        .author
        .iter()
        .chain(status.requested_reviewers.users.iter())
        .chain(status.suggested_reviewers.iter())
        .chain(status.approved_reviewers.iter())
        .chain(status.changes_requested_reviewers.iter())
        .chain(status.commented_reviewers.iter())
        .chain(status.addressed_reviewers.iter())
        .chain(status.dismissed_reviewers.iter())
        .map(String::as_str)
}

pub(in crate::commands) fn pull_request_reviewer_tokens(
    status: &PullRequestStatusRecord,
    color: bool,
    display_names: &BTreeMap<String, String>,
) -> Vec<String> {
    let approved = status
        .approved_reviewers
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let changes_requested = status
        .changes_requested_reviewers
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let commented = status
        .commented_reviewers
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let addressed = status
        .addressed_reviewers
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let dismissed = status
        .dismissed_reviewers
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let requested_names = requested_reviewer_names(&status.requested_reviewers);
    let requested = requested_names.iter().cloned().collect::<BTreeSet<_>>();
    let suggested_names = if status.draft {
        status.suggested_reviewers.clone()
    } else {
        Vec::new()
    };

    let mut tokens = Vec::new();
    for name in &requested_names {
        if approved.contains(name.as_str()) {
            continue;
        }
        let state = if changes_requested.contains(name.as_str()) {
            ReviewerTokenState::ChangesRequested
        } else if commented.contains(name.as_str()) {
            ReviewerTokenState::Commented
        } else if addressed.contains(name.as_str()) {
            ReviewerTokenState::Addressed
        } else {
            ReviewerTokenState::Requested
        };
        tokens.push(pull_request_reviewer_token(
            name,
            state,
            None,
            color,
            display_names,
        ));
    }
    tokens.extend(
        status
            .changes_requested_reviewers
            .iter()
            .filter(|name| !approved.contains(name.as_str()) && !requested.contains(name.as_str()))
            .map(|name| {
                pull_request_reviewer_token(
                    name,
                    ReviewerTokenState::ChangesRequested,
                    None,
                    color,
                    display_names,
                )
            }),
    );
    tokens.extend(
        status
            .commented_reviewers
            .iter()
            .filter(|name| {
                !approved.contains(name.as_str())
                    && !changes_requested.contains(name.as_str())
                    && !requested.contains(name.as_str())
            })
            .map(|name| {
                pull_request_reviewer_token(
                    name,
                    ReviewerTokenState::Commented,
                    None,
                    color,
                    display_names,
                )
            }),
    );
    tokens.extend(
        status
            .addressed_reviewers
            .iter()
            .filter(|name| {
                !approved.contains(name.as_str())
                    && !changes_requested.contains(name.as_str())
                    && !commented.contains(name.as_str())
                    && !requested.contains(name.as_str())
            })
            .map(|name| {
                pull_request_reviewer_token(
                    name,
                    ReviewerTokenState::Addressed,
                    None,
                    color,
                    display_names,
                )
            }),
    );
    // Dismissed reviews are stale reviewer context, not an active GitHub request.
    tokens.extend(
        status
            .dismissed_reviewers
            .iter()
            .filter(|name| {
                !approved.contains(name.as_str())
                    && !changes_requested.contains(name.as_str())
                    && !commented.contains(name.as_str())
                    && !addressed.contains(name.as_str())
                    && !requested.contains(name.as_str())
            })
            .map(|name| {
                pull_request_reviewer_token(
                    name,
                    ReviewerTokenState::Addressed,
                    None,
                    color,
                    display_names,
                )
            }),
    );
    tokens.extend(suggested_names.iter().filter_map(|name| {
        let name = name.as_str();
        (!approved.contains(name)
            && !changes_requested.contains(name)
            && !commented.contains(name)
            && !addressed.contains(name)
            && !dismissed.contains(name)
            && !requested.contains(name))
        .then(|| {
            pull_request_reviewer_token(
                name,
                ReviewerTokenState::Requested,
                None,
                color,
                display_names,
            )
        })
    }));
    tokens.extend(status.approved_reviewers.iter().map(|name| {
        let state = if commented.contains(name.as_str()) {
            ReviewerTokenState::ApprovedWithComments
        } else {
            ReviewerTokenState::Approved
        };
        pull_request_reviewer_token(name, state, None, color, display_names)
    }));
    tokens
}

pub(in crate::commands) const REVIEW_LAG_WIDTH: usize = 4;

pub(in crate::commands) struct ReviewLagCell {
    pub(in crate::commands) label: String,
    pub(in crate::commands) over_threshold: bool,
    subdued: bool,
}

pub(in crate::commands) fn pull_request_stack_review_lag(
    status: Option<&PullRequestStatusRecord>,
    threshold_seconds: Option<u64>,
) -> ReviewLagCell {
    review_lag_cell(
        status.and_then(stack_review_lag_timestamp),
        threshold_seconds,
    )
}

pub(in crate::commands) fn render_review_lag_cell(
    cell: &ReviewLagCell,
    color: bool,
    restore_style: &str,
    merged: bool,
    subdued_only: bool,
) -> String {
    let label = format!("{:<width$}", cell.label, width = REVIEW_LAG_WIDTH);
    if !color {
        return label;
    }
    if subdued_only {
        return format!("{DIM_STYLE}{label}{RESET_STYLE}{restore_style}");
    }
    if merged && cell.label != "—" {
        return format!("{GREEN_STYLE}{label}{RESET_STYLE}{restore_style}");
    }
    if cell.over_threshold {
        return format!("{RED_BOLD_STYLE}{label}{RESET_STYLE}{restore_style}");
    }
    if cell.subdued {
        return format!("{DIM_STYLE}{label}{RESET_STYLE}{restore_style}");
    }
    label
}

pub(in crate::commands) fn pull_request_completed_reviewer_tokens(
    status: &PullRequestStatusRecord,
    color: bool,
    display_names: &BTreeMap<String, String>,
) -> Vec<String> {
    let approved = status
        .approved_reviewers
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    let changes_requested = status
        .changes_requested_reviewers
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    let mut tokens = Vec::new();
    tokens.extend(
        status
            .changes_requested_reviewers
            .iter()
            .filter(|name| !approved.contains(name.as_str()))
            .map(|name| {
                pull_request_reviewer_token(
                    name,
                    ReviewerTokenState::ChangesRequested,
                    None,
                    color,
                    display_names,
                )
            }),
    );
    tokens.extend(
        status
            .commented_reviewers
            .iter()
            .filter(|name| {
                !approved.contains(name.as_str()) && !changes_requested.contains(name.as_str())
            })
            .map(|name| {
                pull_request_reviewer_token(
                    name,
                    ReviewerTokenState::Commented,
                    None,
                    color,
                    display_names,
                )
            }),
    );
    tokens.extend(status.approved_reviewers.iter().map(|name| {
        pull_request_reviewer_token(
            name,
            ReviewerTokenState::MergedApproved,
            None,
            color,
            display_names,
        )
    }));
    tokens
}

#[derive(Clone, Copy)]
enum ReviewerTokenState {
    Requested,
    ChangesRequested,
    Commented,
    Addressed,
    Approved,
    ApprovedWithComments,
    MergedApproved,
}

fn pull_request_reviewer_token(
    login: &str,
    state: ReviewerTokenState,
    age: Option<String>,
    color: bool,
    display_names: &BTreeMap<String, String>,
) -> String {
    let name = pull_request_user_short_name(login, display_names);
    let label = match age {
        Some(age) => format!("{name} {age}"),
        None => name.to_owned(),
    };
    if !color {
        return label;
    }
    let style = match state {
        ReviewerTokenState::Requested => BLACK_BOLD_STYLE,
        ReviewerTokenState::ChangesRequested => RED_BOLD_STYLE,
        ReviewerTokenState::Commented => ORANGE_STYLE,
        ReviewerTokenState::Addressed => BLACK_ITALIC_STYLE,
        ReviewerTokenState::Approved => GREEN_STYLE,
        ReviewerTokenState::ApprovedWithComments => GREEN_ITALIC_STYLE,
        ReviewerTokenState::MergedApproved => MERGED_APPROVED_REVIEWER_STYLE,
    };
    format!("{style}{label}{RESET_STYLE}")
}

fn stack_review_lag_timestamp(
    status: &PullRequestStatusRecord,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if status.draft {
        return draft_started_at(status).or_else(|| created_at(status));
    }
    latest_review_timestamp(status)
        .into_iter()
        .chain(ready_for_review_started_at(status))
        .chain(created_at(status))
        .max()
}

/// Selects the viewer's review-wait timestamp when no inbox history timestamp is available.
pub(in crate::commands) fn viewer_review_lag_timestamp(
    status: &PullRequestStatusRecord,
    viewer: &str,
    waiting_on_viewer: bool,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let reviewer_review = reviewer_review_timestamp(status, viewer);
    let review_request = latest_review_request_timestamp(status, viewer);
    if waiting_on_viewer {
        return review_request
            .into_iter()
            .chain(ready_for_review_started_at(status))
            .chain(created_at(status))
            .max()
            .or(reviewer_review);
    }
    reviewer_review
        .into_iter()
        .chain(review_request)
        .chain(ready_for_review_started_at(status))
        .chain(created_at(status))
        .max()
}

fn latest_review_timestamp(
    status: &PullRequestStatusRecord,
) -> Option<chrono::DateTime<chrono::Utc>> {
    status
        .review_activity
        .iter()
        .filter_map(|activity| parse_review_lag_timestamp(activity.reviewed_at.as_str()))
        .max()
}

fn reviewer_review_timestamp(
    status: &PullRequestStatusRecord,
    reviewer: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    status
        .review_activity
        .iter()
        .filter(|activity| activity.reviewer == reviewer)
        .filter_map(|activity| parse_review_lag_timestamp(activity.reviewed_at.as_str()))
        .max()
}

fn latest_review_request_timestamp(
    status: &PullRequestStatusRecord,
    reviewer: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    status
        .timeline_events
        .iter()
        .filter(|event| event.kind == PullRequestTimelineEventKind::ReviewRequested)
        .filter(|event| event.reviewer.as_deref() == Some(reviewer))
        .filter_map(|event| parse_review_lag_timestamp(event.created_at.as_str()))
        .max()
}

fn ready_for_review_started_at(
    status: &PullRequestStatusRecord,
) -> Option<chrono::DateTime<chrono::Utc>> {
    status
        .timeline_events
        .iter()
        .filter(|event| event.kind == PullRequestTimelineEventKind::ReadyForReview)
        .filter_map(|event| parse_review_lag_timestamp(event.created_at.as_str()))
        .max()
}

fn draft_started_at(status: &PullRequestStatusRecord) -> Option<chrono::DateTime<chrono::Utc>> {
    status
        .timeline_events
        .iter()
        .filter(|event| event.kind == PullRequestTimelineEventKind::ConvertToDraft)
        .filter_map(|event| parse_review_lag_timestamp(event.created_at.as_str()))
        .max()
}

fn created_at(status: &PullRequestStatusRecord) -> Option<chrono::DateTime<chrono::Utc>> {
    status
        .created_at
        .as_deref()
        .and_then(parse_review_lag_timestamp)
}

/// Formats the selected timestamp as a lag label and threshold signal.
pub(in crate::commands) fn review_lag_cell(
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
    threshold_seconds: Option<u64>,
) -> ReviewLagCell {
    let Some(timestamp) = timestamp else {
        return ReviewLagCell {
            label: "—".to_owned(),
            over_threshold: false,
            subdued: false,
        };
    };
    let now = chrono::Utc::now();
    let age = now.signed_duration_since(timestamp);
    let threshold = threshold_seconds
        .and_then(|seconds| i64::try_from(seconds).ok().map(chrono::Duration::seconds));
    let over_threshold = threshold.is_some_and(|threshold| age > threshold);
    ReviewLagCell {
        label: review_lag_label_since(timestamp, now),
        over_threshold,
        subdued: threshold.is_some() && !over_threshold,
    }
}

fn parse_review_lag_timestamp(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
}

fn review_lag_label_since(
    since: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let age = now.signed_duration_since(since);
    if age < chrono::Duration::hours(1) {
        return "<1h".to_owned();
    }
    if age < chrono::Duration::days(1) {
        let hours = (age.num_minutes() + 59) / 60;
        return format!("{hours}h");
    }
    let days = (age.num_hours() + 23) / 24;
    format!("{days}d")
}

/// Uses the first word of a person's display name in human tables, preserving login fallbacks and teams.
pub(in crate::commands) fn pull_request_user_short_name<'a>(
    login: &'a str,
    display_names: &'a BTreeMap<String, String>,
) -> &'a str {
    if login.starts_with("team/") {
        return login;
    }
    display_names
        .get(login)
        .and_then(|name| name.split_whitespace().next())
        .unwrap_or(login)
}

fn requested_reviewer_names(reviewers: &ReviewerSelection) -> Vec<String> {
    let mut names = reviewers.users.clone();
    names.extend(reviewers.teams.iter().map(|team| format!("team/{team}")));
    names
}

pub(in crate::commands) fn pull_request_has_merge_conflict(
    status: &PullRequestStatusRecord,
) -> bool {
    !status.merged
        && !status.closed
        && matches!(status.merge_status, PullRequestMergeStatus::Conflicting)
}

pub(in crate::commands) fn pull_request_node_symbol(
    status: Option<&PullRequestStatusRecord>,
    draft: bool,
) -> String {
    let symbol = if status.is_some_and(pull_request_has_merge_conflict) {
        "⊘"
    } else if status.is_some_and(|status| status.closed && !status.merged) {
        "⊖"
    } else if draft {
        "◌"
    } else if status.is_some_and(|status| {
        status.auto_merge_status == PullRequestAutoMergeStatus::PrerequisitesRequired
    }) {
        "◈"
    } else if status
        .is_some_and(|status| status.auto_merge_status == PullRequestAutoMergeStatus::Missing)
    {
        "◆"
    } else if status
        .is_some_and(|status| status.auto_merge_status == PullRequestAutoMergeStatus::Armed)
    {
        "◎"
    } else {
        "◯"
    };
    symbol.to_owned()
}

/// Renders the lifecycle marker and title, emphasizing auto-merge gaps only on active PRs.
pub(in crate::commands) fn pull_request_node_title_with_restore(
    status: Option<&PullRequestStatusRecord>,
    draft: bool,
    title: &str,
    color: bool,
    restore_style: &str,
) -> String {
    let symbol = pull_request_node_symbol(status, draft);
    let Some(status) = status else {
        return format!("{symbol} {title}");
    };
    let active_color = color && !draft && !status.closed && !status.merged;
    match status.auto_merge_status {
        PullRequestAutoMergeStatus::Missing | PullRequestAutoMergeStatus::PrerequisitesRequired
            if active_color =>
        {
            format!("{ORANGE_STYLE}{symbol} {title}{RESET_STYLE}{restore_style}")
        }
        PullRequestAutoMergeStatus::Armed if active_color && symbol == "◎" => {
            format!("{CYAN_STYLE}{symbol}{RESET_STYLE}{restore_style} {title}")
        }
        _ => format!("{symbol} {title}"),
    }
}

/// Renders a three-column check label, or a plain symbol; unavailable checks leave a blank cell.
pub(in crate::commands) fn pull_request_check_cell(
    status: Option<&PullRequestStatusRecord>,
    merged: bool,
    color: bool,
    restore_style: &str,
) -> String {
    let signal = if merged {
        Some(("✓", PullRequestStatusStyle::Good))
    } else {
        match status.map(|status| status.check_status) {
            Some(PullRequestCheckStatus::Passing) => Some(("✓", PullRequestStatusStyle::Good)),
            Some(PullRequestCheckStatus::Failing) => Some(("✗", PullRequestStatusStyle::Bad)),
            Some(PullRequestCheckStatus::Pending) => Some(("◷", PullRequestStatusStyle::Warn)),
            Some(PullRequestCheckStatus::Missing | PullRequestCheckStatus::Unknown) | None => None,
        }
    };
    render_pull_request_status_cell("Chk", signal, color, restore_style)
}

/// Renders known stack review state only for PRs with requested or actual reviewers.
pub(in crate::commands) fn pull_request_review_cell(
    status: Option<&PullRequestStatusRecord>,
    merged: bool,
    color: bool,
    review_lag_over_threshold: bool,
    restore_style: &str,
) -> String {
    render_pull_request_status_cell(
        "Rev",
        stack_review_signal(status, merged, review_lag_over_threshold),
        color,
        restore_style,
    )
}

fn stack_review_signal(
    status: Option<&PullRequestStatusRecord>,
    merged: bool,
    review_lag_over_threshold: bool,
) -> Option<(&'static str, PullRequestStatusStyle)> {
    let status = status?;
    if !pull_request_has_reviewers(status) {
        return None;
    }
    if merged {
        return Some(("✓", PullRequestStatusStyle::Good));
    }
    if status.review_status == PullRequestReviewStatus::ChangesRequested
        || !status.changes_requested_reviewers.is_empty()
    {
        return Some(("!", PullRequestStatusStyle::Bad));
    }
    if status.review_status == PullRequestReviewStatus::Approved
        || !status.approved_reviewers.is_empty()
    {
        return Some((
            "✓",
            if status.review_status == PullRequestReviewStatus::Approved
                && status.commented_reviewers.is_empty()
            {
                PullRequestStatusStyle::Good
            } else {
                PullRequestStatusStyle::Comment
            },
        ));
    }
    if !status.commented_reviewers.is_empty() {
        return Some(("!", PullRequestStatusStyle::Comment));
    }
    if !status.requested_reviewers.is_empty() {
        return Some((
            "?",
            pull_request_review_wait_style(review_lag_over_threshold),
        ));
    }
    None
}

/// Counts requested users/teams and past review participants, but never suggested reviewers.
pub(in crate::commands) fn pull_request_has_reviewers(status: &PullRequestStatusRecord) -> bool {
    !status.requested_reviewers.is_empty()
        || !status.approved_reviewers.is_empty()
        || !status.changes_requested_reviewers.is_empty()
        || !status.commented_reviewers.is_empty()
        || !status.addressed_reviewers.is_empty()
        || !status.dismissed_reviewers.is_empty()
        || !status.review_activity.is_empty()
}

pub(in crate::commands) fn pull_request_review_wait_style(
    review_lag_over_threshold: bool,
) -> PullRequestStatusStyle {
    if review_lag_over_threshold {
        PullRequestStatusStyle::Bad
    } else {
        PullRequestStatusStyle::Info
    }
}

#[derive(Clone, Copy)]
pub(in crate::commands) enum PullRequestStatusStyle {
    Good,
    Bad,
    Comment,
    Warn,
    Info,
}

/// Renders a full-intensity status label or a padded plain symbol, then restores the row style.
pub(in crate::commands) fn render_pull_request_status_cell(
    label: &str,
    signal: Option<(&str, PullRequestStatusStyle)>,
    color: bool,
    restore_style: &str,
) -> String {
    let Some((symbol, style)) = signal else {
        return "   ".to_owned();
    };
    if !color {
        return format!("{symbol:<3}");
    }
    let style = match style {
        PullRequestStatusStyle::Good => GREEN_STYLE,
        PullRequestStatusStyle::Bad => RED_BOLD_STYLE,
        PullRequestStatusStyle::Comment => ORANGE_STYLE,
        PullRequestStatusStyle::Warn => YELLOW_STYLE,
        PullRequestStatusStyle::Info => CYAN_STYLE,
    };
    format!("\x1b[22m{style}{label}{RESET_STYLE}{restore_style}")
}

#[cfg(test)]
#[path = "tests/pull_request_health.rs"]
mod tests;
