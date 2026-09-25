use super::*;
use serde_json::{json, Value};

#[test]
fn summary_and_full_snapshot_capture_the_same_review_key() {
    let payload = pull_request_payload();
    let summary = summary(&payload);
    let status = status(&payload);
    assert_eq!(
        status.review_refresh_key.as_deref(),
        Some(summary.review_refresh_key.as_str())
    );
    assert_eq!(status.approved_reviewers, ["peer"]);
    assert_eq!(status.requested_reviewers.users, ["reviewer"]);

    for query in [
        pull_request_update_summary_query(&[12]),
        pull_request_status_query(&[12]),
    ] {
        assert!(query.contains("...PullRequestReviewRefreshFields"));
        assert_eq!(query.matches(REVIEW_REFRESH_FRAGMENT).count(), 1);
    }
}

#[test]
fn individual_approval_invalidates_an_already_approved_pr_without_timestamp_or_request_changes() {
    let before = pull_request_payload();
    let mut after = before.clone();
    after["latestReviews"]["nodes"][1]["state"] = json!("APPROVED");
    // Even unchanged request lists, submittedAt, and aggregate APPROVED must not hide this change.
    assert_ne!(
        summary(&before).review_refresh_key,
        summary(&after).review_refresh_key
    );
    assert_eq!(summary(&before).updated_at, summary(&after).updated_at);
    assert_eq!(status(&before).review_status, status(&after).review_status);
    assert_eq!(status(&after).approved_reviewers, ["peer", "reviewer"]);
}

#[test]
fn dismissal_rerequest_and_new_comment_review_invalidate_the_review_key() {
    let initial = pull_request_payload();
    let original_key = summary(&initial).review_refresh_key;
    let mut dismissed = initial.clone();
    dismissed["latestReviews"]["nodes"][0]["state"] = json!("DISMISSED");
    let mut requested = initial.clone();
    requested["reviewRequests"]["nodes"][0]["requestedReviewer"]["login"] =
        json!("another-reviewer");
    let mut removed_request = initial.clone();
    removed_request["reviewRequests"] = json!({"totalCount": 0, "nodes": []});
    let mut commented = initial.clone();
    commented["latestReviews"]["nodes"][1]["submittedAt"] = json!("2026-09-25T13:30:20Z");
    let mut decision = initial.clone();
    decision["reviewDecision"] = json!("CHANGES_REQUESTED");
    for changed in [dismissed, requested, removed_request, commented, decision] {
        assert_ne!(summary(&changed).review_refresh_key, original_key);
        assert_eq!(
            status(&changed).review_refresh_key,
            Some(summary(&changed).review_refresh_key)
        );
    }
}

#[test]
fn review_connection_order_does_not_invalidate_the_cache() {
    let mut before = pull_request_payload();
    before["reviewRequests"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"requestedReviewer": {"__typename": "User", "login": "other"}}));
    before["reviewRequests"]["totalCount"] = json!(2);
    let mut reordered = before.clone();
    reordered["latestReviews"]["nodes"]
        .as_array_mut()
        .unwrap()
        .reverse();
    reordered["reviewRequests"]["nodes"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert_eq!(
        summary(&before).review_refresh_key,
        summary(&reordered).review_refresh_key
    );
}

#[test]
fn old_snapshots_remain_readable_without_a_review_key() {
    let current = status(&pull_request_payload());
    let mut old = serde_json::to_value(&current).unwrap();
    old.as_object_mut().unwrap().remove("review_refresh_key");
    let restored: PullRequestStatusRecord = serde_json::from_value(old).unwrap();
    assert_eq!(restored.review_refresh_key, None);
    assert_eq!(restored.approved_reviewers, current.approved_reviewers);
}

fn summary(payload: &Value) -> PullRequestUpdateSummary {
    map_graphql_pull_request_update_summary(serde_json::from_value(payload.clone()).unwrap())
}

fn status(payload: &Value) -> PullRequestStatusRecord {
    map_graphql_pull_request_status(serde_json::from_value(payload.clone()).unwrap())
}

fn pull_request_payload() -> Value {
    json!({
        "number": 12,
        "title": "Awaiting my review",
        "url": "https://github.com/owner/repo/pull/12",
        "createdAt": "2026-09-23T12:00:00Z",
        "updatedAt": "2026-09-24T21:10:47Z",
        "headRefName": "topic/review",
        "baseRefName": "main",
        "baseRepository": {"defaultBranchRef": {"name": "main"}},
        "author": {"__typename": "User", "login": "author"},
        "isDraft": false,
        "merged": false,
        "closed": false,
        "mergeable": "MERGEABLE",
        "reviewDecision": "APPROVED",
        "reviewRequests": {"totalCount": 1, "nodes": [{"requestedReviewer": {"__typename": "User", "login": "reviewer"}}]},
        "latestReviews": {"nodes": [
            {"state": "APPROVED", "submittedAt": "2026-09-24T21:10:47Z", "author": {"__typename": "User", "login": "peer"}, "authorAssociation": "MEMBER"},
            {"state": "COMMENTED", "submittedAt": "2026-09-24T13:44:51Z", "author": {"__typename": "User", "login": "reviewer"}, "authorAssociation": "MEMBER"}
        ]},
        "suggestedReviewers": [],
        "labels": {"nodes": []},
        "reviews": {"nodes": []},
        "comments": {"nodes": []},
        "reviewThreads": {"nodes": []},
        "timelineItems": {"nodes": []},
        "commits": {"nodes": [{"commit": {"oid": "same-head", "statusCheckRollup": null}}]}
    })
}
