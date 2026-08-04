use super::*;
use std::time::{Duration, Instant};

impl JjWorkspace {
    /// Returns read-side facts for the current working-copy change.
    pub fn facts(&self) -> Result<WorkspaceFacts, JjError> {
        self.facts_for_revision(None, StackBasePolicy::CurrentTrunk)
    }

    /// Returns local cached trunk facts for each configured remote status should report.
    pub fn status_facts<'a>(
        &self,
        remote_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<StatusWorkspaceFacts, JjError> {
        self.status_facts_with_metrics(remote_names)
            .map(|result| result.facts)
    }

    pub fn stack_trunk_status_facts<'a>(
        &self,
        remote_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<StatusWorkspaceFacts, JjError> {
        self.stack_trunk_status_facts_with_metrics(remote_names)
            .map(|result| result.facts)
    }

    /// Returns trunk-status facts for stack status without treating PR refs as trunk.
    pub fn stack_trunk_status_facts_with_metrics<'a>(
        &self,
        remote_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<StatusWorkspaceFactsWithMetrics, JjError> {
        self.ensure_git_backed()?;
        let current_commit_started = Instant::now();
        let target = self.current_commit()?;
        let mut metrics = StatusWorkspaceMetrics {
            current_commit_us: duration_us(current_commit_started.elapsed()),
            remotes: Vec::new(),
        };
        let mut remotes = Vec::new();

        for remote in remote_names {
            let resolve_trunk_started = Instant::now();
            let (branch, trunk, trunk_metrics) = self.unanchored_trunk_for_remote(remote)?;
            let resolve_trunk_us = duration_us(resolve_trunk_started.elapsed());

            let linear_stack_path_started = Instant::now();
            let stack_path = self.stack_status_path_from_trunk(&trunk, &target)?;
            let linear_stack_path_us = duration_us(linear_stack_path_started.elapsed());

            let count_non_empty_commits_started = Instant::now();
            let local_ahead_by = self.non_empty_commit_count(&stack_path)?;
            let count_non_empty_commits_us = duration_us(count_non_empty_commits_started.elapsed());
            metrics.remotes.push(StatusRemoteMetrics {
                remote: remote.to_owned(),
                branch: branch.clone(),
                stack_path_len: stack_path.len(),
                non_empty_count: local_ahead_by,
                resolve_trunk_us,
                trunk: trunk_metrics,
                linear_stack_path_us,
                count_non_empty_commits_us,
            });
            remotes.push(StatusRemoteFacts {
                remote: remote.to_owned(),
                branch,
                trunk_git_commit_sha: trunk.id().hex(),
                trunk_short_commit_id: short_commit_id(trunk.id()),
                local_ahead_by,
            });
        }

        Ok(StatusWorkspaceFactsWithMetrics {
            facts: StatusWorkspaceFacts { remotes },
            metrics,
        })
    }

    /// Returns local cached trunk facts and timing detail for performance tracing.
    pub fn status_facts_with_metrics<'a>(
        &self,
        remote_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<StatusWorkspaceFactsWithMetrics, JjError> {
        self.ensure_git_backed()?;
        let current_commit_started = Instant::now();
        let target = self.current_commit()?;
        let mut metrics = StatusWorkspaceMetrics {
            current_commit_us: duration_us(current_commit_started.elapsed()),
            remotes: Vec::new(),
        };
        let mut remotes = Vec::new();

        for remote in remote_names {
            let resolve_trunk_started = Instant::now();
            let (branch, trunk, trunk_metrics) =
                self.resolve_trunk_for_remote_with_hint_and_metrics(&target, remote, None)?;
            let resolve_trunk_us = duration_us(resolve_trunk_started.elapsed());

            let linear_stack_path_started = Instant::now();
            let stack_path = self.linear_stack_path(&trunk, &target)?;
            let linear_stack_path_us = duration_us(linear_stack_path_started.elapsed());

            let count_non_empty_commits_started = Instant::now();
            let local_ahead_by = self.non_empty_commit_count(&stack_path)?;
            let count_non_empty_commits_us = duration_us(count_non_empty_commits_started.elapsed());
            metrics.remotes.push(StatusRemoteMetrics {
                remote: remote.to_owned(),
                branch: branch.clone(),
                stack_path_len: stack_path.len(),
                non_empty_count: local_ahead_by,
                resolve_trunk_us,
                trunk: trunk_metrics,
                linear_stack_path_us,
                count_non_empty_commits_us,
            });
            remotes.push(StatusRemoteFacts {
                remote: remote.to_owned(),
                branch,
                trunk_git_commit_sha: trunk.id().hex(),
                trunk_short_commit_id: short_commit_id(trunk.id()),
                local_ahead_by,
            });
        }

        Ok(StatusWorkspaceFactsWithMetrics {
            facts: StatusWorkspaceFacts { remotes },
            metrics,
        })
    }

