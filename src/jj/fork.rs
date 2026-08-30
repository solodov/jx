use super::*;

impl JjWorkspace {
    /// Plans how to align a local fork branch with a fetched upstream branch.
    pub fn fork_sync_branch_plan(
        &self,
        branch: &str,
        upstream_remote: &str,
        upstream_branch: &str,
    ) -> Result<ForkSyncBranchPlan, JjError> {
        self.ensure_git_backed()?;
        let branch_ref = RefName::new(branch);
        let local_target = self.repo.view().get_local_bookmark(branch_ref);
        if local_target.has_conflict() {
            return Err(JjError::ConflictedBookmark {
                branch: branch.to_owned(),
            });
        }
        let Some(local_commit_id) = local_target.as_normal().cloned() else {
            return Err(JjError::MissingLocalBookmark {
                branch: branch.to_owned(),
            });
        };

        let upstream_target = self.remote_bookmark_target(upstream_remote, upstream_branch)?;
        let origin_remote = ORIGIN_REMOTE_NAME.to_owned();
        let origin_ref = self
            .repo
            .view()
            .get_remote_bookmark(branch_ref.to_remote_symbol(RemoteName::new(&origin_remote)));
        if origin_ref.target.has_conflict() {
            return Err(JjError::ConflictedRemoteBookmark {
                branch: branch.to_owned(),
                remote: origin_remote,
            });
        }
        let origin_commit_id = origin_ref.target.as_normal().cloned();
        let push_needed = !origin_ref.is_tracked()
            || origin_commit_id
                .as_ref()
                .is_none_or(|origin_commit_id| origin_commit_id != &local_commit_id);

        let operation = self.fork_sync_branch_operation(
            branch,
            upstream_remote,
            upstream_branch,
            &local_commit_id,
            &upstream_target,
        )?;

        Ok(ForkSyncBranchPlan {
            branch: branch.to_owned(),
            origin_remote: ORIGIN_REMOTE_NAME.to_owned(),
            upstream_remote: upstream_remote.to_owned(),
            upstream_branch: upstream_branch.to_owned(),
            local_short_commit_id: short_commit_id(&local_commit_id),
            local_commit_id: local_commit_id.hex(),
            upstream_short_commit_id: short_commit_id(&upstream_target),
            upstream_commit_id: upstream_target.hex(),
            origin_short_commit_id: origin_commit_id.as_ref().map(short_commit_id),
            origin_commit_id: origin_commit_id.map(|commit_id| commit_id.hex()),
            push_needed,
            operation,
        })
    }

    /// Applies a planned local fork branch alignment.
    pub fn apply_fork_sync_branch_plan(
        &mut self,
        plan: &ForkSyncBranchPlan,
    ) -> Result<ForkSyncBranchOutcome, JjError> {
        self.ensure_git_backed()?;
        let old_commit_id = commit_id_from_hex(&plan.local_commit_id)?;
        let upstream_commit_id = commit_id_from_hex(&plan.upstream_commit_id)?;
        let upstream_commit = self.load_commit(&upstream_commit_id)?;

        match &plan.operation {
            ForkSyncBranchOperation::AlreadySynced => Ok(ForkSyncBranchOutcome {
                branch: plan.branch.clone(),
                origin_remote: plan.origin_remote.clone(),
                upstream_remote: plan.upstream_remote.clone(),
                upstream_branch: plan.upstream_branch.clone(),
                old_short_commit_id: plan.local_short_commit_id.clone(),
                new_short_commit_id: plan.local_short_commit_id.clone(),
                operation: ForkSyncBranchOutcomeKind::AlreadySynced,
                rebased_commits: Vec::new(),
                abandoned_commits: 0,
                skipped_commits: 0,
                current_updated: false,
            }),
            ForkSyncBranchOperation::FastForward => self.fast_forward_fork_branch(
                plan,
                old_commit_id,
                upstream_commit_id,
                upstream_commit,
            ),
            ForkSyncBranchOperation::Rebase {
                root_commit_id,
                root_short_change_id,
                commit_count,
            } => {
                let root_commit_id = commit_id_from_hex(root_commit_id)?;
                self.rebase_fork_branch_stack(
                    plan,
                    root_commit_id,
                    root_short_change_id.clone(),
                    *commit_count,
                    upstream_commit,
                )
            }
        }
    }

