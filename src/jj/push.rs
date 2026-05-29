use super::*;

impl JjWorkspace {
    /// Pushes `branch` to fixed `origin` and updates jj's remote bookmark view.
    pub fn push_bookmark(&mut self, branch: &str) -> Result<PushOutcome, JjError> {
        self.ensure_git_backed()?;
        let branches = [branch.to_owned()];
        self.push_bookmarks(&branches)
            .map(|mut outcomes| outcomes.remove(0))
    }

    /// Pushes selected branches to fixed `origin` with one Git transport mutation.
    pub fn push_bookmarks(&mut self, branches: &[String]) -> Result<Vec<PushOutcome>, JjError> {
        self.ensure_git_backed()?;

        let mut outcomes = Vec::new();
        let mut updates = Vec::new();
        for branch in branches {
            let bookmark = RefName::new(branch);
            let targets = self.local_and_origin_bookmark_targets(bookmark);
            if targets.local_target.has_conflict() {
                return Err(JjError::ConflictedBookmark {
                    branch: branch.clone(),
                });
            }
            if targets.local_target.as_normal().is_none() {
                return Err(JjError::MissingLocalBookmark {
                    branch: branch.clone(),
                });
            }

            let Some(update) = classify_push_bookmark_update(
                bookmark.to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME)),
                targets,
                true,
                false,
            )?
            else {
                outcomes.push(PushOutcome {
                    branch: branch.clone(),
                    pushed_refs: 0,
                    pushed_commits: Vec::new(),
                });
                continue;
            };

