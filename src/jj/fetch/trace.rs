//! Diagnostic records for fetch decisions, separate from phase timings.

use super::{super::*, TrunkChildChange};

/// Emits a diagnostic without turning an expected conflict into a command error.
pub(in crate::jj) fn record_detail(
    trace: &mut dyn FnMut(FetchTraceStep),
    name: &str,
    mut attrs: Vec<FetchTraceAttr>,
) {
    attrs.push(fetch_trace_attr("diagnostic", true));
    trace(FetchTraceStep {
        name: name.to_owned(),
        duration_us: 0,
        attrs,
        error: None,
    });
}

/// Captures stack roots before import can delete their bookmarks or rewrite them.
pub(in crate::jj) fn record_root_snapshot(
    repo: &dyn jj_lib::repo::Repo,
    root: &TrunkChildChange,
    trace: &mut dyn FnMut(FetchTraceStep),
) {
    let mut attrs = vec![
        fetch_trace_attr("change_id", root.change_id.hex()),
        fetch_trace_attr("old_commit", root.commit_id.hex()),
    ];
    match load_commit_from_repo(repo, &root.commit_id) {
        Ok(commit) => {
            attrs.extend(commit_attrs(&commit));
            let bookmarks = repo
                .view()
                .local_bookmarks()
                .filter(|(_, target)| target.added_ids().any(|id| id == commit.id()))
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>();
            attrs.push(fetch_trace_attr("bookmarks", bookmarks.join(",")));
        }
        Err(error) => attrs.push(fetch_trace_attr("diagnostic_error", error.to_string())),
    }
    record_detail(trace, "rebase_root_snapshot", attrs);
}

/// Records the root selected after import and the reason it is replayed or skipped.
pub(in crate::jj) fn record_rebase_decision(
    root: &TrunkChildChange,
    resolved: Option<&Commit>,
    trunk: &Commit,
    decision: &str,
    trace: &mut dyn FnMut(FetchTraceStep),
) {
    let mut attrs = vec![
        fetch_trace_attr("change_id", root.change_id.hex()),
        fetch_trace_attr("original_commit", root.commit_id.hex()),
        fetch_trace_attr("updated_trunk", trunk.id().hex()),
        fetch_trace_attr("decision", decision),
    ];
    if let Some(commit) = resolved {
        attrs.extend(commit_attrs(commit));
    }
    record_detail(trace, "rebase_decision", attrs);
}

/// Records rewritten and abandoned commits, including bounded conflict-path samples.
pub(in crate::jj) fn record_rebase_result(
    old: &Commit,
    result: &RebasedCommit,
    phase: &str,
    empty_behavior: EmptyBehavior,
    trace: &mut dyn FnMut(FetchTraceStep),
) {
    let mut attrs = commit_attrs(old);
    attrs.push(fetch_trace_attr("phase", phase));
    attrs.push(fetch_trace_attr(
        "empty_policy",
        match empty_behavior {
            EmptyBehavior::Keep => "keep",
            EmptyBehavior::AbandonNewlyEmpty => "abandon_newly_empty",
            EmptyBehavior::AbandonAllEmpty => "abandon_all_empty",
        },
    ));
    match result {
        RebasedCommit::Rewritten(new) => {
            attrs.extend([
                fetch_trace_attr("outcome", "rewritten"),
                fetch_trace_attr("new_commit", new.id().hex()),
                fetch_trace_attr("new_parents", commit_ids(new.parent_ids())),
                fetch_trace_attr("conflict_after", new.has_conflict()),
                fetch_trace_attr("new_conflict", !old.has_conflict() && new.has_conflict()),
            ]);
            if new.has_conflict() {
                attrs.extend(conflict_path_attrs(new));
            }
        }
        RebasedCommit::Abandoned { parent_id } => {
            attrs.extend([
                fetch_trace_attr("outcome", "abandoned"),
                fetch_trace_attr("replacement_parent", parent_id.hex()),
            ]);
        }
    }
    record_detail(trace, "rebase_result", attrs);
}