    fn remote_bookmark_target(&self, remote: &str, branch: &str) -> Result<CommitId, JjError> {
        let remote_ref = self
            .repo
            .view()
            .get_remote_bookmark(RefName::new(branch).to_remote_symbol(RemoteName::new(remote)));
        if remote_ref.target.has_conflict() {
            return Err(JjError::ConflictedRemoteBookmark {
                branch: branch.to_owned(),
                remote: remote.to_owned(),
            });
        }
        remote_ref
            .target
            .as_normal()
            .cloned()
            .ok_or_else(|| JjError::MissingRemoteBookmark {
                branch: branch.to_owned(),
                remote: remote.to_owned(),
            })
    }

    fn fork_sync_branch_operation(
        &self,
        branch: &str,
        upstream_remote: &str,
        upstream_branch: &str,
        local_commit_id: &CommitId,
        upstream_commit_id: &CommitId,
    ) -> Result<ForkSyncBranchOperation, JjError> {
        if local_commit_id == upstream_commit_id {
            return Ok(ForkSyncBranchOperation::AlreadySynced);
        }

        let stack_expression = ResolvedRevsetExpression::commit(local_commit_id.clone())
            .ancestors()
            .minus(&ResolvedRevsetExpression::commit(upstream_commit_id.clone()).ancestors());
        let stack_commit_ids = self.evaluate_commit_ids(stack_expression.clone())?;
        if stack_commit_ids.is_empty() {
            return Ok(ForkSyncBranchOperation::FastForward);
        }

        let roots = self.evaluate_commit_ids(stack_expression.roots())?;
        if roots.len() != 1 {
            return Err(JjError::MultipleForkStackRoots {
                branch: branch.to_owned(),
                upstream: format!("{upstream_branch}@{upstream_remote}"),
                roots: roots.iter().map(short_commit_id).collect(),
            });
        }
        let root = self.load_commit(&roots[0])?;

        Ok(ForkSyncBranchOperation::Rebase {
            root_commit_id: root.id().hex(),
            root_short_change_id: short_change_id(&root),
            commit_count: stack_commit_ids.len(),
        })
    }

    fn evaluate_commit_ids(
        &self,
        expression: Arc<ResolvedRevsetExpression>,
    ) -> Result<Vec<CommitId>, JjError> {
        let revset = expression
            .evaluate(self.repo.as_ref())
            .map_err(|error| JjError::Backend {
                message: error.into_backend_error().to_string(),
            })?;
        pollster::block_on(revset.stream().try_collect::<Vec<_>>()).map_err(|error| {
            JjError::Backend {
                message: error.into_backend_error().to_string(),
            }
        })
    }

    fn fast_forward_fork_branch(
        &mut self,
        plan: &ForkSyncBranchPlan,
        old_commit_id: CommitId,
        upstream_commit_id: CommitId,
        upstream_commit: Commit,
    ) -> Result<ForkSyncBranchOutcome, JjError> {
        let child_roots = self.fast_forward_child_roots(&old_commit_id, &upstream_commit_id)?;
        let update = ForkSyncBranchUpdate::new(self)?;
        let mut tx = self.repo.start_transaction();
        tx.repo_mut().set_local_bookmark_target(
            RefName::new(&plan.branch),
            RefTarget::normal(upstream_commit_id.clone()),
        );
        let move_stats = move_roots_onto(tx.repo_mut(), child_roots, upstream_commit.id().clone())?;
        finish_fork_sync_update(
            self,
            tx,
            update,
            plan,
            ForkSyncBranchOutcomeKind::FastForward,
            move_stats,
            &upstream_commit,
        )
    }