            let update = (RefNameBuf::from(branch.as_str()), update);
            let pushed_commits = self.pushed_commits_for_updates(std::slice::from_ref(&update))?;
            outcomes.push(PushOutcome {
                branch: branch.clone(),
                pushed_refs: 1,
                pushed_commits,
            });
            updates.push(update);
        }

        if updates.is_empty() {
            return Ok(outcomes);
        }

        let push_stats = self.push_origin_bookmark_updates(
            updates,
            "jx stack publish push branches".to_owned(),
            "stack publish branches".to_owned(),
        )?;
        let mut pushed = push_stats.pushed.len();
        for outcome in &mut outcomes {
            if outcome.pushed_refs == 0 {
                continue;
            }
            outcome.pushed_refs = usize::from(pushed > 0);
            pushed = pushed.saturating_sub(1);
        }

        Ok(outcomes)
    }

    /// Pushes all tracked fixed-origin bookmarks, including local deletions.
    pub fn push_tracked_deleted(&mut self) -> Result<TrackedPushOutcome, JjError> {
        self.ensure_git_backed()?;
        let updates = self.tracked_origin_bookmark_updates()?;
        self.push_tracked_updates(
            updates,
            "jx push tracked bookmarks".to_owned(),
            "tracked bookmarks".to_owned(),
        )
    }

    /// Pushes tracked bookmarks whose push ranges do not contain conflicted commits.
    pub fn push_syncable_tracked(&mut self) -> Result<SyncPushOutcome, JjError> {
        self.ensure_git_backed()?;
        let updates = self.tracked_origin_bookmark_updates()?;
        let split = self.split_conflicted_tracked_bookmark_updates(updates)?;
        let mut pushed = self.push_tracked_updates(
            split.pushable,
            "jx sync push tracked bookmarks".to_owned(),
            "tracked bookmarks".to_owned(),
        )?;
        let pushed_branches = pushed
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.branch.clone())
            .collect::<BTreeSet<_>>();
        pushed
            .bookmarks
            .extend(self.unchanged_tracked_bookmark_summaries(&pushed_branches)?);

        Ok(SyncPushOutcome {
            pushed,
            skipped_conflicted_bookmarks: split.skipped_conflicted,
        })
    }

    /// Pushes one selected bookmarked revision when its push range has no conflicts.
    pub fn push_syncable_revision(
        &mut self,
        revision: Option<&str>,
    ) -> Result<SyncPushOutcome, JjError> {
        self.ensure_git_backed()?;
        let selection = self.sync_bookmark_selection_for_revision(revision)?;
        let bookmark = RefName::new(&selection.branch);
        let targets = self.local_and_origin_bookmark_targets(bookmark);
        if targets.local_target.has_conflict() {
            return Err(JjError::ConflictedBookmark {
                branch: selection.branch,
            });
        }
        let Some(target_id) = targets.local_target.as_normal().cloned() else {
            return Err(JjError::MissingLocalBookmark {
                branch: selection.branch,
            });
        };
        if target_id != selection.target_id {
            return Err(JjError::BookmarkExistsOnDifferentChange {
                branch: selection.branch,
            });
        }

        let Some(update) = classify_push_bookmark_update(
            bookmark.to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME)),
            targets,
            true,
            false,
        )?
        else {
            return Ok(SyncPushOutcome {
                pushed: self.unchanged_sync_bookmark_outcome(&selection.branch, &target_id)?,
                skipped_conflicted_bookmarks: Vec::new(),
            });
        };

        let split = self.split_conflicted_tracked_bookmark_updates(vec![(
            RefNameBuf::from(selection.branch.as_str()),
            update,
        )])?;
        let pushed = self.push_tracked_updates(
            split.pushable,
            format!("jx sync push {}", selection.branch),
            selection.branch,
        )?;

        Ok(SyncPushOutcome {
            pushed,
            skipped_conflicted_bookmarks: split.skipped_conflicted,
        })
    }

    pub(super) fn sync_bookmark_selection_for_revision(
        &self,
        revision: Option<&str>,
    ) -> Result<SyncBookmarkSelection, JjError> {
        if let Some(selector) = revision
            .map(str::trim)
            .filter(|selector| !selector.is_empty())
        {
            let target = self.repo.view().get_local_bookmark(RefName::new(selector));
            if target.has_conflict() {
                return Err(JjError::ConflictedBookmark {
                    branch: selector.to_owned(),
                });
            }
            if let Some(commit_id) = target.as_normal() {
                return Ok(SyncBookmarkSelection {
                    branch: selector.to_owned(),
                    target_id: commit_id.clone(),
                });
            }
        }

        let target = self.target_for_revision(revision)?;
        let mut bookmarks = self.local_bookmarks_for_commit(target.id());
        bookmarks.sort();
        bookmarks.dedup();

        match bookmarks.as_slice() {
            [] => Err(JjError::MissingSyncBookmark),
            [branch] => Ok(SyncBookmarkSelection {
                branch: branch.clone(),
                target_id: target.id().clone(),
            }),
            _ => Err(JjError::AmbiguousSyncBookmark { bookmarks }),
        }
    }

    fn unchanged_sync_bookmark_outcome(
        &self,
        branch: &str,
        target_id: &CommitId,
    ) -> Result<TrackedPushOutcome, JjError> {
        let update = (
            RefNameBuf::from(branch),
            Diff {
                before: Some(target_id.clone()),
                after: Some(target_id.clone()),
            },
        );
        let trunk = self.tracked_push_trunk();
        Ok(TrackedPushOutcome {
            pushed_refs: 0,
            bookmarks: pushed_bookmark_summaries(
                self.repo.as_ref(),
                &[update],
                trunk.as_ref(),
                self.workspace.workspace_name(),
            )?,
            pushed_commits: Vec::new(),
        })
    }

    fn push_tracked_updates(
        &mut self,
        updates: Vec<BookmarkPushUpdate>,
        tx_description: String,
        error_branch: String,
    ) -> Result<TrackedPushOutcome, JjError> {
        if updates.is_empty() {
            return Ok(TrackedPushOutcome {
                pushed_refs: 0,
                bookmarks: Vec::new(),
                pushed_commits: Vec::new(),
            });
        }

        let trunk = self.tracked_push_trunk();
        let bookmarks = pushed_bookmark_summaries(
            self.repo.as_ref(),
            &updates,
            trunk.as_ref(),
            self.workspace.workspace_name(),
        )?;
        let pushed_commits = self.pushed_commits_for_updates(&updates)?;
        let push_stats =
            self.push_origin_bookmark_updates(updates, tx_description, error_branch)?;

        Ok(TrackedPushOutcome {
            pushed_refs: push_stats.pushed.len(),
            bookmarks,
            pushed_commits,
        })
    }

    pub(super) fn tracked_origin_bookmark_updates(
        &self,
    ) -> Result<Vec<BookmarkPushUpdate>, JjError> {
        let mut updates = Vec::new();

        for (name, targets) in self
            .repo
            .view()
            .local_remote_bookmarks(RemoteName::new(ORIGIN_REMOTE_NAME))
        {
            if !targets.remote_ref.is_tracked() {
                continue;
            }

            if let Some(update) = classify_push_bookmark_update(
                name.to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME)),
                targets,
                false,
                true,
            )? {
                updates.push((name.to_owned(), update));
            }
        }

        Ok(updates)
    }

    fn unchanged_tracked_bookmark_summaries(
        &self,
        exclude: &BTreeSet<String>,
    ) -> Result<Vec<PushedBookmarkSummary>, JjError> {
        let trunk = self.tracked_push_trunk();
        let updates = self.unchanged_tracked_origin_bookmark_updates(
            exclude,
            trunk.as_ref().map(|trunk| &trunk.id),
        );
        pushed_bookmark_summaries(
            self.repo.as_ref(),
            &updates,
            trunk.as_ref(),
            self.workspace.workspace_name(),
        )
    }

    fn unchanged_tracked_origin_bookmark_updates(
        &self,
        exclude: &BTreeSet<String>,
        trunk_id: Option<&CommitId>,
    ) -> Vec<BookmarkPushUpdate> {
        self.repo
            .view()
            .local_remote_bookmarks(RemoteName::new(ORIGIN_REMOTE_NAME))
            .filter_map(|(name, targets)| {
                if exclude.contains(name.as_str()) || !targets.remote_ref.is_tracked() {
                    return None;
                }
                let local = targets.local_target.as_normal()?;
                let remote = targets.remote_ref.target.as_normal()?;
                if local != remote || trunk_id == Some(local) {
                    return None;
                }

                Some((
                    name.to_owned(),
                    Diff {
                        before: Some(local.clone()),
                        after: Some(local.clone()),
                    },
                ))
            })
            .collect()
    }

    pub(super) fn split_conflicted_tracked_bookmark_updates(
        &self,
        updates: Vec<BookmarkPushUpdate>,
    ) -> Result<SyncableBookmarkUpdates, JjError> {
        let trunk_id = self.tracked_push_trunk_id();
        let mut pushable = Vec::new();
        let mut skipped_conflicted = Vec::new();

        for (name, update) in updates {
            let conflicted_commits = self.conflicted_commits_for_update(
                &update,
                trunk_id.as_ref(),
                self.workspace.workspace_name(),
            )?;
            if conflicted_commits.is_empty() {
                pushable.push((name, update));
            } else {
                skipped_conflicted.push(SkippedPushBookmarkSummary {
                    branch: name.as_str().to_owned(),
                    conflicted_commits,
                });
            }
        }

        Ok(SyncableBookmarkUpdates {
            pushable,
            skipped_conflicted,
        })
    }

    fn conflicted_commits_for_update(
        &self,
        update: &Diff<Option<CommitId>>,
        trunk_id: Option<&CommitId>,
        current_workspace: &WorkspaceName,
    ) -> Result<Vec<ConflictedCommitSummary>, JjError> {
        let Some(new_head) = update.after.clone() else {
            return Ok(Vec::new());
        };

        let old_heads = self
            .repo
            .view()
            .remote_bookmarks(RemoteName::new(ORIGIN_REMOTE_NAME))
            .flat_map(|(_, remote_ref)| remote_ref.target.added_ids().cloned())
            .collect::<Vec<_>>();
        let revset = ResolvedRevsetExpression::commits(old_heads)
            .range(&ResolvedRevsetExpression::commit(new_head))
            .evaluate(self.repo.as_ref())
            .map_err(|error| JjError::Backend {
                message: error.into_backend_error().to_string(),
            })?;
        let ids = pollster::block_on(revset.stream().try_collect::<Vec<_>>()).map_err(|error| {
            JjError::Backend {
                message: error.into_backend_error().to_string(),
            }
        })?;
        let mut commits = ids
            .into_iter()
            .map(|id| self.load_commit(&id))
            .collect::<Result<Vec<_>, _>>()?;
        commits.reverse();

        commits
            .into_iter()
            .filter(|commit| commit.has_conflict())
            .map(|commit| {
                Ok(ConflictedCommitSummary {
                    short_commit_id: short_commit_id(commit.id()),
                    description: first_description_line(commit.description()).to_owned(),
                    workspace_visibility: commit_workspace_visibility(
                        self.repo.as_ref(),
                        Some(commit.id()),
                        trunk_id,
                        current_workspace,
                    )?,
                })
            })
            .collect()
    }

    fn tracked_push_trunk_id(&self) -> Option<CommitId> {
        self.tracked_push_trunk().map(|trunk| trunk.id)
    }

    fn tracked_push_trunk(&self) -> Option<TrackedPushTrunk> {
        self.current_commit().ok().and_then(|current| {
            self.resolve_trunk(&current)
                .ok()
                .map(|(branch, trunk)| TrackedPushTrunk {
                    branch,
                    id: trunk.id().clone(),
                })
        })
    }

    pub(super) fn pushed_commits_for_updates(
        &self,
        updates: &[BookmarkPushUpdate],
    ) -> Result<Vec<PushedCommitSummary>, JjError> {
        let new_heads = updates
            .iter()
            .filter_map(|(_, update)| update.after.clone())
            .collect::<Vec<_>>();
        if new_heads.is_empty() {
            return Ok(Vec::new());
        }

        let old_heads = self
            .repo
            .view()
            .remote_bookmarks(RemoteName::new(ORIGIN_REMOTE_NAME))
            .flat_map(|(_, remote_ref)| remote_ref.target.added_ids().cloned())
            .collect::<Vec<_>>();
        let revset = ResolvedRevsetExpression::commits(old_heads)
            .range(&ResolvedRevsetExpression::commits(new_heads))
            .evaluate(self.repo.as_ref())
            .map_err(|error| JjError::Backend {
                message: error.into_backend_error().to_string(),
            })?;
        let ids = pollster::block_on(revset.stream().try_collect::<Vec<_>>()).map_err(|error| {
            JjError::Backend {
                message: error.into_backend_error().to_string(),
            }
        })?;
        let mut commits = ids
            .into_iter()
            .map(|id| {
                self.load_commit(&id)
                    .and_then(|commit| pushed_commit_summary(&commit))
            })
            .collect::<Result<Vec<_>, _>>()?;
        commits.reverse();

        Ok(commits)
    }

    pub(super) fn push_origin_bookmark_updates(
        &mut self,
        updates: Vec<BookmarkPushUpdate>,
        tx_description: String,
        error_branch: String,
    ) -> Result<git::GitPushStats, JjError> {
        let mut tx = self.repo.start_transaction();
        let push_stats = push_origin_bookmark_updates(tx.repo_mut(), updates, &error_branch)?;
        if !push_stats.all_ok() {
            return Err(JjError::PushRejected {
                branch: error_branch,
                message: push_rejection_message(&push_stats),
            });
        }
        export_git_refs(tx.repo_mut())?;

        let repo = pollster::block_on(tx.commit(tx_description)).map_err(|error| {
            JjError::Transaction {
                message: error.to_string(),
            }
        })?;
        self.repo = repo;

        Ok(push_stats)
    }
}

