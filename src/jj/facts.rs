use super::*;

impl JjWorkspace {
    /// Returns read-side facts for the current working-copy change.
    pub fn facts(&self) -> Result<WorkspaceFacts, JjError> {
        self.facts_for_revision(None)
    }

    /// Returns local cached trunk facts for each configured remote status should report.
    pub fn status_facts<'a>(
        &self,
        remote_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<StatusWorkspaceFacts, JjError> {
        self.ensure_git_backed()?;
        let target = self.current_commit()?;
        let mut remotes = Vec::new();

        for remote in remote_names {
            let (branch, trunk) = self.resolve_trunk_for_remote(&target, remote)?;
            let stack_path = self.linear_stack_path(&trunk, &target)?;
            let local_ahead_by = self.non_empty_commit_count(&stack_path)?;
            remotes.push(StatusRemoteFacts {
                remote: remote.to_owned(),
                branch,
                trunk_git_commit_sha: trunk.id().hex(),
                trunk_short_commit_id: short_commit_id(trunk.id()),
                local_ahead_by,
            });
        }

        Ok(StatusWorkspaceFacts { remotes })
    }

    fn non_empty_commit_count(&self, commits: &[Commit]) -> Result<i64, JjError> {
        let mut count = 0;
        for commit in commits {
            let is_empty =
                pollster::block_on(commit.is_empty(self.repo.as_ref())).map_err(|error| {
                    JjError::Backend {
                        message: error.to_string(),
                    }
                })?;
            if !is_empty {
                count += 1;
            }
        }
        Ok(count)
    }

    /// Returns read-side facts for the selected revision, or the working copy when omitted.
    pub fn facts_for_revision(&self, revision: Option<&str>) -> Result<WorkspaceFacts, JjError> {
        self.ensure_git_backed()?;
        self.facts_for_commit(self.target_for_revision(revision)?)
    }

    /// Returns push planning facts, reusing local bookmarks even before an origin trunk is known.
    pub fn push_facts_for_revision(
        &self,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError> {
        self.ensure_git_backed()?;
        let target = self.target_for_revision(revision)?;
        match self.facts_for_commit(target.clone()) {
            Ok(facts) => Ok(facts),
            Err(JjError::MissingTrunk { remote }) if remote == ORIGIN_REMOTE_NAME => {
                self.push_facts_without_trunk(target)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn target_for_revision(&self, revision: Option<&str>) -> Result<Commit, JjError> {
        match revision {
            Some(revision) => self.resolve_single_revision(revision, "In selected jj revision"),
            None => self.current_commit(),
        }
    }

    pub(super) fn facts_for_commit(&self, target: Commit) -> Result<WorkspaceFacts, JjError> {
        let (origin_branch, trunk) = self.resolve_trunk(&target)?;
        let stack_path = self.linear_stack_path(&trunk, &target)?;
        let local_bookmarks = self.local_bookmarks();
        let local_bookmarks_at_target = self.local_bookmarks_for_commit(target.id());
        let nearest_ancestor_bookmark = self.nearest_ancestor_bookmark(&trunk, &stack_path);
        let stack_index = stack_path.len().saturating_sub(1);
        let changed_files = changed_files_for_commit(self.repo.as_ref(), &target)?;

        Ok(WorkspaceFacts {
            workspace_root: self.workspace_root(),
            target_change: self.change_summary(&target)?,
            trunk: TrunkSummary {
                branch: origin_branch.clone(),
                commit_id: trunk.id().hex(),
                short_commit_id: short_commit_id(trunk.id()),
            },
            trunk_git_commit_sha: trunk.id().hex(),
            origin_branch,
            local_bookmarks,
            local_bookmarks_at_target,
            nearest_ancestor_bookmark,
            changed_files,
            stack_index,
        })
    }

    pub(super) fn push_facts_without_trunk(
        &self,
        target: Commit,
    ) -> Result<WorkspaceFacts, JjError> {
        let local_bookmarks = self.local_bookmarks();
        let local_bookmarks_at_target = self.local_bookmarks_for_commit(target.id());
        let Some(origin_branch) = local_bookmarks_at_target.first().cloned() else {
            return Err(JjError::MissingTrunk {
                remote: ORIGIN_REMOTE_NAME.to_owned(),
            });
        };
        let target_commit_id = target.id().hex();

        Ok(WorkspaceFacts {
            workspace_root: self.workspace_root(),
            target_change: self.change_summary(&target)?,
            trunk: TrunkSummary {
                branch: origin_branch.clone(),
                commit_id: target_commit_id.clone(),
                short_commit_id: short_commit_id(target.id()),
            },
            trunk_git_commit_sha: target_commit_id,
            origin_branch,
            local_bookmarks,
            local_bookmarks_at_target,
            nearest_ancestor_bookmark: None,
            changed_files: changed_files_for_commit(self.repo.as_ref(), &target)?,
            stack_index: 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TrunkCandidate {
    pub(super) branch: String,
    pub(super) commit_id: CommitId,
}

pub(super) fn select_trunk_candidate(
    remote: &str,
    candidates: Vec<TrunkCandidate>,
    conflicted: Vec<String>,
) -> Result<TrunkCandidate, JjError> {
    if candidates.is_empty() {
        if conflicted.is_empty() {
            return Err(JjError::MissingTrunk {
                remote: remote.to_owned(),
            });
        }

        return Err(JjError::ConflictedTrunk {
            remote: remote.to_owned(),
            branches: conflicted,
        });
    }

    let preferred = candidates
        .iter()
        .filter(|candidate| PREFERRED_TRUNK_BRANCHES.contains(&candidate.branch.as_str()))
        .collect::<Vec<_>>();

    match (preferred.as_slice(), candidates.as_slice()) {
        ([candidate], _) => Ok((*candidate).clone()),
        ([], [candidate]) => Ok(candidate.clone()),
        _ => Err(JjError::AmbiguousTrunk {
            remote: remote.to_owned(),
            branches: candidates
                .into_iter()
                .map(|candidate| candidate.branch)
                .collect(),
        }),
    }
}

pub(super) fn changed_files_for_commit(
    repo: &dyn jj_lib::repo::Repo,
    commit: &Commit,
) -> Result<Vec<String>, JjError> {
    pollster::block_on(async {
        let parent_tree = commit
            .parent_tree(repo)
            .await
            .map_err(|error| JjError::Backend {
                message: error.to_string(),
            })?;
        let target_tree = commit.tree();
        let mut changed_files = Vec::new();
        let mut diff_stream = parent_tree.diff_stream(&target_tree, &EverythingMatcher);

        while let Some(entry) = diff_stream.next().await {
            entry.values.map_err(|error| JjError::Backend {
                message: error.to_string(),
            })?;
            changed_files.push(entry.path.as_internal_file_string().to_owned());
        }
        changed_files.sort();
        changed_files.dedup();

        Ok(changed_files)
    })
}