    fn fast_forward_child_roots(
        &self,
        old_commit_id: &CommitId,
        upstream_commit_id: &CommitId,
    ) -> Result<Vec<CommitId>, JjError> {
        collect_child_ids(self.repo.as_ref(), old_commit_id)?
            .into_iter()
            .filter_map(
                |child_id| match self.is_ancestor_or_equal(&child_id, upstream_commit_id) {
                    Ok(true) => None,
                    Ok(false) => Some(Ok(child_id)),
                    Err(error) => Some(Err(error)),
                },
            )
            .collect()
    }

    fn rebase_fork_branch_stack(
        &mut self,
        plan: &ForkSyncBranchPlan,
        root_commit_id: CommitId,
        root_short_change_id: String,
        commit_count: usize,
        upstream_commit: Commit,
    ) -> Result<ForkSyncBranchOutcome, JjError> {
        let update = ForkSyncBranchUpdate::new(self)?;
        let mut tx = self.repo.start_transaction();
        let move_stats = move_roots_onto(
            tx.repo_mut(),
            vec![root_commit_id],
            upstream_commit.id().clone(),
        )?;
        finish_fork_sync_update(
            self,
            tx,
            update,
            plan,
            ForkSyncBranchOutcomeKind::Rebased {
                root_short_change_id,
                commit_count,
            },
            move_stats,
            &upstream_commit,
        )
    }
}

struct ForkSyncBranchUpdate {
    current_before: Commit,
    current_before_tree: jj_lib::merged_tree::MergedTree,
    workspace_name: WorkspaceNameBuf,
}

impl ForkSyncBranchUpdate {
    fn new(workspace: &JjWorkspace) -> Result<Self, JjError> {
        let current_before = workspace.current_commit()?;
        let current_before_tree = current_before.tree();
        Ok(Self {
            current_before,
            current_before_tree,
            workspace_name: workspace.workspace.workspace_name().to_owned(),
        })
    }
}

struct ForkSyncMoveStats {
    rebased_commits: Vec<super::fetch::RebasedCommitRecord>,
    abandoned_commits: usize,
    skipped_commits: usize,
}

fn move_roots_onto(
    mut_repo: &mut MutableRepo,
    root_ids: Vec<CommitId>,
    destination_id: CommitId,
) -> Result<ForkSyncMoveStats, JjError> {
    if root_ids.is_empty() {
        return Ok(ForkSyncMoveStats {
            rebased_commits: Vec::new(),
            abandoned_commits: 0,
            skipped_commits: 0,
        });
    }

    let location = MoveCommitsLocation {
        new_parent_ids: vec![destination_id],
        new_child_ids: Vec::new(),
        target: MoveCommitsTarget::Roots(root_ids),
    };
    let options = RebaseOptions {
        empty: EmptyBehavior::Keep,
        rewrite_refs: RewriteRefsOptions {
            delete_abandoned_bookmarks: false,
        },
        simplify_ancestor_merge: false,
    };
    let stats = pollster::block_on(async {
        let stats = compute_move_commits(mut_repo, &location)
            .await?
            .apply(mut_repo, &options)
            .await?;
        let _remaining_descendants = mut_repo.rebase_descendants().await?;
        Ok::<_, BackendError>(stats)
    })
    .map_err(|error| JjError::Backend {
        message: error.to_string(),
    })?;
    let mut rebased_commits = fork_sync_rebased_commit_records(stats.rebased_commits);
    rebased_commits.sort_by(|left, right| {
        (&left.short_change_id, &left.new_commit_id)
            .cmp(&(&right.short_change_id, &right.new_commit_id))
    });

    Ok(ForkSyncMoveStats {
        rebased_commits,
        abandoned_commits: stats.num_abandoned_empty as usize,
        skipped_commits: stats.num_skipped_rebases as usize,
    })
}

