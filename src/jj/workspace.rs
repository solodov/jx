use super::*;

/// High-level jj workspace wrapper used by command/domain services.
pub struct JjWorkspace {
    pub(super) workspace: Workspace,
    pub(super) repo: Arc<ReadonlyRepo>,
}

impl JjWorkspace {
    /// Loads a jj workspace rooted at `workspace_root` using the resolved jj config.
    pub fn load(workspace_root: impl Into<PathBuf>) -> Result<Self, JjError> {
        let workspace_root = workspace_root.into();
        let ui = Ui::null();
        let loader = DefaultWorkspaceLoaderFactory
            .create(&workspace_root)
            .map_err(workspace_load_error)?;
        let config = resolved_workspace_config_for_workspace_load(&ui, loader.as_ref())?;
        let settings = UserSettings::from_config(config).map_err(|error| JjError::Settings {
            message: error.to_string(),
        })?;
        let store_factories = StoreFactories::default();
        let working_copy_factories = default_working_copy_factories();
        let workspace = loader
            .load(&settings, &store_factories, &working_copy_factories)
            .map_err(|error| JjError::WorkspaceLoad {
                message: error.to_string(),
            })?;
        let repo = pollster::block_on(workspace.repo_loader().load_at_head()).map_err(|error| {
            JjError::RepoLoad {
                message: error.to_string(),
            }
        })?;

        Ok(Self { workspace, repo })
    }

    /// Returns the workspace root reported by jj.
    pub fn workspace_root(&self) -> PathBuf {
        self.workspace.workspace_root().to_path_buf()
    }

    /// Returns the jj workspace name for current-workspace rendering and safety checks.
    pub fn workspace_name(&self) -> String {
        self.workspace.workspace_name().as_str().to_owned()
    }

    pub(super) fn current_commit(&self) -> Result<Commit, JjError> {
        let workspace_name = self.workspace.workspace_name();
        let commit_id = self
            .repo
            .view()
            .get_wc_commit_id(workspace_name)
            .ok_or_else(|| JjError::MissingWorkingCopy {
                workspace: workspace_name.as_str().to_owned(),
            })?;

        self.load_commit(commit_id)
    }

    /// Resolves a user-supplied jj revision expression to exactly one commit.
    pub(super) fn resolve_single_revision(
        &self,
        revision: &str,
        diagnostics_source: &'static str,
    ) -> Result<Commit, JjError> {
        let revision = revision.trim();
        if revision.is_empty() {
            return Err(JjError::Revision {
                revision: revision.to_owned(),
                message: "revision expression is empty".to_owned(),
            });
        }

        let ui = Ui::null();
        let settings = self.workspace.settings();
        let fileset_aliases_map = load_fileset_aliases(&ui, settings.config())
            .map_err(|error| revision_command_error(revision, error))?;
        let revset_aliases_map = load_revset_aliases(&ui, settings.config())
            .map_err(|error| revision_command_error(revision, error))?;
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
        )
        .map_err(|error| revision_error(revision, error))?;
        let id_prefix_context =
            log_id_prefix_context(settings, &ui, &revset_context, revset_extensions.clone())
                .map_err(|error| revision_error(revision, error))?;

        let mut diagnostics = RevsetDiagnostics::new();
        let expression = revset::parse(&mut diagnostics, revision, &revset_context)
            .map_err(|error| revision_error(revision, error))?;
        print_parse_diagnostics(&ui, diagnostics_source, &diagnostics)
            .map_err(|error| revision_error(revision, error))?;
        let evaluator = RevsetExpressionEvaluator::new(
            self.repo.as_ref(),
            revset_extensions,
            &id_prefix_context,
            expression,
        );
        let mut commits = evaluator
            .evaluate_to_commits()
            .map_err(|error| revision_error(revision, error))?;
        let first = pollster::block_on(commits.try_next())
            .map_err(|error| revision_error(revision, error))?
            .ok_or_else(|| JjError::RevisionNotFound {
                revision: revision.to_owned(),
            })?;
        if pollster::block_on(commits.try_next())
            .map_err(|error| revision_error(revision, error))?
            .is_some()
        {
            return Err(JjError::AmbiguousRevision {
                revision: revision.to_owned(),
            });
        }