#[derive(Debug)]
pub(super) struct SyncBookmarkSelection {
    pub(super) branch: String,
    pub(super) target_id: CommitId,
}

#[derive(Debug, Clone)]
pub(super) struct TrackedPushTrunk {
    pub(super) branch: String,
    pub(super) id: CommitId,
}

struct BookmarkStackBase {
    branch: String,
    id: CommitId,
}

pub(super) struct SyncableBookmarkUpdates {
    pub(super) pushable: Vec<BookmarkPushUpdate>,
    pub(super) skipped_conflicted: Vec<SkippedPushBookmarkSummary>,
}

pub(super) fn classify_push_bookmark_update(
    remote_symbol: jj_lib::ref_name::RemoteRefSymbol<'_>,
    targets: LocalAndRemoteRef<'_>,
    allow_new: bool,
    allow_delete: bool,
) -> Result<Option<Diff<Option<CommitId>>>, JjError> {
    match classify_ref_push_action(targets) {
        RefPushAction::AlreadyMatches => Ok(None),
        RefPushAction::LocalConflicted => Err(JjError::ConflictedBookmark {
            branch: remote_symbol.name.as_str().to_owned(),
        }),
        RefPushAction::RemoteConflicted => Err(JjError::ConflictedRemoteBookmark {
            branch: remote_symbol.name.as_str().to_owned(),
            remote: ORIGIN_REMOTE_NAME,
        }),
        RefPushAction::RemoteUntracked => Err(JjError::NonTrackingRemoteBookmark {
            branch: remote_symbol.name.as_str().to_owned(),
            remote: ORIGIN_REMOTE_NAME,
        }),
        RefPushAction::Update(update) if update.after.is_none() && !allow_delete => {
            Err(JjError::DeletedBookmarkNotRequested {
                branch: remote_symbol.name.as_str().to_owned(),
            })
        }
        RefPushAction::Update(_) if !targets.remote_ref.is_tracked() && !allow_new => {
            Err(JjError::NewRemoteBookmarkNotAllowed {
                branch: remote_symbol.name.as_str().to_owned(),
                remote: ORIGIN_REMOTE_NAME,
            })
        }
        RefPushAction::Update(update) => Ok(Some(update)),
    }
}

