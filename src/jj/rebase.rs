use super::*;

impl JjWorkspace {
    /// Moves the local trunk bookmark to the latest publishable stack commit.
    pub fn advance_trunk_for_sync(&mut self) -> Result<AdvanceTrunkOutcome, JjError> {
        self.ensure_git_backed()?;

        let current_before = self.current_commit()?;
        let current_before_tree = current_before.tree();
        let (branch, trunk) = self.resolve_trunk(&current_before)?;
        let (target, should_create_empty_child) =
            self.sync_advance_target(&trunk, &current_before)?;
        let bookmark = RefName::new(&branch);
        let remote_ref = self
            .repo
            .view()
            .get_remote_bookmark(bookmark.to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME)));
        if !remote_ref.is_tracked() {
            return Err(JjError::NonTrackingRemoteBookmark {
                branch: branch.clone(),
                remote: ORIGIN_REMOTE_NAME,
            });
        }

        let local_target = self.repo.view().get_local_bookmark(bookmark);
        if local_target.has_conflict() {
            return Err(JjError::ConflictedBookmark {
                branch: branch.clone(),
            });
        }
        let Some(old_id) = local_target.as_normal().cloned() else {
            return Err(JjError::MissingLocalBookmark {
                branch: branch.clone(),
            });
        };
        if !self.is_ancestor_or_equal(&old_id, target.id())? {
            return Err(JjError::TrunkBookmarkOutsideStack {
                branch: branch.clone(),
            });
        }

        let bookmark_needs_update = old_id != *target.id();
        if !bookmark_needs_update && !should_create_empty_child {
            return Ok(AdvanceTrunkOutcome {
                branch,
                old_short_commit_id: short_commit_id(&old_id),
                new_short_commit_id: short_commit_id(target.id()),
                current_updated: false,
            });
        }

        let workspace_name = self.workspace.workspace_name().to_owned();
        let mut tx = self.repo.start_transaction();
        if bookmark_needs_update {
            tx.repo_mut()
                .set_local_bookmark_target(bookmark, RefTarget::normal(target.id().clone()));
        }
        if should_create_empty_child {
            pollster::block_on(tx.repo_mut().check_out(workspace_name.clone(), &target)).map_err(
                |error| JjError::WorkingCopyCheckout {
                    message: error.to_string(),
                },
            )?;
        }
        export_git_refs(tx.repo_mut())?;
        let final_current_id = should_create_empty_child
            .then(|| {
                tx.repo()
                    .view()
                    .get_wc_commit_id(&workspace_name)
                    .cloned()
                    .ok_or_else(|| JjError::MissingWorkingCopy {
                        workspace: workspace_name.as_str().to_owned(),
                    })
            })
            .transpose()?;
        let repo = pollster::block_on(tx.commit(format!("jx sync advance {branch}"))).map_err(
            |error| JjError::Transaction {
                message: error.to_string(),
            },
        )?;

        if let Some(final_current_id) = final_current_id {
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

        self.repo = repo;
        Ok(AdvanceTrunkOutcome {
            branch,
            old_short_commit_id: short_commit_id(&old_id),
            new_short_commit_id: short_commit_id(target.id()),
            current_updated: should_create_empty_child,
        })
    }

    /// Rebases selected source revisions and descendants onto the fixed origin trunk.
    pub fn rebase_on_trunk(
        &mut self,
        source_revisions: &[String],
    ) -> Result<RebaseOnTrunkOutcome, JjError> {
        self.ensure_git_backed()?;

        let sources = if source_revisions.is_empty() {
            vec![self.current_commit()?]
        } else {
            source_revisions
                .iter()
                .map(|revision| {
                    self.resolve_single_revision(revision, "In `jx rebase-on-trunk --source`")
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let (branch, trunk) = self.resolve_trunk_destination()?;
        let source_short_commit_ids = sources
            .iter()
            .map(|source| short_commit_id(source.id()))
            .collect::<Vec<_>>();
        let trunk_short_commit_id = short_commit_id(trunk.id());
        let mut source_ids = Vec::new();
        let mut skipped_commits = 0;
        for source in &sources {
            if self.is_ancestor_or_equal(source.id(), trunk.id())? {
                skipped_commits += 1;
            } else {
                source_ids.push(source.id().clone());
            }
        }

        if source_ids.is_empty() {
            return Ok(RebaseOnTrunkOutcome {
                branch,
                source_short_commit_ids,
                trunk_short_commit_id,
                rebased_commits: 0,
                skipped_commits,
                current_updated: false,
            });
        }

        let current_before = self.current_commit()?;
        let current_before_tree = current_before.tree();
        let workspace_name = self.workspace.workspace_name().to_owned();
        let mut tx = self.repo.start_transaction();
        let location = MoveCommitsLocation {
            new_parent_ids: vec![trunk.id().clone()],
            new_child_ids: Vec::new(),
            target: MoveCommitsTarget::Roots(source_ids.clone()),
        };
        let options = RebaseOptions {
            empty: EmptyBehavior::Keep,
            rewrite_refs: RewriteRefsOptions {
                delete_abandoned_bookmarks: false,
            },
            simplify_ancestor_merge: false,
        };
        let (stats, remaining_descendants) = pollster::block_on(async {
            let stats = compute_move_commits(tx.repo(), &location)
                .await?
                .apply(tx.repo_mut(), &options)
                .await?;
            let remaining_descendants = tx.repo_mut().rebase_descendants().await?;
            Ok::<_, BackendError>((stats, remaining_descendants))
        })
        .map_err(|error| JjError::Backend {
            message: error.to_string(),
        })?;
        let rebased_commits = (stats.num_rebased_targets + stats.num_rebased_descendants) as usize
            + remaining_descendants;
        skipped_commits += stats.num_skipped_rebases as usize;

        if rebased_commits == 0 && stats.num_abandoned_empty == 0 {
            return Ok(RebaseOnTrunkOutcome {
                branch,
                source_short_commit_ids,
                trunk_short_commit_id,
                rebased_commits,
                skipped_commits,
                current_updated: false,
            });
        }

        export_git_refs(tx.repo_mut())?;
        let final_current_id = tx
            .repo()
            .view()
            .get_wc_commit_id(&workspace_name)
            .cloned()
            .ok_or_else(|| JjError::MissingWorkingCopy {
                workspace: workspace_name.as_str().to_owned(),
            })?;
        let repo = pollster::block_on(tx.commit(format!(
            "jx rebase-on-trunk {} onto {}/{}",
            source_ids
                .iter()
                .map(|source| source.hex())
                .collect::<Vec<_>>()
                .join(","),
            ORIGIN_REMOTE_NAME,
            branch
        )))
        .map_err(|error| JjError::Transaction {
            message: error.to_string(),
        })?;
        let current_updated = final_current_id != *current_before.id();
        if current_updated {
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

        self.repo = repo;

        Ok(RebaseOnTrunkOutcome {
            branch,
            source_short_commit_ids,
            trunk_short_commit_id,
            rebased_commits,
            skipped_commits,
            current_updated,
        })
    }

    /// Selects the newest contiguous stack commit that is complete enough to publish.
    ///
    /// A sync-published trunk commit must carry file changes, a non-empty
    /// description, and no conflicts. Incomplete or conflicted commits above that
    /// point remain local so sync does not publish unresolved working-copy state.
    pub(super) fn sync_advance_target(
        &self,
        trunk: &Commit,
        current: &Commit,
    ) -> Result<(Commit, bool), JjError> {
        let stack_path = self.linear_stack_path(trunk, current)?;
        let mut target = trunk.clone();
        for commit in stack_path {
            if !self.sync_advance_commit_is_publishable(&commit)? {
                break;
            }
            target = commit;
        }

        let should_create_empty_child = target.id() == current.id();
        Ok((target, should_create_empty_child))
    }

    /// Returns whether a stack commit is complete enough to become trunk during sync.
    fn sync_advance_commit_is_publishable(&self, commit: &Commit) -> Result<bool, JjError> {
        if commit.has_conflict() || commit.description().trim().is_empty() {
            return Ok(false);
        }

        let is_empty =
            pollster::block_on(commit.is_empty(self.repo.as_ref())).map_err(|error| {
                JjError::Backend {
                    message: error.to_string(),
                }
            })?;

        Ok(!is_empty)
    }
}