    fn unanchored_trunk_for_remote(
        &self,
        remote: &str,
    ) -> Result<(String, Commit, TrunkResolveMetrics), JjError> {
        let mut metrics = TrunkResolveMetrics::default();
        let mut candidates = Vec::new();
        let mut conflicted = Vec::new();

        let scan_started = Instant::now();
        for (branch, remote_ref) in self.repo.view().remote_bookmarks(RemoteName::new(remote)) {
            metrics.remote_bookmark_count += 1;
            let branch_name = branch.as_str().to_owned();
            let ref_target = &remote_ref.target;

            if ref_target.has_conflict() {
                metrics.conflicted_bookmark_count += 1;
                conflicted.push(branch_name);
                continue;
            }

            if let Some(commit_id) = ref_target.as_normal() {
                metrics.normal_bookmark_count += 1;
                candidates.push(TrunkCandidate {
                    branch: branch_name,
                    commit_id: commit_id.clone(),
                });
            }
        }
        metrics.scan_remote_bookmarks_us = duration_us(scan_started.elapsed());
        metrics.candidate_count = candidates.len();

        let select_started = Instant::now();
        let candidate = select_trunk_candidate_with_hint(remote, candidates, conflicted, None)?;
        metrics.select_candidate_us = duration_us(select_started.elapsed());

        let load_started = Instant::now();
        let trunk = self.load_commit(&candidate.commit_id)?;
        metrics.load_trunk_commit_us = duration_us(load_started.elapsed());

        Ok((candidate.branch, trunk, metrics))
    }

