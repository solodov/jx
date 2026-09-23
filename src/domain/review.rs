use super::*;

/// Detects pending required CI in policy-normalized checks, even when other checks have failed.
pub fn review_request_has_pending_checks(status: &PullRequestStatusRecord) -> bool {
    status
        .checks
        .iter()
        .any(|check| check.required && check.status == PullRequestCheckStatus::Pending)
}

/// Classifies a pull request from the authenticated reviewer's point of view.
pub fn review_request_state(status: &PullRequestStatusRecord, viewer: &str) -> ReviewRequestState {
    let approved = status
        .approved_reviewers
        .iter()
        .any(|reviewer| reviewer == viewer);
    let changes_requested = status
        .changes_requested_reviewers
        .iter()
        .any(|reviewer| reviewer == viewer);
    let directly_requested = status
        .requested_reviewers
        .users
        .iter()
        .any(|reviewer| reviewer == viewer);

    if directly_requested {
        if approved {
            ReviewRequestState::Again
        } else {
            ReviewRequestState::New
        }
    } else if approved {
        ReviewRequestState::Approved
    } else if status
        .addressed_reviewers
        .iter()
        .any(|reviewer| reviewer == viewer)
    {
        ReviewRequestState::Answered
    } else if changes_requested {
        ReviewRequestState::ChangesRequested
    } else if status
        .commented_reviewers
        .iter()
        .any(|reviewer| reviewer == viewer)
    {
        ReviewRequestState::Commented
    } else {
        ReviewRequestState::New
    }
}