/// Records imported ref targets and commits removed or rewritten by Git import.
pub(in crate::jj) fn record_import_details(
    repo: &dyn jj_lib::repo::Repo,
    local_before: &BTreeMap<String, RefTarget>,
    stats: &git::GitImportStats,
    trace: &mut dyn FnMut(FetchTraceStep),
) {
    for (symbol, (old, new)) in &stats.changed_remote_bookmarks {
        let mut attrs = vec![
            fetch_trace_attr("branch", symbol.name.as_str()),
            fetch_trace_attr("remote", symbol.remote.as_str()),
            fetch_trace_attr("deleted", new.is_absent()),
            fetch_trace_attr("tracked", old.is_tracked()),
            fetch_trace_attr("old_remote_target", ref_target(&old.target)),
            fetch_trace_attr("new_remote_target", ref_target(new)),
            fetch_trace_attr(
                "new_local_target",
                ref_target(repo.view().get_local_bookmark(&symbol.name)),
            ),
        ];
        if let Some(target) = local_before.get(symbol.name.as_str()) {
            attrs.push(fetch_trace_attr("old_local_target", ref_target(target)));
        }
        record_detail(trace, "import_bookmark", attrs);
    }
    for commit in &stats.abandoned_commits {
        record_detail(trace, "import_abandoned", commit_attrs(commit));
    }
    let mut rewritten = stats.rewritten_commit_ids.iter().collect::<Vec<_>>();
    rewritten.sort();
    for id in rewritten {
        record_detail(
            trace,
            "import_rewritten",
            vec![fetch_trace_attr("old_commit", id.hex())],
        );
    }
}

fn ref_target(target: &RefTarget) -> String {
    if target.has_conflict() {
        let removed = target.removed_ids().map(CommitId::hex).collect::<Vec<_>>();
        let added = target.added_ids().map(CommitId::hex).collect::<Vec<_>>();
        format!(
            "removed=[{}],added=[{}]",
            removed.join(","),
            added.join(",")
        )
    } else {
        target.as_normal().map(CommitId::hex).unwrap_or_default()
    }
}

fn commit_attrs(commit: &Commit) -> Vec<FetchTraceAttr> {
    vec![
        fetch_trace_attr("change", short_change_id(commit)),
        fetch_trace_attr("change_id", commit.change_id().hex()),
        fetch_trace_attr("old_commit", commit.id().hex()),
        fetch_trace_attr("old_parents", commit_ids(commit.parent_ids())),
        fetch_trace_attr("conflict_before", commit.has_conflict()),
    ]
}

fn commit_ids(ids: &[CommitId]) -> String {
    ids.iter().map(CommitId::hex).collect::<Vec<_>>().join(",")
}

fn conflict_path_attrs(commit: &Commit) -> Vec<FetchTraceAttr> {
    const MAX_PATHS: usize = 20;
    let mut paths = Vec::new();
    let mut error = None;
    let mut conflicted_parents = Vec::new();
    for id in commit.parent_ids() {
        match commit.store().get_commit(id) {
            Ok(parent) if parent.has_conflict() => conflicted_parents.push(id.hex()),
            Ok(_) => {}
            Err(source) => error = Some(source.to_string()),
        }
    }
    for (path, value) in commit.tree().conflicts().take(MAX_PATHS + 1) {
        paths.push(path.as_internal_file_string().to_owned());
        if let Err(source) = value {
            error = Some(source.to_string());
        }
    }
    let truncated = paths.len() > MAX_PATHS;
    paths.truncate(MAX_PATHS);
    let mut attrs = vec![
        fetch_trace_attr(
            "conflict_paths",
            serde_json::to_string(&paths).unwrap_or_default(),
        ),
        fetch_trace_attr("conflict_paths_truncated", truncated),
        fetch_trace_attr("conflicted_parents", conflicted_parents.join(",")),
    ];
    if let Some(error) = error {
        attrs.push(fetch_trace_attr("diagnostic_error", error));
    }
    attrs
}
