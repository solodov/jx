use super::*;
use std::time::{Duration, Instant};

impl JjWorkspace {
    /// Fetches tracked `origin` refs plus trunk, then rebases mutable pre-fetch trunk children.
    /// Commits whose changes are already present upstream are abandoned so remaining local work
    /// sits directly on the updated trunk.
    pub fn fetch_origin(&mut self) -> Result<FetchOutcome, JjError> {
        let mut trace = |_| {};
        self.fetch_origin_with_trace(&mut trace)
    }

    /// Fetches origin while emitting fetch substeps as they complete.
    pub fn fetch_origin_with_trace(
        &mut self,
        trace: &mut dyn FnMut(FetchTraceStep),
    ) -> Result<FetchOutcome, JjError> {
        self.ensure_git_backed()?;

        let current_before = measure_fetch_step(
            trace,
            "current_commit",
            [
                fetch_trace_attr("workspace", self.workspace.workspace_name().as_str()),
                fetch_trace_attr("op_id_before", self.repo.op_id().hex()),
            ],
            || self.current_commit(),
            |result| match result {
                Ok(commit) => vec![
                    fetch_trace_attr("current_commit", commit.id().hex()),
                    fetch_trace_attr("current_change", commit.change_id().hex()),
                ],
                Err(_) => Vec::new(),
            },
        )?;
        let current_before_tree = current_before.tree();
        let fetch_trunk = measure_fetch_step(
            trace,
            "resolve_fetch_trunk",
            [fetch_trace_attr(
                "current_commit",
                current_before.id().hex(),
            )],
            || self.resolve_fetch_trunk(&current_before),
            |result| match result {
                Ok(selection) => vec![
                    fetch_trace_attr("branch", &selection.branch),
                    fetch_trace_attr("trunk_commit", selection.commit.id().hex()),
                    fetch_trace_attr("refresh_bookmark_count", selection.refresh_bookmarks.len()),
                    fetch_trace_attr(
                        "refresh_bookmarks",
                        joined_fetch_values(&selection.refresh_bookmarks),
                    ),
                ],
                Err(_) => Vec::new(),
            },
        )?;
        let trunk_children_before = measure_fetch_step(
            trace,
            "collect_trunk_children",
            [
                fetch_trace_attr("branch", &fetch_trunk.branch),
                fetch_trace_attr("trunk_commit", fetch_trunk.commit.id().hex()),
            ],
            || collect_trunk_child_changes(self.repo.as_ref(), fetch_trunk.commit.id()),
            |result| match result {
                Ok(children) => vec![fetch_trace_attr("child_count", children.len())],
                Err(_) => Vec::new(),
            },
        )?;
        let immutable_expression = self.fetch_immutable_expression()?;

        let mut tx = self.repo.start_transaction();
        let import_stats = fetch_origin_refs(
            tx.repo_mut(),
            &fetch_trunk.branch,
            &fetch_trunk.refresh_bookmarks,
            trace,
        )?;
        let updated_trunk = measure_fetch_step(
            trace,
            "load_updated_trunk",
            [fetch_trace_attr("branch", &fetch_trunk.branch)],
            || load_origin_branch(tx.repo(), &fetch_trunk.branch),
            |result| match result {
                Ok(commit) => vec![fetch_trace_attr("updated_trunk", commit.id().hex())],
                Err(_) => Vec::new(),
            },
        )?;
        let mut rebase_stats = measure_fetch_step(
            trace,
            "rebase_trunk_children",
            [
                fetch_trace_attr("branch", &fetch_trunk.branch),
                fetch_trace_attr("previous_trunk", fetch_trunk.commit.id().hex()),
                fetch_trace_attr("updated_trunk", updated_trunk.id().hex()),
                fetch_trace_attr("child_count", trunk_children_before.len()),
            ],
            || {
                pollster::block_on(rebase_trunk_child_changes_onto_updated_trunk(
                    tx.repo_mut(),
                    &trunk_children_before,
                    &updated_trunk,
                    &immutable_expression,
                ))
            },
            fetch_rebase_stats_attrs,
        )?;

        let workspace_name = self.workspace.workspace_name().to_owned();
        let repair_stats = measure_fetch_step(
            trace,
            "repair_working_copy",
            [
                fetch_trace_attr("workspace", workspace_name.as_str()),
                fetch_trace_attr("current_before", current_before.id().hex()),
                fetch_trace_attr("previous_trunk", fetch_trunk.commit.id().hex()),
                fetch_trace_attr("updated_trunk", updated_trunk.id().hex()),
            ],
            || {
                pollster::block_on(repair_immutable_working_copy(
                    tx.repo_mut(),
                    workspace_name.clone(),
                    current_before.id(),
                    fetch_trunk.commit.id(),
                    &updated_trunk,
                ))
            },
            |result| match result {
                Ok(stats) => vec![
                    fetch_trace_attr("repaired", stats.repaired),
                    fetch_trace_attr("rebased_descendants", stats.rebased_descendants),
                ],
                Err(_) => Vec::new(),
            },
        )?;
        rebase_stats.rebased_descendants += repair_stats.rebased_descendants;
        measure_fetch_step(
            trace,
            "export_git_refs",
            Vec::new(),
            || export_git_refs(tx.repo_mut()),
            |_| Vec::new(),
        )?;
        let final_current_id = measure_fetch_step(
            trace,
            "resolve_final_current",
            [fetch_trace_attr(
                "workspace",
                self.workspace.workspace_name().as_str(),
            )],
            || {
                tx.repo()
                    .view()
                    .get_wc_commit_id(self.workspace.workspace_name())
                    .cloned()
                    .ok_or_else(|| JjError::MissingWorkingCopy {
                        workspace: self.workspace.workspace_name().as_str().to_owned(),
                    })
            },
            |result| match result {
                Ok(commit_id) => vec![fetch_trace_attr("final_current", commit_id.hex())],
                Err(_) => Vec::new(),
            },
        )?;

        let repo = measure_fetch_step(
            trace,
            "commit_transaction",
            Vec::new(),
            || {
                pollster::block_on(
                    tx.commit(format!("jx fetch {remote}", remote = ORIGIN_REMOTE_NAME)),
                )
                .map_err(|error| JjError::Transaction {
                    message: error.to_string(),
                })
            },
            |result| match result {
                Ok(repo) => vec![fetch_trace_attr("op_id", repo.op_id().hex())],
                Err(_) => Vec::new(),
            },
        )?;

        let current_repaired = repair_stats.repaired || final_current_id != *current_before.id();
        if current_repaired {
            let final_current = measure_fetch_step(
                trace,
                "load_final_current",
                [fetch_trace_attr("final_current", final_current_id.hex())],
                || load_commit_from_repo(repo.as_ref(), &final_current_id),
                |_| Vec::new(),
            )?;
            measure_fetch_step(
                trace,
                "checkout_working_copy",
                [
                    fetch_trace_attr("workspace", self.workspace.workspace_name().as_str()),
                    fetch_trace_attr("current_before", current_before.id().hex()),
                    fetch_trace_attr("final_current", final_current.id().hex()),
                    fetch_trace_attr("op_id", repo.op_id().hex()),
                ],
                || {
                    pollster::block_on(self.workspace.check_out(
                        repo.op_id().clone(),
                        Some(&current_before_tree),
                        &final_current,
                    ))
                    .map_err(|error| JjError::WorkingCopyCheckout {
                        message: error.to_string(),
                    })
                },
                |_| Vec::new(),
            )?;
        }

        let rebased_commits = measure_fetch_step(
            trace,
            "summarize_rebased_commits",
            [fetch_trace_attr(
                "rebased_commit_count",
                rebase_stats.rebased_commits.len(),
            )],
            || {
                rebased_commit_summaries(
                    repo.as_ref(),
                    rebase_stats.rebased_commits,
                    Some(updated_trunk.id()),
                    self.workspace.workspace_name(),
                )
            },
            |result| match result {
                Ok(commits) => vec![fetch_trace_attr("rebased_commit_count", commits.len())],
                Err(_) => Vec::new(),
            },
        )?;
        self.repo = repo;

        Ok(FetchOutcome {
            branch: fetch_trunk.branch.clone(),
            trunk: Some(trunk_state_summary(fetch_trunk.branch, &updated_trunk)),
            changed_remote_bookmarks: import_stats.changed_remote_bookmarks.len(),
            changed_remote_tags: import_stats.changed_remote_tags.len(),
            abandoned_commits: import_stats.abandoned_commits.len()
                + rebase_stats.abandoned_empty_commits,
            rebased_trunk_children: rebase_stats.rebased_trunk_children,
            rebased_descendants: rebase_stats.rebased_descendants,
            skipped_trunk_children: rebase_stats.skipped_trunk_children,
            current_repaired,
            rebased_commits,
        })
    }