        Ok(first)
    }

    pub(super) fn resolve_trunk(&self, target: &Commit) -> Result<(String, Commit), JjError> {
        self.resolve_trunk_for_remote(target, ORIGIN_REMOTE_NAME)
    }

    pub(super) fn resolve_trunk_destination(&self) -> Result<(String, Commit), JjError> {
        let mut candidates = Vec::new();
        let mut conflicted = Vec::new();

        for (branch, remote_ref) in self
            .repo
            .view()
            .remote_bookmarks(RemoteName::new(ORIGIN_REMOTE_NAME))
        {
            let branch_name = branch.as_str().to_owned();
            let ref_target = &remote_ref.target;

            if ref_target.has_conflict() {
                conflicted.push(branch_name);
                continue;
            }

            if let Some(commit_id) = ref_target.as_normal() {
                candidates.push(TrunkCandidate {
                    branch: branch_name,
                    commit_id: commit_id.clone(),
                });
            }
        }

        let candidate = select_trunk_candidate(ORIGIN_REMOTE_NAME, candidates, conflicted)?;
        let trunk = self.load_commit(&candidate.commit_id)?;

        Ok((candidate.branch, trunk))
    }

    pub(super) fn resolve_trunk_for_remote(
        &self,
        target: &Commit,
        remote: &str,
    ) -> Result<(String, Commit), JjError> {
        let mut candidates = Vec::new();
        let mut conflicted = Vec::new();

        for (branch, remote_ref) in self.repo.view().remote_bookmarks(RemoteName::new(remote)) {
            let branch_name = branch.as_str().to_owned();
            let ref_target = &remote_ref.target;

            if ref_target.has_conflict() {
                conflicted.push(branch_name);
                continue;
            }

            let Some(commit_id) = ref_target.as_normal() else {
                continue;
            };

            if self.is_ancestor_or_equal(commit_id, target.id())? {
                candidates.push(TrunkCandidate {
                    branch: branch_name,
                    commit_id: commit_id.clone(),
                });
            }
        }

        let candidate = select_trunk_candidate(remote, candidates, conflicted)?;
        let trunk = self.load_commit(&candidate.commit_id)?;

        Ok((candidate.branch, trunk))
    }

    pub(super) fn linear_stack_path(
        &self,
        trunk: &Commit,
        target: &Commit,
    ) -> Result<Vec<Commit>, JjError> {
        if trunk.id() == target.id() {
            return Ok(Vec::new());
        }

        let mut reverse_path = Vec::new();
        let mut cursor = target.clone();
        let mut seen = HashSet::new();

        loop {
            if cursor.id() == trunk.id() {
                reverse_path.reverse();
                return Ok(reverse_path);
            }

            if !seen.insert(cursor.id().clone()) {
                return Err(JjError::NonLinearStack {
                    message: format!(
                        "cycle detected while walking from selected change {} to trunk {}",
                        short_commit_id(target.id()),
                        short_commit_id(trunk.id())
                    ),
                });
            }

            reverse_path.push(cursor.clone());
            let parents = cursor.parent_ids();
            if parents.len() != 1 {
                return Err(JjError::NonLinearStack {
                    message: format!(
                        "commit {} has {} parents; expected a linear path from trunk to selected change",
                        short_commit_id(cursor.id()),
                        parents.len()
                    ),
                });
            }

            cursor = self.load_commit(&parents[0])?;
        }
    }

    pub(super) fn nearest_ancestor_bookmark(
        &self,
        trunk: &Commit,
        stack_path: &[Commit],
    ) -> Option<String> {
        stack_path
            .iter()
            .rev()
            .skip(1)
            .chain(std::iter::once(trunk))
            .find_map(|commit| {
                self.local_bookmarks_for_commit(commit.id())
                    .into_iter()
                    .next()
            })
    }

    pub(super) fn change_summary(&self, commit: &Commit) -> Result<ChangeSummary, JjError> {
        let is_empty =
            pollster::block_on(commit.is_empty(self.repo.as_ref())).map_err(|error| {
                JjError::Backend {
                    message: error.to_string(),
                }
            })?;

        Ok(ChangeSummary {
            change_id: commit.change_id().hex(),
            commit_id: commit.id().hex(),
            short_commit_id: short_commit_id(commit.id()),
            description: commit.description().to_owned(),
            is_empty,
        })
    }

    pub(super) fn local_bookmarks(&self) -> Vec<String> {
        self.repo
            .view()
            .local_bookmarks()
            .map(|(name, _)| name.as_str().to_owned())
            .collect()
    }

    pub(super) fn local_bookmarks_for_commit(&self, commit_id: &CommitId) -> Vec<String> {
        self.repo
            .view()
            .local_bookmarks_for_commit(commit_id)
            .map(|(name, _)| name.as_str().to_owned())
            .collect()
    }

    pub(super) fn is_ancestor_or_equal(
        &self,
        ancestor: &CommitId,
        descendant: &CommitId,
    ) -> Result<bool, JjError> {
        self.repo
            .index()
            .is_ancestor(ancestor, descendant)
            .map_err(|error| JjError::Index {
                message: error.to_string(),
            })
    }

    pub(super) fn load_commit(&self, commit_id: &CommitId) -> Result<Commit, JjError> {
        self.repo
            .store()
            .get_commit(commit_id)
            .map_err(|error| JjError::Backend {
                message: error.to_string(),
            })
    }

    pub(super) fn ensure_git_backed(&self) -> Result<(), JjError> {
        let backend_name = self.repo.store().backend().name();
        if backend_name == GIT_BACKEND_NAME {
            Ok(())
        } else {
            Err(JjError::NotGitBacked {
                backend: backend_name.to_owned(),
            })
        }
    }
}
