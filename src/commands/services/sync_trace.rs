//! Sync diagnostics derived from facts already loaded by the workflow.

use super::*;

/// Captures PR lifecycle and base without logging authored titles or bodies.
pub(super) fn pull_request_fact_attrs(pull_request: &PullRequestRecord) -> Vec<PerfAttr> {
    vec![
        perf_attr("number", pull_request.number),
        perf_attr("head_branch", &pull_request.head_branch),
        perf_attr("base", &pull_request.base_branch),
        perf_attr("merged", pull_request.merged),
        perf_attr("draft", pull_request.draft),
    ]
}

/// Captures retargeting intent and the prior cached base, without another API call.
pub(super) fn pull_request_update_attrs(
    number: u64,
    request: &PullRequestUpdate,
    previous: Option<&PullRequestRecord>,
) -> Vec<PerfAttr> {
    let mut attrs = vec![
        perf_attr("number", number),
        perf_attr("update_title", request.title.is_some()),
        perf_attr("update_body", request.body.is_some()),
        perf_attr("update_base", request.base.is_some()),
        perf_attr("base_before_known", previous.is_some()),
    ];
    if let Some(previous) = previous {
        attrs.push(perf_attr("base_before", &previous.base_branch));
        attrs.push(perf_attr("merged_before", previous.merged));
    }
    if let Some(base) = &request.base {
        attrs.push(perf_attr("requested_base", base));
    }
    attrs
}

/// Records why a successful partial sync left particular bookmarks unpushed.
pub(super) fn record_skipped_pushes(
    environment: &RuntimeEnvironment,
    context: &RepositoryContext,
    outcome: &SyncPushOutcome,
) {
    if outcome.skipped_conflicted_bookmarks.is_empty() {
        return;
    }
    let mut span = PerfLog::from_environment(environment).start(
        "sync.skipped_pushes",
        [
            perf_attr("repo", context.origin.github.slug()),
            perf_attr(
                "workspace_root",
                context.workspace_root.display().to_string(),
            ),
        ],
    );
    for bookmark in &outcome.skipped_conflicted_bookmarks {
        span.record_diagnostic(
            "push_skipped_conflict",
            [
                perf_attr("branch", &bookmark.branch),
                perf_attr(
                    "conflicted_commits",
                    bookmark
                        .conflicted_commits
                        .iter()
                        .map(|commit| commit.short_commit_id.as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                ),
            ],
            None::<&JjError>,
        );
    }
    span.end();
}

#[cfg(test)]
#[path = "../tests/sync_trace.rs"]
mod tests;