    fn fetch_immutable_expression(&self) -> Result<Arc<ResolvedRevsetExpression>, JjError> {
        let ui = Ui::null();
        let settings = self.workspace.settings();
        let fileset_aliases_map =
            load_fileset_aliases(&ui, settings.config()).map_err(log_command_error)?;
        let revset_aliases_map =
            load_revset_aliases(&ui, settings.config()).map_err(log_command_error)?;
        let revset_extensions = Arc::new(RevsetExtensions::default());
        let path_converter = RepoPathUiConverter::Fs {
            cwd: self.workspace.workspace_root().to_path_buf(),
            base: self.workspace.workspace_root().to_path_buf(),
        };
        let workspace_context = RevsetWorkspaceContext {
            path_converter: &path_converter,
            workspace_name: self.workspace.workspace_name(),
        };
        let revset_context = revset_parse_context(
            settings,
            self.repo.as_ref(),
            &fileset_aliases_map,
            &revset_aliases_map,
            &revset_extensions,
            Some(workspace_context),
        )?;
        let expression = immutable_expression(&ui, &revset_context)?;
        let id_prefix_context = IdPrefixContext::new(revset_extensions.clone());
        RevsetExpressionEvaluator::new(
            self.repo.as_ref(),
            revset_extensions,
            &id_prefix_context,
            expression,
        )
        .resolve()
        .map_err(log_error)
    }