fn fork_sync_rebased_commit_records(
    rebased: HashMap<CommitId, RebasedCommit>,
) -> Vec<super::fetch::RebasedCommitRecord> {
    rebased
        .into_iter()
        .filter_map(|(old_id, rebased)| match rebased {
            RebasedCommit::Rewritten(new) => Some(super::fetch::RebasedCommitRecord {
                short_change_id: short_change_id(&new),
                old_short_commit_id: short_commit_id(&old_id),
                new_commit_id: new.id().hex(),
                new_short_commit_id: short_commit_id(new.id()),
                description: fork_sync_description_line(new.description()).to_owned(),
                has_conflict: new.has_conflict(),
            }),
            RebasedCommit::Abandoned { .. } => None,
        })
        .collect()
}

fn finish_fork_sync_update(
    workspace: &mut JjWorkspace,
    mut tx: jj_lib::transaction::Transaction,
    update: ForkSyncBranchUpdate,
    plan: &ForkSyncBranchPlan,
    operation: ForkSyncBranchOutcomeKind,
    move_stats: ForkSyncMoveStats,
    upstream_commit: &Commit,
) -> Result<ForkSyncBranchOutcome, JjError> {
    export_git_refs(tx.repo_mut())?;
    let final_current_id = tx
        .repo()
        .view()
        .get_wc_commit_id(&update.workspace_name)
        .cloned()
        .ok_or_else(|| JjError::MissingWorkingCopy {
            workspace: update.workspace_name.as_str().to_owned(),
        })?;
    let repo = pollster::block_on(tx.commit(fork_sync_transaction_description(plan))).map_err(
        |error| JjError::Transaction {
            message: error.to_string(),
        },
    )?;
    let current_updated = final_current_id != *update.current_before.id();
    if current_updated {
        let final_current = load_commit_from_repo(repo.as_ref(), &final_current_id)?;
        pollster::block_on(workspace.workspace.check_out(
            repo.op_id().clone(),
            Some(&update.current_before_tree),
            &final_current,
        ))
        .map_err(|error| JjError::WorkingCopyCheckout {
            message: error.to_string(),
        })?;
    }

    let new_branch_target = repo
        .view()
        .get_local_bookmark(RefName::new(&plan.branch))
        .as_normal()
        .cloned()
        .ok_or_else(|| JjError::MissingLocalBookmark {
            branch: plan.branch.clone(),
        })?;
    let rebased_commits = super::fetch::rebased_commit_summaries(
        repo.as_ref(),
        move_stats.rebased_commits,
        Some(upstream_commit.id()),
        &update.workspace_name,
    )?;
    workspace.repo = repo;

    Ok(ForkSyncBranchOutcome {
        branch: plan.branch.clone(),
        origin_remote: plan.origin_remote.clone(),
        upstream_remote: plan.upstream_remote.clone(),
        upstream_branch: plan.upstream_branch.clone(),
        old_short_commit_id: plan.local_short_commit_id.clone(),
        new_short_commit_id: short_commit_id(&new_branch_target),
        operation,
        rebased_commits,
        abandoned_commits: move_stats.abandoned_commits,
        skipped_commits: move_stats.skipped_commits,
        current_updated,
    })
}

fn fork_sync_transaction_description(plan: &ForkSyncBranchPlan) -> String {
    match &plan.operation {
        ForkSyncBranchOperation::AlreadySynced => format!("jx fork sync {}", plan.branch),
        ForkSyncBranchOperation::FastForward => format!(
            "jx fork sync fast-forward {} to {}@{}",
            plan.branch, plan.upstream_branch, plan.upstream_remote
        ),
        ForkSyncBranchOperation::Rebase { .. } => format!(
            "jx fork sync rebase {} onto {}@{}",
            plan.branch, plan.upstream_branch, plan.upstream_remote
        ),
    }
}

fn commit_id_from_hex(value: &str) -> Result<CommitId, JjError> {
    CommitId::try_from_hex(value).ok_or_else(|| JjError::Backend {
        message: format!("internal fork sync commit id `{value}` is invalid"),
    })
}

fn fork_sync_description_line(description: &str) -> &str {
    description.lines().next().unwrap_or("(no description)")
}
