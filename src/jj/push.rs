use super::*;

impl JjWorkspace {
    /// Pushes `branch` to fixed `origin` and updates jj's remote bookmark view.
    pub fn push_bookmark(&mut self, branch: &str) -> Result<PushOutcome, JjError> {
        self.ensure_git_backed()?;

        let bookmark = RefName::new(branch);
        let targets = self.local_and_origin_bookmark_targets(bookmark);
        if targets.local_target.has_conflict() {
            return Err(JjError::ConflictedBookmark {
                branch: branch.to_owned(),
            });
        }
        if targets.local_target.as_normal().is_none() {
            return Err(JjError::MissingLocalBookmark {
                branch: branch.to_owned(),
            });
        }

        let Some(update) = classify_push_bookmark_update(
            bookmark.to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME)),
            targets,
            true,
            false,
        )?
        else {
            return Ok(PushOutcome {
                branch: branch.to_owned(),
                pushed_refs: 0,
                pushed_commits: Vec::new(),
            });
        };
        let updates = vec![(RefNameBuf::from(branch), update)];
        let pushed_commits = self.pushed_commits_for_updates(&updates)?;
        let push_stats = self.push_origin_bookmark_updates(
            updates,
            format!("jx push {branch}"),
            branch.to_owned(),
        )?;

        Ok(PushOutcome {
            branch: branch.to_owned(),
            pushed_refs: push_stats.pushed.len(),
            pushed_commits,
        })
    }

    /// Pushes all tracked fixed-origin bookmarks, including local deletions.
    pub fn push_tracked_deleted(&mut self) -> Result<TrackedPushOutcome, JjError> {
        self.ensure_git_backed()?;

        let updates = self.tracked_origin_bookmark_updates()?;
        if updates.is_empty() {
            return Ok(TrackedPushOutcome {
                pushed_refs: 0,
                bookmarks: Vec::new(),
                pushed_commits: Vec::new(),
            });
        }

        let trunk_id = self.current_commit().ok().and_then(|current| {
            self.resolve_trunk(&current)
                .ok()
                .map(|(_, trunk)| trunk.id().clone())
        });
        let bookmarks = pushed_bookmark_summaries(
            self.repo.as_ref(),
            &updates,
            trunk_id.as_ref(),
            self.workspace.workspace_name(),
        )?;
        let pushed_commits = self.pushed_commits_for_updates(&updates)?;
        let push_stats = self.push_origin_bookmark_updates(
            updates,
            "jx push tracked bookmarks".to_owned(),
            "tracked bookmarks".to_owned(),
        )?;

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
    trunk_id: Option<&CommitId>,
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
                new_workspace_visibility: commit_workspace_visibility(
                    repo,
                    update.after.as_ref(),
                    trunk_id,
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