pub(super) fn pushed_bookmark_summaries(
    repo: &dyn jj_lib::repo::Repo,
    updates: &[BookmarkPushUpdate],
    trunk: Option<&TrackedPushTrunk>,
    current_workspace: &WorkspaceName,
) -> Result<Vec<PushedBookmarkSummary>, JjError> {
    updates
        .iter()
        .map(|(branch, update)| {
            Ok(PushedBookmarkSummary {
                branch: branch.as_str().to_owned(),
                old_short_commit_id: update.before.as_ref().map(short_commit_id),
                new_short_commit_id: update.after.as_ref().map(short_commit_id),
                old_description: commit_description(repo, update.before.as_ref())?,
                new_description: commit_description(repo, update.after.as_ref())?,
                pull_request_description: bookmark_pull_request_description(
                    repo,
                    update.after.as_ref(),
                    trunk,
                )?,
                pull_request_base: bookmark_pull_request_base(repo, update.after.as_ref(), trunk)?,
                new_workspace_visibility: commit_workspace_visibility(
                    repo,
                    update.after.as_ref(),
                    trunk.map(|trunk| &trunk.id),
                    current_workspace,
                )?,
            })
        })
        .collect()
}

pub(super) fn commit_description(
    repo: &dyn jj_lib::repo::Repo,
    commit_id: Option<&CommitId>,
) -> Result<Option<String>, JjError> {
    commit_id
        .map(|commit_id| {
            load_commit_from_repo(repo, commit_id)
                .map(|commit| first_description_line(commit.description()).to_owned())
        })
        .transpose()
}

