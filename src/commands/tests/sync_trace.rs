use super::*;
use crate::jj::{ConflictedCommitSummary, SkippedPushBookmarkSummary, WorkspaceVisibility};

#[test]
fn pr_trace_distinguishes_retargeting_from_body_only_updates() {
    let previous = pull_request();
    let update = PullRequestUpdate {
        base: Some("main".to_owned()),
        ..Default::default()
    };
    let attrs = pull_request_update_attrs(previous.number, &update, Some(&previous));
    assert!(attrs.contains(&perf_attr("base_before", "topic/root")));
    assert!(attrs.contains(&perf_attr("requested_base", "main")));
    assert!(attrs.contains(&perf_attr("update_base", true)));
    assert!(attrs.contains(&perf_attr("merged_before", true)));

    let update = PullRequestUpdate {
        body: Some("private authored content".to_owned()),
        ..Default::default()
    };
    let attrs = pull_request_update_attrs(previous.number, &update, None);
    assert!(attrs.contains(&perf_attr("base_before_known", false)));
    assert!(attrs.contains(&perf_attr("update_base", false)));
    assert!(attrs.contains(&perf_attr("update_body", true)));
    assert!(!format!("{attrs:?}").contains("private authored content"));
    let attrs = pull_request_fact_attrs(&previous);
    assert!(attrs.contains(&perf_attr("merged", true)));
    assert!(attrs.contains(&perf_attr("base", "topic/root")));
    assert!(!format!("{attrs:?}").contains("private title"));
}

#[test]
fn fetch_diagnostics_receive_timestamps_without_changing_span_status() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = root.path().join("perf.log");
    let environment = RuntimeEnvironment::new(
        root.path(),
        [("JX_PERF_LOG".to_owned(), path.display().to_string())],
    );
    let mut span = PerfLog::from_environment(&environment).start("jj.fetch_origin", Vec::new());
    super::super::record_fetch_trace_step(
        &mut span,
        FetchTraceStep {
            name: "rebase_result".to_owned(),
            duration_us: 0,
            error: None,
            attrs: vec![
                crate::jj::fetch_trace_attr("diagnostic", true),
                crate::jj::fetch_trace_attr("conflict_after", true),
            ],
        },
    );
    super::super::record_fetch_trace_step(
        &mut span,
        FetchTraceStep {
            name: "git_fetch".to_owned(),
            duration_us: 123,
            error: None,
            attrs: Vec::new(),
        },
    );
    span.end();
    let event: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("log")).expect("JSON");
    assert_eq!(event["status"], "ok");
    assert_eq!(event["steps"][0]["diagnostic"], true);
    assert_eq!(event["steps"][0]["conflict_after"], true);
    chrono::DateTime::parse_from_rfc3339(
        event["steps"][0]["recorded_at"]
            .as_str()
            .expect("timestamp"),
    )
    .expect("RFC3339");
    assert_eq!(event["steps"][1]["duration_us"], 123);
    assert!(event["steps"][1].get("recorded_at").is_none());
}

#[test]
fn skipped_push_trace_records_bookmark_and_conflicting_commits() {
    let root = tempfile::tempdir().expect("tempdir");
    let settings =
        jj_lib::settings::UserSettings::from_config(jj_lib::config::StackedConfig::with_defaults())
            .expect("settings");
    pollster::block_on(async {
        let (_, repo) = jj_lib::workspace::Workspace::init_internal_git(&settings, root.path())
            .await
            .expect("workspace");
        let mut tx = repo.start_transaction();
        jj_lib::git::add_remote(
            tx.repo_mut(),
            jj_lib::ref_name::RemoteName::new("origin"),
            "https://github.com/owner/repo.git",
            None,
            gix::remote::fetch::Tags::None,
        )
        .expect("origin");
        tx.commit("test origin").await.expect("commit");
    });
    let path = root.path().join("perf.log");
    let environment = RuntimeEnvironment::new(
        root.path(),
        [("JX_PERF_LOG".to_owned(), path.display().to_string())],
    );
    let context = RepositoryContext::discover(&environment).expect("context");
    let outcome = SyncPushOutcome {
        pushed: TrackedPushOutcome {
            pushed_refs: 0,
            bookmarks: Vec::new(),
            pushed_commits: Vec::new(),
        },
        skipped_conflicted_bookmarks: vec![SkippedPushBookmarkSummary {
            branch: "topic/child".to_owned(),
            conflicted_commits: vec![ConflictedCommitSummary {
                short_commit_id: "12345678".to_owned(),
                description: "private description".to_owned(),
                workspace_visibility: WorkspaceVisibility::default(),
            }],
        }],
    };
    record_skipped_pushes(&environment, &context, &outcome);
    let text = std::fs::read_to_string(path).expect("log");
    let event: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(event["steps"][0]["branch"], "topic/child");
    assert_eq!(event["steps"][0]["conflicted_commits"], "12345678");
    assert!(!text.contains("private description"));
}

fn pull_request() -> PullRequestRecord {
    PullRequestRecord {
        number: 42,
        title: "private title".to_owned(),
        body: None,
        head_branch: "topic/child".to_owned(),
        base_branch: "topic/root".to_owned(),
        html_url: None,
        draft: false,
        merged: true,
        reviewers: ReviewerSelection::default(),
    }
}
