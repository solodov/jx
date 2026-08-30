use super::*;

impl JjWorkspace {
    /// Moves the local trunk bookmark to the latest publishable stack commit.
    pub fn advance_trunk_for_sync(&mut self) -> Result<AdvanceTrunkOutcome, JjError> {
        self.ensure_git_backed()?;

        let current_before = self.current_commit()?;
        let (branch, trunk) = self.resolve_unanchored_origin_trunk()?;
        let bookmark = RefName::new(&branch);
        let remote_ref = self
            .repo
            .view()
            .get_remote_bookmark(bookmark.to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME)));
        if !remote_ref.is_tracked() {
            return Err(JjError::NonTrackingRemoteBookmark {
                branch: branch.clone(),
                remote: ORIGIN_REMOTE_NAME.to_owned(),
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
        if !self.is_ancestor_or_equal(trunk.id(), current_before.id())? {
            return Ok(AdvanceTrunkOutcome {
                branch,
                old_short_commit_id: short_commit_id(&old_id),
                new_short_commit_id: short_commit_id(&old_id),
                trunk: None,
                current_updated: false,
            });
        }

        let current_before_tree = current_before.tree();
        let (target, should_create_empty_child) =
            self.sync_advance_target(&trunk, &current_before)?;
        if !self.is_ancestor_or_equal(&old_id, target.id())? {
            return Err(JjError::TrunkBookmarkOutsideStack {
                branch: branch.clone(),
            });
        }

        let bookmark_needs_update = old_id != *target.id();
        if !bookmark_needs_update && !should_create_empty_child {
            return Ok(AdvanceTrunkOutcome {
                branch: branch.clone(),
                old_short_commit_id: short_commit_id(&old_id),
                new_short_commit_id: short_commit_id(target.id()),
                trunk: Some(trunk_state_summary(branch, &target)),
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
            branch: branch.clone(),
            old_short_commit_id: short_commit_id(&old_id),
            new_short_commit_id: short_commit_id(target.id()),
            trunk: Some(trunk_state_summary(branch, &target)),
            current_updated: should_create_empty_child,
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
