use super::*;

impl JjWorkspace {
    /// Ensures `branch` points at the selected change as a local jj bookmark.
    pub fn ensure_bookmark(
        &mut self,
        branch: &str,
        target_commit_id: &str,
    ) -> Result<BookmarkUpdate, JjError> {
        self.ensure_git_backed()?;

        let target_commit_id = CommitId::try_from_hex(target_commit_id).ok_or_else(|| {
            JjError::InvalidTargetCommitId {
                commit_id: target_commit_id.to_owned(),
            }
        })?;
        let target = self.load_commit(&target_commit_id)?;
        let bookmark = RefName::new(branch);
        let existing_target = self.repo.view().get_local_bookmark(bookmark);

        if existing_target.has_conflict() {
            return Err(JjError::ConflictedBookmark {
                branch: branch.to_owned(),
            });
        }

        if let Some(existing_id) = existing_target.as_normal() {
            if existing_id == target.id() {
                return Ok(BookmarkUpdate {
                    branch: branch.to_owned(),
                    created: false,
                });
            }

            return Err(JjError::BookmarkExistsOnDifferentChange {
                branch: branch.to_owned(),
            });
        }

        let mut tx = self.repo.start_transaction();
        tx.repo_mut()
            .set_local_bookmark_target(bookmark, RefTarget::normal(target.id().clone()));
        export_git_refs(tx.repo_mut())?;
        let repo =
            pollster::block_on(tx.commit(format!("jx bookmark {branch}"))).map_err(|error| {
                JjError::Transaction {
                    message: error.to_string(),
                }
            })?;
        self.repo = repo;

        Ok(BookmarkUpdate {
            branch: branch.to_owned(),
            created: true,
        })
    }

    pub(super) fn local_and_origin_bookmark_targets(
        &self,
        bookmark: &RefName,
    ) -> LocalAndRemoteRef<'_> {
        LocalAndRemoteRef {
            local_target: self.repo.view().get_local_bookmark(bookmark),
            remote_ref: self.repo.view().get_remote_bookmark(
                bookmark.to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME)),
            ),
        }
    }
}
