use super::*;

impl JjWorkspace {
    /// Ensures `branch` points at the selected change as a local jj bookmark.
    pub fn ensure_bookmark(
        &mut self,
        branch: &str,
        target_commit_id: &str,
    ) -> Result<BookmarkUpdate, JjError> {
        self.ensure_git_backed()?;
        let targets = [(branch.to_owned(), target_commit_id.to_owned())];
        self.ensure_bookmarks(&targets)
            .map(|mut updates| updates.remove(0))
    }

    /// Ensures every branch points at its selected change in one jj transaction.
    pub fn ensure_bookmarks(
        &mut self,
        targets: &[(String, String)],
    ) -> Result<Vec<BookmarkUpdate>, JjError> {
        self.ensure_git_backed()?;

        let mut planned = Vec::new();
        let mut updates = Vec::new();
        for (branch, target_commit_id) in targets {
            let target_commit_id = CommitId::try_from_hex(target_commit_id).ok_or_else(|| {
                JjError::InvalidTargetCommitId {
                    commit_id: target_commit_id.clone(),
                }
            })?;
            let target = self.load_commit(&target_commit_id)?;
            let bookmark = RefName::new(branch);
            let existing_target = self.repo.view().get_local_bookmark(bookmark);

            if existing_target.has_conflict() {
                return Err(JjError::ConflictedBookmark {
                    branch: branch.clone(),
                });
            }

            if let Some(existing_id) = existing_target.as_normal() {
                if existing_id == target.id() {
                    updates.push(BookmarkUpdate {
                        branch: branch.clone(),
                        created: false,
                    });
                    continue;
                }

                return Err(JjError::BookmarkExistsOnDifferentChange {
                    branch: branch.clone(),
                });
            }

            updates.push(BookmarkUpdate {
                branch: branch.clone(),
                created: true,
            });
            planned.push((branch.clone(), target.id().clone()));
        }

        if planned.is_empty() {
            return Ok(updates);
        }

        let mut tx = self.repo.start_transaction();
        for (branch, target_id) in &planned {
            tx.repo_mut().set_local_bookmark_target(
                RefName::new(branch),
                RefTarget::normal(target_id.clone()),
            );
        }
        export_git_refs(tx.repo_mut())?;
        let repo = pollster::block_on(tx.commit("jx bookmark stack publish branches")).map_err(
            |error| JjError::Transaction {
                message: error.to_string(),
            },
        )?;
        self.repo = repo;

        Ok(updates)
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