/// Returns the local description that should back a PR for a pushed bookmark target.
pub(super) fn bookmark_pull_request_description(
    repo: &dyn jj_lib::repo::Repo,
    target_id: Option<&CommitId>,
    trunk: Option<&TrackedPushTrunk>,
) -> Result<Option<String>, JjError> {
    let Some(target_id) = target_id else {
        return Ok(None);
    };
    let description_commit_id = match bookmark_pull_request_stack_base(repo, target_id, trunk)? {
        Some(base) => first_linear_stack_commit_id(repo, target_id, &base.id)?
            .unwrap_or_else(|| target_id.clone()),
        None => target_id.clone(),
    };

    commit_full_description(repo, &description_commit_id)
}

/// Returns the PR base branch that keeps a pushed bookmark in its local stack.
pub(super) fn bookmark_pull_request_base(
    repo: &dyn jj_lib::repo::Repo,
    target_id: Option<&CommitId>,
    trunk: Option<&TrackedPushTrunk>,
) -> Result<Option<String>, JjError> {
    let Some(target_id) = target_id else {
        return Ok(None);
    };

    Ok(bookmark_pull_request_stack_base(repo, target_id, trunk)?.map(|base| base.branch))
}

fn first_linear_stack_commit_id(
    repo: &dyn jj_lib::repo::Repo,
    target_id: &CommitId,
    trunk_id: &CommitId,
) -> Result<Option<CommitId>, JjError> {
    if target_id == trunk_id || !is_ancestor_or_equal_in_repo(repo, trunk_id, target_id)? {
        return Ok(None);
    }

    let mut cursor = load_commit_from_repo(repo, target_id)?;
    let mut first = target_id.clone();
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(cursor.id().clone()) {
            return Ok(None);
        }
        let parents = cursor.parent_ids();
        if parents.len() != 1 {
            return Ok(None);
        }
        let parent_id = &parents[0];
        if parent_id == trunk_id {
            return Ok(Some(first));
        }
        first = parent_id.clone();
        cursor = load_commit_from_repo(repo, parent_id)?;
    }
}

