use super::*;

impl JjWorkspace {
    /// Fetches tracked `origin` refs plus trunk, then rebases mutable pre-fetch trunk children.
    /// Commits whose changes are already present upstream are abandoned so remaining local work
    /// sits directly on the updated trunk.
    pub fn fetch_origin(&mut self) -> Result<FetchOutcome, JjError> {
        self.ensure_git_backed()?;

        let current_before = self.current_commit()?;
        let current_before_tree = current_before.tree();
        let fetch_trunk = self.resolve_fetch_trunk(&current_before)?;
        let trunk_children_before = collect_child_ids(self.repo.as_ref(), fetch_trunk.commit.id())?;

        let mut tx = self.repo.start_transaction();
        let import_stats = fetch_origin_refs(
            tx.repo_mut(),
            &fetch_trunk.branch,
            &fetch_trunk.refresh_bookmarks,
        )?;
        let updated_trunk = load_origin_branch(tx.repo(), &fetch_trunk.branch)?;
        let mut rebase_stats = pollster::block_on(rebase_trunk_children_onto_updated_trunk(
            tx.repo_mut(),
            &trunk_children_before,
            &updated_trunk,
        ))?;

        let repair_stats = pollster::block_on(repair_immutable_working_copy(
            tx.repo_mut(),
            self.workspace.workspace_name().to_owned(),
            current_before.id(),
            fetch_trunk.commit.id(),
            &updated_trunk,
        ))?;
        rebase_stats.rebased_descendants += repair_stats.rebased_descendants;
        export_git_refs(tx.repo_mut())?;
        let final_current_id = tx
            .repo()
            .view()
            .get_wc_commit_id(self.workspace.workspace_name())
            .cloned()
            .ok_or_else(|| JjError::MissingWorkingCopy {
                workspace: self.workspace.workspace_name().as_str().to_owned(),
            })?;

        let repo = pollster::block_on(
            tx.commit(format!("jx fetch {remote}", remote = ORIGIN_REMOTE_NAME)),
        )
        .map_err(|error| JjError::Transaction {
            message: error.to_string(),
        })?;

        let current_repaired = repair_stats.repaired || final_current_id != *current_before.id();
        if current_repaired {
            let final_current = load_commit_from_repo(repo.as_ref(), &final_current_id)?;
            pollster::block_on(self.workspace.check_out(
                repo.op_id().clone(),
                Some(&current_before_tree),
                &final_current,
            ))
            .map_err(|error| JjError::WorkingCopyCheckout {
                message: error.to_string(),
            })?;
        }

        let rebased_commits = rebased_commit_summaries(
            repo.as_ref(),
            rebase_stats.rebased_commits,
            Some(updated_trunk.id()),
            self.workspace.workspace_name(),
        )?;
        self.repo = repo;

        Ok(FetchOutcome {
            branch: fetch_trunk.branch,
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

/// Trunk selection plus extra refs fetch should refresh to prune stale ambiguous candidates.
pub(super) struct FetchTrunkSelection {
    pub(super) branch: String,
    pub(super) commit: Commit,
    pub(super) refresh_bookmarks: Vec<String>,
}

pub(super) async fn rebase_trunk_children_onto_updated_trunk(
    mut_repo: &mut MutableRepo,
    trunk_children_before: &[CommitId],
    updated_trunk: &Commit,
) -> Result<FetchRebaseStats, JjError> {
    let mut stats = FetchRebaseStats::default();
    let options = fetch_rebase_options();

    for child_id in trunk_children_before {
        let child = match mut_repo.store().get_commit(child_id) {
            Ok(child) => child,
            Err(BackendError::ObjectNotFound { .. }) => {
                stats.skipped_trunk_children += 1;
                continue;
            }
            Err(error) => {
                return Err(JjError::Backend {
                    message: error.to_string(),
                });
            }
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
            .rebase_descendants_with_options(&options, |old, rebased| match rebased {
                RebasedCommit::Rewritten(new) => {
                    stats
                        .rebased_commits
                        .push(rebased_commit_record(&old, &new));
                    rebased_descendants += 1;
                }
                RebasedCommit::Abandoned { .. } => {
                    stats.abandoned_empty_commits += 1;
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