    /// Resolves the trunk branch fetch should refresh, using live remote HEAD to break stale local ambiguity.
    pub(super) fn resolve_fetch_trunk(
        &self,
        target: &Commit,
    ) -> Result<FetchTrunkSelection, JjError> {
        self.resolve_fetch_trunk_with_default_branch(target, |remote| {
            live_remote_default_branch(&self.workspace_root(), remote)
        })
    }

    /// Chooses a fetch trunk from cached jj state, with an injectable live-default lookup for tests.
    pub(super) fn resolve_fetch_trunk_with_default_branch(
        &self,
        target: &Commit,
        live_default_branch: impl FnOnce(&str) -> Option<String>,
    ) -> Result<FetchTrunkSelection, JjError> {
        match self.resolve_trunk(target) {
            Ok((branch, commit)) => Ok(FetchTrunkSelection {
                branch,
                commit,
                refresh_bookmarks: Vec::new(),
            }),
            Err(JjError::AmbiguousTrunk { remote, branches }) if remote == ORIGIN_REMOTE_NAME => {
                let Some(default_branch) = live_default_branch(&remote) else {
                    return Err(JjError::AmbiguousTrunk { remote, branches });
                };
                if !branches.iter().any(|branch| branch == &default_branch) {
                    return Err(JjError::AmbiguousTrunk { remote, branches });
                }

                let (branch, commit) = self.resolve_trunk_for_remote_with_hint(
                    target,
                    &remote,
                    Some(&default_branch),
                )?;
                Ok(FetchTrunkSelection {
                    branch,
                    commit,
                    refresh_bookmarks: branches,
                })
            }
            Err(error) => Err(error),
        }
    }
}