fn bookmark_pull_request_stack_base(
    repo: &dyn jj_lib::repo::Repo,
    target_id: &CommitId,
    trunk: Option<&TrackedPushTrunk>,
) -> Result<Option<BookmarkStackBase>, JjError> {
    let Some(trunk) = trunk else {
        return Ok(None);
    };
    if target_id == &trunk.id || !is_ancestor_or_equal_in_repo(repo, &trunk.id, target_id)? {
        return Ok(None);
    }

    let base = nearest_linear_stack_parent_bookmark(repo, target_id, &trunk.id)?.or_else(|| {
        Some(BookmarkStackBase {
            branch: trunk.branch.clone(),
            id: trunk.id.clone(),
        })
    });
    Ok(base)
}

fn nearest_linear_stack_parent_bookmark(
    repo: &dyn jj_lib::repo::Repo,
    target_id: &CommitId,
    trunk_id: &CommitId,
) -> Result<Option<BookmarkStackBase>, JjError> {
    if target_id == trunk_id || !is_ancestor_or_equal_in_repo(repo, trunk_id, target_id)? {
        return Ok(None);
    }

    let mut cursor = load_commit_from_repo(repo, target_id)?;
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(cursor.id().clone()) {
            return Ok(None);
        }
        let parents = cursor.parent_ids();
        if parents.len() != 1 {
            return Ok(None);
        }
        let parent_id = &parents[0];
        if parent_id == trunk_id {
            return Ok(None);
        }
        if let Some((bookmark, _)) = repo.view().local_bookmarks_for_commit(parent_id).next() {
            return Ok(Some(BookmarkStackBase {
                branch: bookmark.as_str().to_owned(),
                id: parent_id.clone(),
            }));
        }
        cursor = load_commit_from_repo(repo, parent_id)?;
    }
}

