use super::*;

impl JjWorkspace {
    /// Moves the working copy to its single editable parent and renders the surrounding chain.
    pub fn move_to_previous_commit_and_render_log(current_dir: &Path) -> Result<String, JjError> {
        let workspace_root = find_jj_workspace_root(current_dir)?;
        let mut workspace = Self::load(workspace_root)?;
        workspace.move_to_previous_commit()?;
        workspace.render_navigation_log(current_dir)
    }

    /// Moves the working copy to its single editable child and renders the surrounding chain.
    pub fn move_to_next_commit_and_render_log(current_dir: &Path) -> Result<String, JjError> {
        let workspace_root = find_jj_workspace_root(current_dir)?;
        let mut workspace = Self::load(workspace_root)?;
        workspace.move_to_next_commit()?;
        workspace.render_navigation_log(current_dir)
    }

    /// Moves the active workspace to the current commit's single non-root parent.
    pub fn move_to_previous_commit(&mut self) -> Result<(), JjError> {
        let current = self.current_commit()?;
        let parents = current.parent_ids();
        let [parent_id] = parents else {
            return Err(match parents.len() {
                0 => JjError::NoPreviousCommit,
                count => JjError::AmbiguousPreviousCommit { count },
            });
        };
        if parent_id == self.repo.store().root_commit_id() {
            return Err(JjError::NoPreviousCommit);
        }

        let target = self.load_commit(parent_id)?;
        if self.is_immutable_commit(&target)? {
            return Err(JjError::NoPreviousCommit);
        }

        self.move_to_commit(&target, "previous")
    }

    /// Moves the active workspace to the current commit's single child.
    pub fn move_to_next_commit(&mut self) -> Result<(), JjError> {
        let current = self.current_commit()?;
        let children = collect_child_ids(self.repo.as_ref(), current.id())?;
        let [child_id] = children.as_slice() else {
            return Err(match children.len() {
                0 => JjError::NoNextCommit,
                count => JjError::AmbiguousNextCommit { count },
            });
        };

        let target = self.load_commit(child_id)?;
        if self.is_immutable_commit(&target)? {
            return Err(JjError::NoNextCommit);
        }

        self.move_to_commit(&target, "next")
    }

    /// Renders the single-parent chain around the working copy, including linear descendants.
    pub fn render_navigation_log(&self, current_dir: &Path) -> Result<String, JjError> {
        let current = self.current_commit()?;
        render_commit_ids_log(
            &self.workspace,
            self.repo.as_ref(),
            current_dir,
            self.navigation_chain_ids(&current)?,
        )
    }

    fn move_to_commit(&mut self, target: &Commit, direction: &'static str) -> Result<(), JjError> {
        let current_before = self.current_commit()?;
        let current_before_tree = current_before.tree();
        let workspace_name = self.workspace.workspace_name().to_owned();
        let mut tx = self.repo.start_transaction();
        pollster::block_on(tx.repo_mut().edit(workspace_name, target)).map_err(|error| {
            JjError::CommitNavigation {
                direction,
                message: error.to_string(),
            }
        })?;
        let target_id = target.id().clone();
        let repo =
            pollster::block_on(tx.commit(format!("jx {direction}-commit"))).map_err(|error| {
                JjError::Transaction {
                    message: error.to_string(),
                }
            })?;
        let target = load_commit_from_repo(repo.as_ref(), &target_id)?;
        pollster::block_on(self.workspace.check_out(
            repo.op_id().clone(),
            Some(&current_before_tree),
            &target,
        ))
        .map_err(|error| JjError::WorkingCopyCheckout {
            message: error.to_string(),
        })?;
        self.repo = repo;
        Ok(())
    }

    fn navigation_chain_ids(&self, current: &Commit) -> Result<Vec<CommitId>, JjError> {
        let mut ids = Vec::new();
        self.collect_linear_descendant_ids(current, &mut ids)?;
        self.collect_unpublished_ancestor_ids(current, &mut ids)?;
        Ok(ids)
    }

    fn collect_linear_descendant_ids(
        &self,
        current: &Commit,
        ids: &mut Vec<CommitId>,
    ) -> Result<(), JjError> {
        let mut cursor = current.clone();
        ids.push(cursor.id().clone());

        loop {
            let children = collect_child_ids(self.repo.as_ref(), cursor.id())?;
            let [child_id] = children.as_slice() else {
                return Ok(());
            };
            let child = self.load_commit(child_id)?;
            ids.push(child.id().clone());
            cursor = child;
        }
    }

    fn collect_unpublished_ancestor_ids(
        &self,
        current: &Commit,
        ids: &mut Vec<CommitId>,
    ) -> Result<(), JjError> {
        let mut cursor = current.clone();

        loop {
            let parents = cursor.parent_ids();
            let [parent_id] = parents else {
                return Ok(());
            };
            if parent_id == self.repo.store().root_commit_id() {
                return Ok(());
            }

            let parent = self.load_commit(parent_id)?;
            let is_immutable = self.is_immutable_commit(&parent)?;
            ids.push(parent.id().clone());
            if is_immutable {
                return Ok(());
            }
            cursor = parent;
        }
    }

    fn is_immutable_commit(&self, commit: &Commit) -> Result<bool, JjError> {
        let ui = Ui::null();
        let settings = self.workspace.settings();
        let fileset_aliases_map =
            load_fileset_aliases(&ui, settings.config()).map_err(log_command_error)?;
        let revset_aliases_map =
            load_revset_aliases(&ui, settings.config()).map_err(log_command_error)?;
        let revset_extensions = Arc::new(RevsetExtensions::default());
        let path_converter = RepoPathUiConverter::Fs {
            cwd: self.workspace.workspace_root().to_path_buf(),
            base: self.workspace.workspace_root().to_path_buf(),
        };
        let workspace_context = RevsetWorkspaceContext {
            path_converter: &path_converter,
            workspace_name: self.workspace.workspace_name(),
        };
        let revset_context = revset_parse_context(
            settings,
            self.repo.as_ref(),
            &fileset_aliases_map,
            &revset_aliases_map,
            &revset_extensions,
            Some(workspace_context),
        )?;
        let id_prefix_context =
            log_id_prefix_context(settings, &ui, &revset_context, revset_extensions.clone())?;
        let expression = immutable_expression(&ui, &revset_context)?
            .intersection(&RevsetExpression::commit(commit.id().clone()));
        let evaluator = RevsetExpressionEvaluator::new(
            self.repo.as_ref(),
            revset_extensions,
            &id_prefix_context,
            expression,
        );
        let mut commits = evaluator.evaluate_to_commit_ids().map_err(log_error)?;

        Ok(pollster::block_on(commits.try_next())
            .map_err(log_error)?
            .is_some())
    }
}