    fn stack_status_path_from_trunk(
        &self,
        trunk: &Commit,
        target: &Commit,
    ) -> Result<Vec<Commit>, JjError> {
        if self.is_ancestor_or_equal(trunk.id(), target.id())? {
            self.linear_stack_path(trunk, target)
        } else {
            Ok(Vec::new())
        }
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

    /// Returns whether global fetch can safely update this repo without touching local work.
    pub fn is_empty_working_copy_child_of_origin_trunk(&self) -> Result<bool, JjError> {
        self.ensure_git_backed()?;
        let current = self.current_commit()?;
        if !current.description().trim().is_empty() {
            return Ok(false);
        }

        let is_empty =
            pollster::block_on(current.is_empty(self.repo.as_ref())).map_err(|error| {
                JjError::Backend {
                    message: error.to_string(),
                }
            })?;
        if !is_empty {
            return Ok(false);
        }

        let parents = current.parent_ids();
        let [parent] = parents else {
            return Ok(false);
        };
        let (_, trunk) = self.resolve_trunk_for_remote(&current, ORIGIN_REMOTE_NAME)?;

        Ok(parent == trunk.id())
    }

    /// Returns read-side facts for the selected revision, or the working copy when omitted.
    pub fn facts_for_revision(
        &self,
        revision: Option<&str>,
        stack_base_policy: StackBasePolicy,
    ) -> Result<WorkspaceFacts, JjError> {
        self.ensure_git_backed()?;
        self.facts_for_commit(self.target_for_revision(revision)?, stack_base_policy)
    }

    /// Returns local bookmark heads that can have associated pull requests.
    pub fn pull_request_bookmarks(&self) -> Result<Vec<String>, JjError> {
        self.ensure_git_backed()?;
        let mut bookmarks = self
            .repo
            .view()
            .local_bookmarks()
            .filter(|(_, target)| target.as_normal().is_some())
            .map(|(bookmark, _)| bookmark.as_str().to_owned())
            .collect::<Vec<_>>();
        bookmarks.sort();
        bookmarks.dedup();
        Ok(bookmarks)
    }

    /// Returns PR bookmark heads for the selected revision, commit prefix, or exact bookmark.
    pub fn pull_request_candidate_bookmarks(
        &self,
        selector: Option<&str>,
    ) -> Result<Vec<String>, JjError> {
        self.ensure_git_backed()?;
        let Some(selector) = selector else {
            return self.pull_request_candidate_bookmarks_for_commit(self.current_commit()?);
        };

        match self.resolve_single_revision(selector, "In selected jj revision") {
            Ok(target) => self.pull_request_candidate_bookmarks_for_commit(target),
            Err(error) if can_try_pull_request_bookmark_selector(&error) => {
                if self.has_normal_local_bookmark(selector.trim()) {
                    Ok(vec![selector.trim().to_owned()])
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }

    fn pull_request_candidate_bookmarks_for_commit(
        &self,
        target: Commit,
    ) -> Result<Vec<String>, JjError> {
        let mut candidates = Vec::new();

        for (bookmark, ref_target) in self.repo.view().local_bookmarks() {
            let Some(commit_id) = ref_target.as_normal() else {
                continue;
            };
            let commit = self.load_commit(commit_id)?;
            let Some(distance) = self.linear_descendant_distance(&target, &commit)? else {
                continue;
            };

            candidates.push((distance, bookmark.as_str().to_owned()));
        }

        candidates.sort_by(
            |(left_distance, left_bookmark), (right_distance, right_bookmark)| {
                left_distance
                    .cmp(right_distance)
                    .then_with(|| left_bookmark.cmp(right_bookmark))
            },
        );
        Ok(candidates
            .into_iter()
            .map(|(_, bookmark)| bookmark)
            .collect())
    }

    fn has_normal_local_bookmark(&self, selector: &str) -> bool {
        self.repo
            .view()
            .local_bookmarks()
            .any(|(bookmark, target)| bookmark.as_str() == selector && target.as_normal().is_some())
    }

    /// Returns push planning facts, reusing local bookmarks even before an origin trunk is known.
    pub fn push_facts_for_revision(
        &self,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError> {
        self.ensure_git_backed()?;
        let target = self.target_for_revision(revision)?;
        match self.facts_for_commit(target.clone(), StackBasePolicy::CurrentTrunk) {
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

    pub(super) fn facts_for_commit(
        &self,
        target: Commit,
        stack_base_policy: StackBasePolicy,
    ) -> Result<WorkspaceFacts, JjError> {
        let (origin_branch, stack_base, stack_path) =
            self.stack_path_for_target(&target, stack_base_policy)?;
        let local_bookmarks = self.local_bookmarks();
        self.facts_for_stack_path(
            &target,
            &origin_branch,
            &stack_base,
            &stack_path,
            &local_bookmarks,
        )
    }

    pub(super) fn facts_for_stack_path(
        &self,
        target: &Commit,
        origin_branch: &str,
        stack_base: &Commit,
        stack_path: &[Commit],
        local_bookmarks: &[String],
    ) -> Result<WorkspaceFacts, JjError> {
        let local_bookmarks_at_target = self.local_bookmarks_for_commit(target.id());
        let nearest_ancestor_bookmark = self.nearest_ancestor_bookmark(stack_base, stack_path);
        let stack_index = stack_path.len().saturating_sub(1);
        let changed = changed_file_facts_for_commit(self.repo.as_ref(), target)?;
        let stack_base_commit_id = stack_base.id().hex();

        Ok(WorkspaceFacts {
            workspace_root: self.workspace_root(),
            target_change: self.change_summary(target)?,
            trunk: TrunkSummary {
                branch: origin_branch.to_owned(),
                commit_id: stack_base_commit_id.clone(),
                short_commit_id: short_commit_id(stack_base.id()),
            },
            trunk_git_commit_sha: stack_base_commit_id,
            origin_branch: origin_branch.to_owned(),
            local_bookmarks: local_bookmarks.to_vec(),
            local_bookmarks_at_target,
            nearest_ancestor_bookmark,
            changed_files: changed.files,
            change_lines: changed.lines,
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
        let changed = changed_file_facts_for_commit(self.repo.as_ref(), &target)?;

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
            changed_files: changed.files,
            change_lines: changed.lines,
            stack_index: 0,
        })
    }
}

fn can_try_pull_request_bookmark_selector(error: &JjError) -> bool {
    matches!(
        error,
        JjError::Revision { .. } | JjError::RevisionNotFound { .. }
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TrunkCandidate {
    pub(super) branch: String,
    pub(super) commit_id: CommitId,
}

/// Chooses a cached trunk candidate, letting networked workflows prefer a trusted branch hint.
pub(super) fn select_trunk_candidate_with_hint(
    remote: &str,
    candidates: Vec<TrunkCandidate>,
    conflicted: Vec<String>,
    branch_hint: Option<&str>,
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

    if let Some(branch_hint) = branch_hint {
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.branch == branch_hint)
        {
            return Ok(candidate.clone());
        }
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

pub(super) struct ChangedFileFacts {
    pub(super) files: Vec<String>,
    pub(super) lines: Vec<String>,
}

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().try_into().unwrap_or(u64::MAX)
}

pub(super) fn changed_file_facts_for_commit(
    repo: &dyn jj_lib::repo::Repo,
    commit: &Commit,
) -> Result<ChangedFileFacts, JjError> {
    pollster::block_on(async {
        let parent_tree = commit
            .parent_tree(repo)
            .await
            .map_err(|error| JjError::Backend {
                message: error.to_string(),
            })?;
        let target_tree = commit.tree();
        let mut changed = Vec::new();
        let mut diff_stream = parent_tree.diff_stream(&target_tree, &EverythingMatcher);

        while let Some(entry) = diff_stream.next().await {
            let values = entry.values.map_err(|error| JjError::Backend {
                message: error.to_string(),
            })?;
            let path = entry.path.as_internal_file_string().to_owned();
            let status = if values.before.is_absent() {
                "A"
            } else if values.after.is_absent() {
                "D"
            } else {
                "M"
            };
            changed.push((path.clone(), format!("{status} {path}")));
        }
        changed.sort_by(|left, right| left.0.cmp(&right.0));
        changed.dedup_by(|left, right| left.0 == right.0);
        let (files, lines) = changed.into_iter().unzip();

        Ok(ChangedFileFacts { files, lines })
    })
}