fn commit_full_description(
    repo: &dyn jj_lib::repo::Repo,
    commit_id: &CommitId,
) -> Result<Option<String>, JjError> {
    load_commit_from_repo(repo, commit_id)
        .map(|commit| Some(commit.description().trim().to_owned()))
}

pub(super) fn commit_workspace_visibility(
    repo: &dyn jj_lib::repo::Repo,
    commit_id: Option<&CommitId>,
    trunk_id: Option<&CommitId>,
    current_workspace: &WorkspaceName,
) -> Result<WorkspaceVisibility, JjError> {
    let Some(commit_id) = commit_id else {
        return Ok(WorkspaceVisibility::default());
    };

    if let Some(trunk_id) = trunk_id {
        if commit_id == trunk_id || !is_ancestor_or_equal_in_repo(repo, trunk_id, commit_id)? {
            return Ok(WorkspaceVisibility::default());
        }
    }

    let mut current_name = None;
    let mut other_names = Vec::new();
    for (workspace_name, workspace_commit_id) in repo.view().wc_commit_ids() {
        if !is_ancestor_or_equal_in_repo(repo, commit_id, workspace_commit_id)? {
            continue;
        }

        let name = workspace_name.as_symbol().to_string();
        if workspace_name.as_str() == current_workspace.as_str() {
            current_name = Some(name);
        } else {
            other_names.push(name);
        }
    }

    let includes_current = current_name.is_some();
    let names = current_name.into_iter().chain(other_names).collect();
    Ok(WorkspaceVisibility {
        names,
        includes_current,
    })
}

pub(super) fn pushed_commit_summary(commit: &Commit) -> Result<PushedCommitSummary, JjError> {
    Ok(PushedCommitSummary {
        short_commit_id: short_commit_id(commit.id()),
        description: first_description_line(commit.description()).to_owned(),
    })
}

pub(super) fn first_description_line(description: &str) -> &str {
    description
        .lines()
        .find_map(|line| {
            let line = line.trim();
            (!line.is_empty()).then_some(line)
        })
        .unwrap_or("(no description)")
}

pub(super) fn push_origin_bookmark_updates(
    mut_repo: &mut MutableRepo,
    updates: Vec<BookmarkPushUpdate>,
    error_branch: &str,
) -> Result<git::GitPushStats, JjError> {
    let git_settings =
        GitSettings::from_settings(mut_repo.base_repo().settings()).map_err(|error| {
            JjError::Settings {
                message: error.to_string(),
            }
        })?;
    let targets = GitPushRefTargets {
        bookmarks: updates,
        tags: Vec::new(),
    };
    let mut callback = SilentGitCallback;
    let options = GitPushOptions::default();

    git::push_refs(
        mut_repo,
        git_settings.to_subprocess_options(),
        RemoteName::new(ORIGIN_REMOTE_NAME),
        &targets,
        &mut callback,
        &options,
    )
    .map_err(|error| JjError::Push {
        branch: error_branch.to_owned(),
        message: error.to_string(),
    })
}

pub(super) fn push_rejection_message(stats: &git::GitPushStats) -> String {
    let mut parts = Vec::new();

    if !stats.rejected.is_empty() {
        parts.push(format!("{} lease rejected", stats.rejected.len()));
    }
    if !stats.remote_rejected.is_empty() {
        parts.push(format!(
            "{} rejected by remote",
            stats.remote_rejected.len()
        ));
    }
    if !stats.unexported_bookmarks.is_empty() {
        parts.push(format!(
            "{} could not be exported back to jj",
            stats.unexported_bookmarks.len()
        ));
    }

    if parts.is_empty() {
        "push was not accepted".to_owned()
    } else {
        parts.join(", ")
    }
}
