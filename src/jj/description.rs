use super::*;

impl JjWorkspace {
    /// Rewrites one selected commit description and rebases descendants onto it.
    pub fn rewrite_commit_description(
        &mut self,
        target_commit_id: &str,
        description: &str,
    ) -> Result<CommitDescriptionRewrite, JjError> {
        self.ensure_git_backed()?;

        let target_commit_id = CommitId::try_from_hex(target_commit_id).ok_or_else(|| {
            JjError::InvalidTargetCommitId {
                commit_id: target_commit_id.to_owned(),
            }
        })?;
        let target = self.load_commit(&target_commit_id)?;
        if target.description() == description {
            return Ok(CommitDescriptionRewrite {
                commit_id: target.id().hex(),
                changed: false,
            });
        }

        let current_before = self.current_commit()?;
        let current_before_tree = current_before.tree();
        let workspace_name = self.workspace.workspace_name().to_owned();

        let mut tx = self.repo.start_transaction();
        let rewritten = pollster::block_on(
            tx.repo_mut()
                .rewrite_commit(&target)
                .set_description(description)
                .write(),
        )
        .map_err(|error| JjError::Backend {
            message: error.to_string(),
        })?;
        pollster::block_on(tx.repo_mut().rebase_descendants()).map_err(|error| {
            JjError::Backend {
                message: error.to_string(),
            }
        })?;
        export_git_refs(tx.repo_mut())?;
        let final_current_id = tx
            .repo()
            .view()
            .get_wc_commit_id(&workspace_name)
            .cloned()
            .ok_or_else(|| JjError::MissingWorkingCopy {
                workspace: workspace_name.as_str().to_owned(),
            })?;
        let repo =
            pollster::block_on(tx.commit("jx update commit description")).map_err(|error| {
                JjError::Transaction {
                    message: error.to_string(),
                }
            })?;

        if final_current_id != *current_before.id() {
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

        Ok(CommitDescriptionRewrite {
            commit_id: rewritten.id().hex(),
            changed: true,
        })
    }
}