fn measure_fetch_step<T>(
    trace: &mut dyn FnMut(FetchTraceStep),
    name: impl Into<String>,
    attrs: impl IntoIterator<Item = FetchTraceAttr>,
    operation: impl FnOnce() -> Result<T, JjError>,
    result_attrs: impl FnOnce(&Result<T, JjError>) -> Vec<FetchTraceAttr>,
) -> Result<T, JjError> {
    let name = name.into();
    let started = Instant::now();
    let result = operation();
    let mut attrs = attrs.into_iter().collect::<Vec<_>>();
    attrs.extend(result_attrs(&result));
    trace(FetchTraceStep {
        name,
        duration_us: fetch_duration_us(started.elapsed()),
        attrs,
        error: result.as_ref().err().map(ToString::to_string),
    });
    result
}

fn fetch_rebase_stats_attrs(result: &Result<FetchRebaseStats, JjError>) -> Vec<FetchTraceAttr> {
    match result {
        Ok(stats) => vec![
            fetch_trace_attr("rebased_trunk_children", stats.rebased_trunk_children),
            fetch_trace_attr("rebased_descendants", stats.rebased_descendants),
            fetch_trace_attr("skipped_trunk_children", stats.skipped_trunk_children),
            fetch_trace_attr("abandoned_empty_commits", stats.abandoned_empty_commits),
            fetch_trace_attr("rebased_commit_count", stats.rebased_commits.len()),
        ],
        Err(_) => Vec::new(),
    }
}

fn joined_fetch_values(values: &[String]) -> String {
    const MAX_VALUES: usize = 20;
    if values.len() <= MAX_VALUES {
        return values.join(",");
    }

    format!(
        "{},…(+{})",
        values[..MAX_VALUES].join(","),
        values.len() - MAX_VALUES
    )
}

