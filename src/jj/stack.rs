use super::*;
use std::time::{Duration, Instant};

impl JjWorkspace {
    /// Moves selected jj revisions, or the current change stack, onto a stack target or trunk.
    pub fn move_stack(
        &mut self,
        revisions: &[String],
        target: StackMoveTarget,
    ) -> Result<StackMoveOutcome, JjError> {
        self.ensure_git_backed()?;

        let current_before = self.current_commit()?;
        let current_before_tree = current_before.tree();
        let target_is_trunk = matches!(target, StackMoveTarget::Trunk);
        let target = match target {
            StackMoveTarget::Onto(target) => self.resolve_stack_move_target(&target)?,
            StackMoveTarget::Trunk => self.resolve_trunk_destination()?.1,
        };
        let target_short_commit_id = short_commit_id(target.id());
        let selection = self.stack_move_selection(revisions, &current_before, &target)?;
        let source_short_commit_ids = selection.source_short_commit_ids();
        if selection.is_empty() {
            return Ok(StackMoveOutcome {
                source_short_commit_ids,
                target_short_commit_id,
                rebased_commits: 0,
                skipped_commits: 0,
                current_updated: false,
            });
        }
        if let StackMoveSelection::CurrentStack { source } = &selection {
            let target_is_descendant = self.is_ancestor_or_equal(source.id(), target.id())?;
            if source.id() == target.id() || (target_is_trunk && target_is_descendant) {
                return Ok(StackMoveOutcome {
                    source_short_commit_ids,
                    target_short_commit_id,
                    rebased_commits: 0,
                    skipped_commits: 1,
                    current_updated: false,
                });
            }
            if target_is_descendant {
                return Err(JjError::StackTargetDescendant);
            }
        }

        let workspace_name = self.workspace.workspace_name().to_owned();
        let mut tx = self.repo.start_transaction();
        let location = selection.location(target.id().clone());
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
                source_short_commit_ids,
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
        let repo = pollster::block_on(tx.commit(stack_move_transaction_description(
            &location.target,
            target.id(),
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
            source_short_commit_ids,
            target_short_commit_id,
            rebased_commits,
            skipped_commits,
            current_updated,
        })
    }

    /// Returns local bookmark ancestry suitable for repairing `jx` stack metadata.
    pub fn local_stack_branches(
        &self,
        stack_base_policy: StackBasePolicy,
    ) -> Result<Vec<LocalStackBranch>, JjError> {
        Ok(self.local_stack_branch_facts(stack_base_policy)?.branches)
    }

    /// Returns local bookmark ancestry and jj-internal timing metrics for performance tracing.
    pub fn local_stack_branch_facts(
        &self,
        stack_base_policy: StackBasePolicy,
    ) -> Result<LocalStackBranchFacts, JjError> {
        self.ensure_git_backed()?;
        let total = Instant::now();
        let mut metrics = LocalStackBranchMetrics::default();

        let started = Instant::now();
        let bookmarks = self
            .repo
            .view()
            .local_bookmarks()
            .map(|(bookmark, ref_target)| (bookmark.as_str().to_owned(), ref_target.clone()))
            .collect::<Vec<_>>();
        metrics.enumerate_bookmarks_us = duration_us(started.elapsed());
        metrics.local_bookmark_count = bookmarks.len();

        let started = Instant::now();
        let fixed_trunk = self.resolve_unanchored_origin_trunk().ok();
        metrics.resolve_trunk_us += duration_us(started.elapsed());
        if fixed_trunk.is_some() {
            metrics.resolved_trunk_count += 1;
        }

        let mut branches = Vec::new();
        for (branch, ref_target) in bookmarks {
            let Some(commit_id) = ref_target.as_normal() else {
                metrics.skipped_non_normal_bookmark_count += 1;
                continue;
            };
            metrics.normal_bookmark_count += 1;

            let started = Instant::now();
            let target = self.load_commit(commit_id)?;
            metrics.load_commit_us += duration_us(started.elapsed());
            metrics.loaded_commit_count += 1;

            let (trunk_branch, stack_base, stack_path) =
                if let Some((trunk_branch, trunk_head)) = &fixed_trunk {
                    let started = Instant::now();
                    let Ok((stack_base, stack_path)) = self.stack_path_from_trunk_head(
                        trunk_branch,
                        trunk_head,
                        &target,
                        stack_base_policy,
                    ) else {
                        metrics.linear_stack_path_us += duration_us(started.elapsed());
                        metrics.skipped_non_linear_count += 1;
                        continue;
                    };
                    metrics.linear_stack_path_us += duration_us(started.elapsed());
                    (trunk_branch.clone(), stack_base, stack_path)
                } else if stack_base_policy.allows_historical_trunk_base() {
                    metrics.skipped_missing_trunk_count += 1;
                    continue;
                } else {
                    let started = Instant::now();
                    let Ok((trunk_branch, trunk)) = self.resolve_trunk(&target) else {
                        metrics.resolve_trunk_us += duration_us(started.elapsed());
                        metrics.skipped_missing_trunk_count += 1;
                        continue;
                    };
                    metrics.resolve_trunk_us += duration_us(started.elapsed());
                    metrics.resolved_trunk_count += 1;

                    let started = Instant::now();
                    let Ok(stack_path) = self.linear_stack_path(&trunk, &target) else {
                        metrics.linear_stack_path_us += duration_us(started.elapsed());
                        metrics.skipped_non_linear_count += 1;
                        continue;
                    };
                    metrics.linear_stack_path_us += duration_us(started.elapsed());
                    (trunk_branch, trunk, stack_path)
                };
            metrics.stack_path_count += 1;
            if stack_path.is_empty() {
                metrics.skipped_trunk_count += 1;
                continue;
            }

            let started = Instant::now();
            let parent_branch = self.nearest_ancestor_bookmark(&stack_base, &stack_path);
            metrics.nearest_ancestor_bookmark_us += duration_us(started.elapsed());
            let base_branch = parent_branch
                .clone()
                .unwrap_or_else(|| trunk_branch.clone());
            branches.push(LocalStackBranch {
                branch,
                base_branch,
                parent_branch,
                title: first_description_line(target.description()).to_owned(),
                commit_id: target.id().hex(),
            });
        }

        let started = Instant::now();
        branches.sort_by(|left, right| left.branch.cmp(&right.branch));
        branches.dedup_by(|left, right| left.branch == right.branch);
        metrics.sort_dedup_us = duration_us(started.elapsed());
        metrics.branch_count = branches.len();
        metrics.total_us = duration_us(total.elapsed());
        Ok(LocalStackBranchFacts { branches, metrics })
    }

    /// Returns local stack facts for either inferred full-stack or explicit-revset publishing.
    pub fn stack_publish_facts(
        &self,
        selection: &StackPublishSelection,
        stack_base_policy: StackBasePolicy,
    ) -> Result<StackPublishFacts, JjError> {
        self.ensure_git_backed()?;
        let total = Instant::now();
        let mut metrics = StackPublishMetrics::default();
        let mut facts = match selection {
            StackPublishSelection::InferredStack { anchor } => self.inferred_stack_publish_facts(
                anchor.as_deref(),
                stack_base_policy,
                &mut metrics,
            ),
            StackPublishSelection::ExplicitRevisions { revisions } => {
                self.explicit_stack_publish_facts(revisions, stack_base_policy, &mut metrics)
            }
        }?;
        metrics.node_count = facts.nodes.len();
        metrics.publish_count = facts.publish_indexes.len();
        metrics.total_us = duration_us(total.elapsed());
        facts.metrics = metrics;
        Ok(facts)
    }

    /// Returns read-only local stack neighbourhood facts for stack planning.
    pub fn stack_plan_facts(
        &self,
        selection: &StackPlanSelection,
        stack_base_policy: StackBasePolicy,
    ) -> Result<StackPlanFacts, JjError> {
        self.ensure_git_backed()?;
        match selection {
            StackPlanSelection::InferredStack { anchor } => {
                let anchor = self.target_for_revision(anchor.as_deref())?;
                let (trunk, root) = self.stack_plan_root(&anchor, stack_base_policy)?;
                let (nodes, indexes_by_commit) =
                    self.stack_plan_neighbourhood(root, stack_base_policy)?;
                let anchor_index = indexes_by_commit.get(anchor.id()).copied();
                Ok(StackPlanFacts {
                    trunk,
                    selected_indexes: (0..nodes.len()).collect(),
                    nodes,
                    anchor_index,
                })
            }
            StackPlanSelection::ExplicitRevisions { revisions } => {
                self.explicit_stack_plan_facts(revisions, stack_base_policy)
            }
        }
    }

    fn explicit_stack_plan_facts(
        &self,
        revisions: &[String],
        stack_base_policy: StackBasePolicy,
    ) -> Result<StackPlanFacts, JjError> {
        let selected = self.resolve_publish_revisions(revisions)?;
        if selected.is_empty() {
            return Err(JjError::EmptyStackPublishSelection);
        }

        let mut selected_ids = BTreeSet::new();
        let mut trunk: Option<TrunkSummary> = None;
        let mut root: Option<Commit> = None;
        for commit in selected {
            selected_ids.insert(commit.id().clone());
            let (next_trunk, next_root) = self.stack_plan_root(&commit, stack_base_policy)?;
            match root.as_ref() {
                Some(root) if root.id() != next_root.id() => {
                    return Err(JjError::StackPublishMultipleStacks);
                }
                Some(_) => {}
                None => {
                    trunk = Some(next_trunk);
                    root = Some(next_root);
                }
            }
        }

        let root = root.ok_or(JjError::EmptyStackPublishSelection)?;
        let (nodes, indexes_by_commit) = self.stack_plan_neighbourhood(root, stack_base_policy)?;
        let mut selected_indexes = Vec::new();
        for commit_id in &selected_ids {
            let Some(index) = indexes_by_commit.get(commit_id).copied() else {
                return Err(JjError::StackPublishNonLinearSelection);
            };
            selected_indexes.push(index);
        }
        selected_indexes.sort_unstable();

        Ok(StackPlanFacts {
            trunk: trunk.ok_or(JjError::EmptyStackPublishSelection)?,
            nodes,
            selected_indexes,
            anchor_index: None,
        })
    }

    fn stack_plan_root(
        &self,
        target: &Commit,
        stack_base_policy: StackBasePolicy,
    ) -> Result<(TrunkSummary, Commit), JjError> {
        let (branch, stack_base, path) = self.stack_path_for_target(target, stack_base_policy)?;
        let Some(root) = path.first() else {
            return Err(JjError::EmptyStackPublishSelection);
        };

        Ok((
            TrunkSummary {
                branch,
                commit_id: stack_base.id().hex(),
                short_commit_id: short_commit_id(stack_base.id()),
            },
            root.clone(),
        ))
    }

    fn stack_plan_neighbourhood(
        &self,
        root: Commit,
        stack_base_policy: StackBasePolicy,
    ) -> Result<(Vec<StackPlanNodeFacts>, BTreeMap<CommitId, usize>), JjError> {
        let mut nodes = Vec::new();
        let mut indexes_by_commit = BTreeMap::new();
        self.append_stack_plan_node(
            root,
            None,
            stack_base_policy,
            &mut nodes,
            &mut indexes_by_commit,
        )?;
        Ok((nodes, indexes_by_commit))
    }

    fn append_stack_plan_node(
        &self,
        commit: Commit,
        parent_index: Option<usize>,
        stack_base_policy: StackBasePolicy,
        nodes: &mut Vec<StackPlanNodeFacts>,
        indexes_by_commit: &mut BTreeMap<CommitId, usize>,
    ) -> Result<(), JjError> {
        if indexes_by_commit.contains_key(commit.id()) {
            return Err(JjError::NonLinearStack {
                message: format!(
                    "cycle detected while walking stack neighbourhood from {}",
                    short_commit_id(commit.id())
                ),
            });
        }

        let index = nodes.len();
        indexes_by_commit.insert(commit.id().clone(), index);
        nodes.push(StackPlanNodeFacts {
            workspace: self.facts_for_commit(commit.clone(), stack_base_policy)?,
            parent_index,
        });

        let mut children = collect_child_ids(self.repo.as_ref(), commit.id())?
            .into_iter()
            .map(|child_id| self.load_commit(&child_id))
            .collect::<Result<Vec<_>, _>>()?;
        children.sort_by(|left, right| {
            stack_plan_commit_sort_key(left).cmp(&stack_plan_commit_sort_key(right))
        });
        for child in children {
            match stack_child_relationship(&child, commit.id()) {
                StackChildRelationship::Linear => {
                    self.append_stack_plan_node(
                        child,
                        Some(index),
                        stack_base_policy,
                        nodes,
                        indexes_by_commit,
                    )?;
                }
                StackChildRelationship::Boundary => {
                    // Merge children depend on this commit but are not part of its single-parent stack.
                }
                StackChildRelationship::Unrelated => {
                    return Err(JjError::NonLinearStack {
                        message: format!(
                            "commit {} is not a child of {}",
                            short_commit_id(child.id()),
                            short_commit_id(commit.id())
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    fn inferred_stack_publish_facts(
        &self,
        anchor: Option<&str>,
        stack_base_policy: StackBasePolicy,
        metrics: &mut StackPublishMetrics,
    ) -> Result<StackPublishFacts, JjError> {
        let started = Instant::now();
        let anchor = self.target_for_revision(anchor)?;
        metrics.target_resolution_us += duration_us(started.elapsed());
        metrics.target_resolution_count += 1;

        let (trunk_branch, stack_base, mut path) =
            self.publish_stack_path_for_target(&anchor, stack_base_policy, metrics)?;
        if path.is_empty() {
            return Err(JjError::EmptyStackPublishSelection);
        }
        let anchor_index = path.len() - 1;

        let mut cursor = anchor;
        loop {
            let started = Instant::now();
            let children = collect_child_ids(self.repo.as_ref(), cursor.id())?;
            metrics.collect_child_ids_us += duration_us(started.elapsed());
            metrics.collected_child_count += children.len();

            let mut stack_children = Vec::new();
            for child_id in children {
                let started = Instant::now();
                let child = self.load_commit(&child_id)?;
                metrics.load_child_commit_us += duration_us(started.elapsed());
                metrics.loaded_child_count += 1;

                match stack_child_relationship(&child, cursor.id()) {
                    StackChildRelationship::Linear => stack_children.push(child),
                    StackChildRelationship::Boundary => {
                        // Merge children are stack boundaries, not continuations to publish.
                    }
                    StackChildRelationship::Unrelated => {
                        return Err(JjError::NonLinearStack {
                            message: format!(
                                "commit {} is not a child of {}",
                                short_commit_id(child.id()),
                                short_commit_id(cursor.id())
                            ),
                        });
                    }
                }
            }

            match stack_children.as_slice() {
                [] => break,
                [child] => {
                    let child = child.clone();
                    path.push(child.clone());
                    cursor = child;
                }
                _ => {
                    return Err(JjError::NonLinearStack {
                        message: format!(
                            "commit {} has {} single-parent children; expected a single linear stack",
                            short_commit_id(cursor.id()),
                            stack_children.len()
                        ),
                    });
                }
            }
        }

        self.stack_publish_facts_from_path(
            path,
            None,
            Some(anchor_index),
            &trunk_branch,
            &stack_base,
            metrics,
        )
    }

    fn explicit_stack_publish_facts(
        &self,
        revisions: &[String],
        stack_base_policy: StackBasePolicy,
        metrics: &mut StackPublishMetrics,
    ) -> Result<StackPublishFacts, JjError> {
        let started = Instant::now();
        let selected = self.resolve_publish_revisions(revisions)?;
        metrics.resolve_revisions_us += duration_us(started.elapsed());
        metrics.resolved_revision_count = selected.len();
        if selected.is_empty() {
            return Err(JjError::EmptyStackPublishSelection);
        }

        let mut selected_ids = BTreeSet::new();
        let mut root_id = None;
        let mut longest_path = Vec::new();
        let mut longest_trunk = None;
        for commit in selected {
            selected_ids.insert(commit.id().clone());
            let (trunk_branch, stack_base, path) =
                self.publish_stack_path_for_target(&commit, stack_base_policy, metrics)?;
            let Some(root) = path.first() else {
                return Err(JjError::EmptyStackPublishSelection);
            };
            match &root_id {
                Some(root_id) if root_id != root.id() => {
                    return Err(JjError::StackPublishMultipleStacks);
                }
                Some(_) => {}
                None => root_id = Some(root.id().clone()),
            }
            if path.len() > longest_path.len() {
                longest_path = path;
                longest_trunk = Some((trunk_branch, stack_base));
            }
        }

        let path_ids = longest_path
            .iter()
            .map(|commit| commit.id().clone())
            .collect::<BTreeSet<_>>();
        if !selected_ids
            .iter()
            .all(|commit_id| path_ids.contains(commit_id))
        {
            return Err(JjError::StackPublishNonLinearSelection);
        }

        let Some((trunk_branch, stack_base)) = longest_trunk else {
            return Err(JjError::EmptyStackPublishSelection);
        };
        self.stack_publish_facts_from_path(
            longest_path,
            Some(&selected_ids),
            None,
            &trunk_branch,
            &stack_base,
            metrics,
        )
    }

    fn resolve_publish_revisions(&self, revisions: &[String]) -> Result<Vec<Commit>, JjError> {
        let mut commits = Vec::new();
        let mut seen = BTreeSet::new();
        for revision in revisions {
            for commit in self.resolve_stack_publish_revision(revision)? {
                if seen.insert(commit.id().clone()) {
                    commits.push(commit);
                }
            }
        }
        Ok(commits)
    }

    fn resolve_stack_publish_revision(&self, revision: &str) -> Result<Vec<Commit>, JjError> {
        match self.resolve_revisions(revision, "In selected jj revision") {
            Ok(commits) => Ok(commits),
            Err(error) if can_try_stack_bookmark_fragment(&error) => {
                let branch = self.resolve_local_bookmark_fragment(revision)?;
                Ok(vec![self.resolve_single_revision(
                    &branch,
                    "In selected jj revision",
                )?])
            }
            Err(error) => Err(error),
        }
    }

    fn publish_stack_path_for_target(
        &self,
        target: &Commit,
        stack_base_policy: StackBasePolicy,
        metrics: &mut StackPublishMetrics,
    ) -> Result<(String, Commit, Vec<Commit>), JjError> {
        let started = Instant::now();
        let unanchored_trunk = self.resolve_unanchored_origin_trunk();
        metrics.resolve_trunk_us += duration_us(started.elapsed());
        match unanchored_trunk {
            Ok((trunk_branch, trunk_head)) => {
                let started = Instant::now();
                let (stack_base, path) = self.stack_path_from_trunk_head(
                    &trunk_branch,
                    &trunk_head,
                    target,
                    stack_base_policy,
                )?;
                metrics.linear_stack_path_us += duration_us(started.elapsed());
                metrics.resolved_trunk_count += 1;
                metrics.stack_path_count += 1;
                Ok((trunk_branch, stack_base, path))
            }
            Err(error) if stack_base_policy.allows_historical_trunk_base() => Err(error),
            Err(_) => {
                let started = Instant::now();
                let (trunk_branch, trunk) = self.resolve_trunk(target)?;
                metrics.resolve_trunk_us += duration_us(started.elapsed());
                metrics.resolved_trunk_count += 1;

                let started = Instant::now();
                let path = self.linear_stack_path(&trunk, target)?;
                metrics.linear_stack_path_us += duration_us(started.elapsed());
                metrics.stack_path_count += 1;
                Ok((trunk_branch, trunk, path))
            }
        }
    }

    fn stack_publish_workspace_facts(
        &self,
        trunk_branch: &str,
        stack_base: &Commit,
        stack_path: &[Commit],
        local_bookmarks: &[String],
    ) -> Result<WorkspaceFacts, JjError> {
        let Some(target) = stack_path.last() else {
            return Err(JjError::EmptyStackPublishSelection);
        };
        self.facts_for_stack_path(
            target,
            trunk_branch,
            stack_base,
            stack_path,
            local_bookmarks,
        )
    }

    fn stack_publish_facts_from_path(
        &self,
        path: Vec<Commit>,
        publish_ids: Option<&BTreeSet<CommitId>>,
        anchor_index: Option<usize>,
        trunk_branch: &str,
        stack_base: &Commit,
        metrics: &mut StackPublishMetrics,
    ) -> Result<StackPublishFacts, JjError> {
        let local_bookmarks = self.local_bookmarks();
        let mut nodes = Vec::new();
        let mut publish_indexes = Vec::new();
        for index in 0..path.len() {
            let commit = &path[index];
            if publish_ids.is_none_or(|ids| ids.contains(commit.id())) {
                publish_indexes.push(index);
            }
            let started = Instant::now();
            let workspace = self.stack_publish_workspace_facts(
                trunk_branch,
                stack_base,
                &path[..=index],
                &local_bookmarks,
            )?;
            metrics.workspace_facts_us += duration_us(started.elapsed());
            metrics.workspace_fact_count += 1;
            nodes.push(StackPublishNodeFacts {
                workspace,
                parent_index: index.checked_sub(1),
            });
        }
        if publish_indexes.is_empty() {
            return Err(JjError::EmptyStackPublishSelection);
        }

        Ok(StackPublishFacts {
            nodes,
            publish_indexes,
            anchor_index,
            metrics: StackPublishMetrics::default(),
        })
    }

    fn stack_move_selection(
        &self,
        revisions: &[String],
        current_before: &Commit,
        target: &Commit,
    ) -> Result<StackMoveSelection, JjError> {
        if revisions.is_empty() {
            return Ok(StackMoveSelection::CurrentStack {
                source: current_before.clone(),
            });
        }

        let commits = self.resolve_stack_move_revisions(revisions)?;
        if commits.is_empty() {
            return Err(JjError::RevisionNotFound {
                revision: revisions.join(" | "),
            });
        }
        if commits.iter().any(|commit| commit.id() == target.id()) {
            return Err(JjError::StackSourceIsTarget {
                commit_id: short_commit_id(target.id()),
            });
        }
        Ok(StackMoveSelection::ExactRevisions { commits })
    }

    fn resolve_stack_move_target(&self, target: &str) -> Result<Commit, JjError> {
        self.resolve_stack_move_revision(target, "In `jx stack --onto`")
    }

    fn resolve_stack_move_revisions(&self, revisions: &[String]) -> Result<Vec<Commit>, JjError> {
        let mut commit_ids = Vec::new();
        let mut seen = BTreeSet::new();
        for revision in revisions {
            for commit in self.resolve_stack_move_revision_set(revision)? {
                if seen.insert(commit.id().clone()) {
                    commit_ids.push(commit.id().clone());
                }
            }
        }
        self.order_stack_move_commits(commit_ids)
    }

    fn resolve_stack_move_revision_set(&self, revision: &str) -> Result<Vec<Commit>, JjError> {
        match self.resolve_revisions(revision, "In `jx stack --revision`") {
            Ok(commits) => Ok(commits),
            Err(error) if can_try_stack_bookmark_fragment(&error) => {
                let branch = self.resolve_local_bookmark_fragment(revision)?;
                Ok(vec![self.resolve_single_revision(
                    &branch,
                    "In `jx stack --revision`",
                )?])
            }
            Err(error) => Err(error),
        }
    }

    fn order_stack_move_commits(&self, commit_ids: Vec<CommitId>) -> Result<Vec<Commit>, JjError> {
        if commit_ids.is_empty() {
            return Ok(Vec::new());
        }

        let revset = ResolvedRevsetExpression::commits(commit_ids)
            .evaluate(self.repo.as_ref())
            .map_err(|error| JjError::Backend {
                message: error.into_backend_error().to_string(),
            })?;
        let commit_ids =
            pollster::block_on(revset.stream().try_collect::<Vec<_>>()).map_err(|error| {
                JjError::Backend {
                    message: error.into_backend_error().to_string(),
                }
            })?;
        commit_ids
            .iter()
            .map(|commit_id| self.load_commit(commit_id))
            .collect()
    }

    fn resolve_stack_move_revision(
        &self,
        revision: &str,
        diagnostics_source: &'static str,
    ) -> Result<Commit, JjError> {
        match self.resolve_single_revision(revision, diagnostics_source) {
            Ok(commit) => Ok(commit),
            Err(error) if can_try_stack_bookmark_fragment(&error) => {
                let branch = self.resolve_local_bookmark_fragment(revision)?;
                self.resolve_single_revision(&branch, diagnostics_source)
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

#[derive(Debug, Clone)]
enum StackMoveSelection {
    CurrentStack { source: Commit },
    ExactRevisions { commits: Vec<Commit> },
}

impl StackMoveSelection {
    fn is_empty(&self) -> bool {
        match self {
            StackMoveSelection::CurrentStack { .. } => false,
            StackMoveSelection::ExactRevisions { commits } => commits.is_empty(),
        }
    }

    fn source_short_commit_ids(&self) -> Vec<String> {
        match self {
            StackMoveSelection::CurrentStack { source } => vec![short_commit_id(source.id())],
            StackMoveSelection::ExactRevisions { commits } => commits
                .iter()
                .map(|commit| short_commit_id(commit.id()))
                .collect(),
        }
    }

    fn location(&self, new_parent_id: CommitId) -> MoveCommitsLocation {
        let (target, new_parent_ids) = match self {
            StackMoveSelection::CurrentStack { source } => (
                MoveCommitsTarget::Roots(vec![source.id().clone()]),
                vec![new_parent_id],
            ),
            StackMoveSelection::ExactRevisions { commits } => (
                MoveCommitsTarget::Commits(
                    commits.iter().map(|commit| commit.id().clone()).collect(),
                ),
                vec![new_parent_id],
            ),
        };
        MoveCommitsLocation {
            new_parent_ids,
            new_child_ids: Vec::new(),
            target,
        }
    }
}

fn stack_move_transaction_description(
    target: &MoveCommitsTarget,
    destination: &CommitId,
) -> String {
    match target {
        MoveCommitsTarget::Commits(ids) => match ids.as_slice() {
            [] => "jx stack move 0 revisions".to_owned(),
            [id] => format!(
                "jx stack move revision {} onto {}",
                id.hex(),
                destination.hex()
            ),
            [first, others @ ..] => format!(
                "jx stack move revision {} and {} more onto {}",
                first.hex(),
                others.len(),
                destination.hex()
            ),
        },
        MoveCommitsTarget::Roots(ids) => match ids.as_slice() {
            [id] => format!("jx stack move {} onto {}", id.hex(), destination.hex()),
            _ => format!(
                "jx stack move {} stack roots onto {}",
                ids.len(),
                destination.hex()
            ),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackChildRelationship {
    Linear,
    Boundary,
    Unrelated,
}

fn stack_child_relationship(child: &Commit, parent_id: &CommitId) -> StackChildRelationship {
    let parents = child.parent_ids();
    if parents.len() == 1 && parents[0] == *parent_id {
        StackChildRelationship::Linear
    } else if parents.iter().any(|id| id == parent_id) {
        StackChildRelationship::Boundary
    } else {
        StackChildRelationship::Unrelated
    }
}

fn stack_plan_commit_sort_key(commit: &Commit) -> (String, String) {
    (
        first_description_line(commit.description()).to_owned(),
        commit.id().hex(),
    )
}

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
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
