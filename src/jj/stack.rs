use super::*;

impl JjWorkspace {
    /// Moves the current change and its descendants onto a stack target or trunk.
    pub fn move_current_stack(
        &mut self,
        target: StackMoveTarget,
    ) -> Result<StackMoveOutcome, JjError> {
        self.ensure_git_backed()?;

        let current_before = self.current_commit()?;
        let current_before_tree = current_before.tree();
        let target = match target {
            StackMoveTarget::Onto(target) => self.resolve_stack_move_target(&target)?,
            StackMoveTarget::Trunk => self.resolve_trunk(&current_before)?.1,
        };

        let source_short_commit_id = short_commit_id(current_before.id());
        let target_short_commit_id = short_commit_id(target.id());
        if current_before.id() == target.id() {
            return Ok(StackMoveOutcome {
                source_short_commit_id,
                target_short_commit_id,
                rebased_commits: 0,
                skipped_commits: 1,
                current_updated: false,
            });
        }
        if self.is_ancestor_or_equal(current_before.id(), target.id())? {
            return Err(JjError::StackTargetDescendant);
        }

        let workspace_name = self.workspace.workspace_name().to_owned();
        let mut tx = self.repo.start_transaction();
        let location = MoveCommitsLocation {
            new_parent_ids: vec![target.id().clone()],
            new_child_ids: Vec::new(),
            target: MoveCommitsTarget::Roots(vec![current_before.id().clone()]),
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
        let skipped_commits = stats.num_skipped_rebases as usize;

        if rebased_commits == 0 && stats.num_abandoned_empty == 0 {
            return Ok(StackMoveOutcome {
                source_short_commit_id,
                target_short_commit_id,
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
            "jx stack move {} onto {}",
            current_before.id().hex(),
            target.id().hex()
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

        Ok(StackMoveOutcome {
            source_short_commit_id,
            target_short_commit_id,
            rebased_commits,
            skipped_commits,
            current_updated,
        })
    }

    /// Returns local bookmark ancestry suitable for repairing `jx` stack metadata.
    pub fn local_stack_branches(&self) -> Result<Vec<LocalStackBranch>, JjError> {
        self.ensure_git_backed()?;

        let mut branches = Vec::new();
        for (bookmark, ref_target) in self.repo.view().local_bookmarks() {
            let Some(commit_id) = ref_target.as_normal() else {
                continue;
            };
            let branch = bookmark.as_str().to_owned();
            let target = self.load_commit(commit_id)?;
            let Ok((trunk_branch, trunk)) = self.resolve_trunk(&target) else {
                continue;
            };
            let Ok(stack_path) = self.linear_stack_path(&trunk, &target) else {
                continue;
            };
            if stack_path.is_empty() {
                continue;
            }
            let parent_branch = self.nearest_ancestor_bookmark(&trunk, &stack_path);
            let base_branch = parent_branch
                .clone()
                .unwrap_or_else(|| trunk_branch.clone());
            branches.push(LocalStackBranch {
                branch,
                base_branch,
                parent_branch,
                title: first_description_line(target.description()).to_owned(),
            });
        }
        branches.sort_by(|left, right| left.branch.cmp(&right.branch));
        branches.dedup_by(|left, right| left.branch == right.branch);
        Ok(branches)
    }

    fn resolve_stack_move_target(&self, target: &str) -> Result<Commit, JjError> {
        match self.resolve_single_revision(target, "In `jx stack --onto`") {
            Ok(commit) => Ok(commit),
            Err(error) if can_try_stack_bookmark_fragment(&error) => {
                let branch = self.resolve_local_bookmark_fragment(target)?;
                self.resolve_single_revision(&branch, "In `jx stack --onto`")
            }
            Err(error) => Err(error),
        }
    }

    fn resolve_local_bookmark_fragment(&self, target: &str) -> Result<String, JjError> {
        let mut matches = self
            .repo
            .view()
            .local_bookmarks()
            .filter(|(_, ref_target)| ref_target.as_normal().is_some())
            .filter_map(|(bookmark, _)| {
                let branch = bookmark.as_str();
                stack_bookmark_match_rank(branch, target).map(|rank| (branch.to_owned(), rank))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| (left.1, &left.0).cmp(&(right.1, &right.0)));

        let Some(best_rank) = matches.first().map(|(_, rank)| *rank) else {
            return Err(JjError::StackTargetNotFound {
                target: target.to_owned(),
            });
        };
        let best_matches = matches
            .into_iter()
            .filter(|(_, rank)| *rank == best_rank)
            .map(|(branch, _)| branch)
            .collect::<Vec<_>>();

        match best_matches.as_slice() {
            [branch] => Ok(branch.clone()),
            _ => Err(JjError::StackTargetAmbiguous {
                target: target.to_owned(),
                matches: best_matches,
            }),
        }
    }
}

fn can_try_stack_bookmark_fragment(error: &JjError) -> bool {
    matches!(
        error,
        JjError::Revision { .. } | JjError::RevisionNotFound { .. }
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum StackBookmarkMatchRank {
    Exact,
    Prefix,
    Contains,
}

fn stack_bookmark_match_rank(candidate: &str, query: &str) -> Option<StackBookmarkMatchRank> {
    if candidate == query {
        Some(StackBookmarkMatchRank::Exact)
    } else if candidate.starts_with(query) {
        Some(StackBookmarkMatchRank::Prefix)
    } else if candidate.contains(query) {
        Some(StackBookmarkMatchRank::Contains)
    } else {
        None
    }
}