fn fetch_duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Trunk selection plus extra refs fetch should refresh to prune stale ambiguous candidates.
pub(super) struct FetchTrunkSelection {
    pub(super) branch: String,
    pub(super) commit: Commit,
    pub(super) refresh_bookmarks: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct TrunkChildChange {
    pub(super) commit_id: CommitId,
    pub(super) change_id: ChangeId,
}

pub(super) fn collect_trunk_child_changes(
    repo: &dyn jj_lib::repo::Repo,
    trunk_id: &CommitId,
) -> Result<Vec<TrunkChildChange>, JjError> {
    collect_child_ids(repo, trunk_id)?
        .into_iter()
        .map(|commit_id| {
            let commit = load_commit_from_repo(repo, &commit_id)?;
            Ok(TrunkChildChange {
                commit_id,
                change_id: commit.change_id().clone(),
            })
        })
        .collect()
}

pub(super) async fn rebase_trunk_child_changes_onto_updated_trunk(
    mut_repo: &mut MutableRepo,
    trunk_children_before: &[TrunkChildChange],
    updated_trunk: &Commit,
    immutable_expression: &Arc<ResolvedRevsetExpression>,
) -> Result<FetchRebaseStats, JjError> {
    let mut stats = FetchRebaseStats::default();
    rebase_import_rewrites(mut_repo, immutable_expression, &mut stats).await?;

    let options = fetch_rebase_options();
    for child_change in trunk_children_before {
        let Some(child) = resolve_visible_trunk_child_change(mut_repo, child_change)? else {
            stats.skipped_trunk_children += 1;
            continue;
        };

        if child.parent_ids().contains(updated_trunk.id())
            || is_ancestor_or_equal_in_repo(mut_repo, child.id(), updated_trunk.id())?
        {
            stats.skipped_trunk_children += 1;
            continue;
        }

        let rebased = rebase_commit_with_options(
            CommitRewriter::new(mut_repo, child.clone(), vec![updated_trunk.id().clone()]),
            &options,
        )
        .await
        .map_err(|error| JjError::Backend {
            message: error.to_string(),
        })?;
        match rebased {
            RebasedCommit::Rewritten(rebased) => {
                stats
                    .rebased_commits
                    .push(rebased_commit_record(&child, &rebased));
                stats.rebased_trunk_children += 1;
            }
            RebasedCommit::Abandoned { .. } => {
                stats.abandoned_empty_commits += 1;
            }
        }
    }

    if mut_repo.has_rewrites() {
        let mut rebased_descendants = 0;
        mut_repo
            .rebase_descendants_with_options(immutable_expression, &options, |old, rebased| {
                match rebased {
                    RebasedCommit::Rewritten(new) => {
                        stats
                            .rebased_commits
                            .push(rebased_commit_record(&old, &new));
                        rebased_descendants += 1;
                    }
                    RebasedCommit::Abandoned { .. } => {
                        stats.abandoned_empty_commits += 1;
                    }
                }
            })
            .await
            .map_err(|error| JjError::Backend {
                message: error.to_string(),
            })?;
        stats.rebased_descendants += rebased_descendants;
    }

    Ok(stats)
}

async fn rebase_import_rewrites(
    mut_repo: &mut MutableRepo,
    immutable_expression: &Arc<ResolvedRevsetExpression>,
    stats: &mut FetchRebaseStats,
) -> Result<(), JjError> {
    if !mut_repo.has_rewrites() {
        return Ok(());
    }

    let mut rebased_descendants = 0;
    mut_repo
        .rebase_descendants_with_options(
            immutable_expression,
            &RebaseOptions::default(),
            |old, rebased| match rebased {
                RebasedCommit::Rewritten(new) => {
                    stats
                        .rebased_commits
                        .push(rebased_commit_record(&old, &new));
                    rebased_descendants += 1;
                }
                RebasedCommit::Abandoned { .. } => {
                    stats.abandoned_empty_commits += 1;
                }
            },
        )
        .await
        .map_err(|error| JjError::Backend {
            message: error.to_string(),
        })?;
    stats.rebased_descendants += rebased_descendants;
    Ok(())
}

fn resolve_visible_trunk_child_change(
    repo: &dyn jj_lib::repo::Repo,
    child: &TrunkChildChange,
) -> Result<Option<Commit>, JjError> {
    let Some(targets) =
        repo.resolve_change_id(&child.change_id)
            .map_err(|error| JjError::Index {
                message: error.to_string(),
            })?
    else {
        return Ok(None);
    };

    let visible = targets
        .visible_with_offsets()
        .map(|(_, commit_id)| commit_id)
        .collect::<Vec<_>>();
    let commit_id = match visible.as_slice() {
        [] => return Ok(None),
        [commit_id] => *commit_id,
        commit_ids => match commit_ids
            .iter()
            .copied()
            .find(|commit_id| *commit_id == &child.commit_id)
        {
            Some(commit_id) => commit_id,
            None => return Ok(None),
        },
    };

    load_commit_from_repo(repo, commit_id).map(Some)
}

pub(super) async fn repair_immutable_working_copy(
    mut_repo: &mut MutableRepo,
    workspace_name: WorkspaceNameBuf,
    previous_current_id: &CommitId,
    previous_trunk_id: &CommitId,
    updated_trunk: &Commit,
) -> Result<WorkingCopyRepairStats, JjError> {
    if previous_current_id == updated_trunk.id()
        || !is_ancestor_or_equal_in_repo(mut_repo, previous_current_id, updated_trunk.id())?
    {
        return Ok(WorkingCopyRepairStats::default());
    }

    let current_id = mut_repo
        .view()
        .get_wc_commit_id(&workspace_name)
        .cloned()
        .ok_or_else(|| JjError::MissingWorkingCopy {
            workspace: workspace_name.as_str().to_owned(),
        })?;
    let current = load_commit_from_repo(mut_repo, &current_id)?;

    if current_id == *previous_current_id
        || previous_current_id == previous_trunk_id
        || current.id() == updated_trunk.id()
    {
        mut_repo
            .check_out(workspace_name, updated_trunk)
            .await
            .map_err(|error| JjError::WorkingCopyCheckout {
                message: error.to_string(),
            })?;
        return Ok(WorkingCopyRepairStats {
            repaired: true,
            rebased_descendants: 0,
        });
    }

    if current.parent_ids().contains(updated_trunk.id()) {
        return Ok(WorkingCopyRepairStats {
            repaired: current_id != *previous_current_id,
            rebased_descendants: 0,
        });
    }

    jj_lib::rewrite::rebase_commit(mut_repo, current, vec![updated_trunk.id().clone()])
        .await
        .map_err(|error| JjError::Backend {
            message: error.to_string(),
        })?;
    let rebased_descendants =
        mut_repo
            .rebase_descendants()
            .await
            .map_err(|error| JjError::Backend {
                message: error.to_string(),
            })?;

    Ok(WorkingCopyRepairStats {
        repaired: true,
        rebased_descendants,
    })
}

pub(super) fn fetch_rebase_options() -> RebaseOptions {
    RebaseOptions {
        empty: EmptyBehavior::AbandonNewlyEmpty,
        rewrite_refs: RewriteRefsOptions {
            delete_abandoned_bookmarks: false,
        },
        simplify_ancestor_merge: false,
    }
}

pub(super) fn rebased_commit_record(old: &Commit, new: &Commit) -> RebasedCommitRecord {
    RebasedCommitRecord {
        short_change_id: short_change_id(new),
        old_short_commit_id: short_commit_id(old.id()),
        new_commit_id: new.id().hex(),
        new_short_commit_id: short_commit_id(new.id()),
        description: first_description_line(new.description()).to_owned(),
        has_conflict: new.has_conflict(),
    }
}

pub(super) fn rebased_commit_summaries(
    repo: &dyn jj_lib::repo::Repo,
    records: Vec<RebasedCommitRecord>,
    trunk_id: Option<&CommitId>,
    current_workspace: &WorkspaceName,
) -> Result<Vec<RebasedCommitSummary>, JjError> {
    records
        .into_iter()
        .map(|record| {
            let commit_id = CommitId::try_from_hex(&record.new_commit_id).ok_or_else(|| {
                JjError::InvalidTargetCommitId {
                    commit_id: record.new_commit_id.clone(),
                }
            })?;
            let commit = load_commit_from_repo(repo, &commit_id)?;
            let is_empty =
                pollster::block_on(commit.is_empty(repo)).map_err(|error| JjError::Backend {
                    message: error.to_string(),
                })?;
            let workspace_visibility =
                commit_workspace_visibility(repo, Some(&commit_id), trunk_id, current_workspace)?;

            Ok(RebasedCommitSummary {
                short_change_id: record.short_change_id,
                old_short_commit_id: record.old_short_commit_id,
                new_short_commit_id: record.new_short_commit_id,
                description: record.description,
                has_conflict: record.has_conflict,
                is_empty,
                workspace_visibility,
            })
        })
        .collect()
}

#[derive(Debug, Default)]
pub(super) struct FetchRebaseStats {
    pub(super) rebased_trunk_children: usize,
    pub(super) rebased_descendants: usize,
    pub(super) skipped_trunk_children: usize,
    pub(super) abandoned_empty_commits: usize,
    pub(super) rebased_commits: Vec<RebasedCommitRecord>,
}

#[derive(Debug)]
pub(super) struct RebasedCommitRecord {
    pub(super) short_change_id: String,
    pub(super) old_short_commit_id: String,
    pub(super) new_commit_id: String,
    pub(super) new_short_commit_id: String,
    pub(super) description: String,
    pub(super) has_conflict: bool,
}

#[derive(Debug, Default)]
pub(super) struct WorkingCopyRepairStats {
    pub(super) repaired: bool,
    pub(super) rebased_descendants: usize,
}
