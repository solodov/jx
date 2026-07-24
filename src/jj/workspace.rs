use super::*;
use std::time::{Duration, Instant};

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
        let mut commits = self.resolve_revisions(revision, diagnostics_source)?;
        let first = commits.pop().ok_or_else(|| JjError::RevisionNotFound {
            revision: revision.trim().to_owned(),
        })?;
        if !commits.is_empty() {
            return Err(JjError::AmbiguousRevision {
                revision: revision.trim().to_owned(),
            });
        }

        Ok(first)
    }

    /// Resolves a user-supplied jj revset to zero or more commits.
    pub(super) fn resolve_revisions(
        &self,
        revision: &str,
        diagnostics_source: &'static str,
    ) -> Result<Vec<Commit>, JjError> {
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
        let commits = pollster::block_on(
            evaluator
                .evaluate_to_commits()
                .map_err(|error| revision_error(revision, error))?
                .try_collect::<Vec<_>>(),
        )
        .map_err(|error| revision_error(revision, error))?;
        Ok(commits)
    }

    pub(super) fn resolve_trunk(&self, target: &Commit) -> Result<(String, Commit), JjError> {
        self.resolve_trunk_for_remote(target, ORIGIN_REMOTE_NAME)
    }

    pub(super) fn resolve_trunk_destination(&self) -> Result<(String, Commit), JjError> {
        self.resolve_unanchored_origin_trunk()
    }

    pub(super) fn resolve_unanchored_origin_trunk(&self) -> Result<(String, Commit), JjError> {
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

        let candidate =
            select_trunk_candidate_with_hint(ORIGIN_REMOTE_NAME, candidates, conflicted, None)?;
        let trunk = self.load_commit(&candidate.commit_id)?;

        Ok((candidate.branch, trunk))
    }

    pub(super) fn stack_path_for_target(
        &self,
        target: &Commit,
        policy: StackBasePolicy,
    ) -> Result<(String, Commit, Vec<Commit>), JjError> {
        match self.resolve_unanchored_origin_trunk() {
            Ok((branch, trunk_head)) => {
                let (stack_base, path) =
                    self.stack_path_from_trunk_head(&branch, &trunk_head, target, policy)?;
                Ok((branch, stack_base, path))
            }
            Err(error) if policy.allows_historical_trunk_base() => Err(error),
            Err(_) => {
                let (branch, trunk) = self.resolve_trunk(target)?;
                let path = self.linear_stack_path(&trunk, target)?;
                Ok((branch, trunk, path))
            }
        }
    }

    pub(super) fn stack_path_from_trunk_head(
        &self,
        trunk_branch: &str,
        trunk_head: &Commit,
        target: &Commit,
        policy: StackBasePolicy,
    ) -> Result<(Commit, Vec<Commit>), JjError> {
        match self.linear_stack_path(trunk_head, target) {
            Ok(path) => Ok((trunk_head.clone(), path)),
            Err(error) if !policy.allows_historical_trunk_base() => Err(error),
            Err(_) => self.historical_stack_path_from_trunk_head(trunk_branch, trunk_head, target),
        }
    }

    fn historical_stack_path_from_trunk_head(
        &self,
        trunk_branch: &str,
        trunk_head: &Commit,
        target: &Commit,
    ) -> Result<(Commit, Vec<Commit>), JjError> {
        let mut reverse_path = Vec::new();
        let mut cursor = target.clone();
        let mut seen = HashSet::new();

        loop {
            if self.is_ancestor_or_equal(cursor.id(), trunk_head.id())?
                && self.can_use_historical_stack_base(cursor.id(), trunk_branch)
            {
                reverse_path.reverse();
                return Ok((cursor, reverse_path));
            }

            if !seen.insert(cursor.id().clone()) {
                return Err(JjError::NonLinearStack {
                    message: format!(
                        "cycle detected while walking from selected change {} to trunk {}",
                        short_commit_id(target.id()),
                        short_commit_id(trunk_head.id())
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

    fn can_use_historical_stack_base(&self, commit_id: &CommitId, trunk_branch: &str) -> bool {
        !self
            .repo
            .view()
            .local_bookmarks_for_commit(commit_id)
            .any(|(branch, _)| branch.as_str() != trunk_branch)
            && !self
                .repo
                .view()
                .remote_bookmarks(RemoteName::new(ORIGIN_REMOTE_NAME))
                .any(|(branch, remote_ref)| {
                    branch.as_str() != trunk_branch
                        && remote_ref.target.as_normal() == Some(commit_id)
                })
    }

    pub(super) fn resolve_trunk_for_remote(
        &self,
        target: &Commit,
        remote: &str,
    ) -> Result<(String, Commit), JjError> {
        self.resolve_trunk_for_remote_with_hint(target, remote, None)
    }

    /// Resolves the trunk ancestor for a remote, optionally trusting a live branch hint to break ties.
    pub(super) fn resolve_trunk_for_remote_with_hint(
        &self,
        target: &Commit,
        remote: &str,
        branch_hint: Option<&str>,
    ) -> Result<(String, Commit), JjError> {
        self.resolve_trunk_for_remote_with_hint_and_metrics(target, remote, branch_hint)
            .map(|(branch, trunk, _)| (branch, trunk))
    }

    /// Resolves a remote trunk and reports where local candidate selection spent time.
    pub(super) fn resolve_trunk_for_remote_with_hint_and_metrics(
        &self,
        target: &Commit,
        remote: &str,
        branch_hint: Option<&str>,
    ) -> Result<(String, Commit, TrunkResolveMetrics), JjError> {
        if let Some((candidate, mut metrics)) =
            self.preferred_trunk_candidate(target, remote, branch_hint)?
        {
            metrics.fast_path = true;
            let load_started = Instant::now();
            let trunk = self.load_commit(&candidate.commit_id)?;
            metrics.load_trunk_commit_us = duration_us(load_started.elapsed());
            return Ok((candidate.branch, trunk, metrics));
        }

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

            let Some(commit_id) = ref_target.as_normal() else {
                continue;
            };
            metrics.normal_bookmark_count += 1;
            metrics.ancestor_check_count += 1;

            if self.is_ancestor_or_equal(commit_id, target.id())? {
                candidates.push(TrunkCandidate {
                    branch: branch_name,
                    commit_id: commit_id.clone(),
                });
            }
        }
        metrics.scan_remote_bookmarks_us = duration_us(scan_started.elapsed());
        metrics.candidate_count = candidates.len();

        let select_started = Instant::now();
        let candidate =
            select_trunk_candidate_with_hint(remote, candidates, conflicted, branch_hint)?;
        metrics.select_candidate_us = duration_us(select_started.elapsed());

        let load_started = Instant::now();
        let trunk = self.load_commit(&candidate.commit_id)?;
        metrics.load_trunk_commit_us = duration_us(load_started.elapsed());

        Ok((candidate.branch, trunk, metrics))
    }

    fn preferred_trunk_candidate(
        &self,
        target: &Commit,
        remote: &str,
        branch_hint: Option<&str>,
    ) -> Result<Option<(TrunkCandidate, TrunkResolveMetrics)>, JjError> {
        if let Some(branch_hint) = branch_hint {
            let mut metrics = TrunkResolveMetrics::default();
            let scan_started = Instant::now();
            let candidate =
                self.trunk_candidate_for_branch(target, remote, branch_hint, &mut metrics)?;
            metrics.scan_remote_bookmarks_us = duration_us(scan_started.elapsed());
            if let Some(candidate) = candidate {
                metrics.candidate_count = 1;
                return Ok(Some((candidate, metrics)));
            }
        }

        let mut metrics = TrunkResolveMetrics::default();
        let scan_started = Instant::now();
        let mut candidates = Vec::new();
        let mut checked = Vec::<&str>::new();
        for branch in PREFERRED_TRUNK_BRANCHES {
            if Some(branch) == branch_hint || checked.contains(&branch) {
                continue;
            }
            checked.push(branch);
            if let Some(candidate) =
                self.trunk_candidate_for_branch(target, remote, branch, &mut metrics)?
            {
                candidates.push(candidate);
            }
        }
        metrics.scan_remote_bookmarks_us = duration_us(scan_started.elapsed());
        metrics.candidate_count = candidates.len();

        match candidates.as_slice() {
            [candidate] => Ok(Some((candidate.clone(), metrics))),
            _ => Ok(None),
        }
    }

    fn trunk_candidate_for_branch(
        &self,
        target: &Commit,
        remote: &str,
        branch: &str,
        metrics: &mut TrunkResolveMetrics,
    ) -> Result<Option<TrunkCandidate>, JjError> {
        metrics.preferred_branch_check_count += 1;
        let remote_ref = self
            .repo
            .view()
            .get_remote_bookmark(RefName::new(branch).to_remote_symbol(RemoteName::new(remote)));
        let ref_target = &remote_ref.target;

        if ref_target.has_conflict() {
            metrics.remote_bookmark_count += 1;
            metrics.conflicted_bookmark_count += 1;
            return Ok(None);
        }

        let Some(commit_id) = ref_target.as_normal() else {
            return Ok(None);
        };
        metrics.remote_bookmark_count += 1;
        metrics.normal_bookmark_count += 1;
        metrics.ancestor_check_count += 1;

        Ok(self
            .is_ancestor_or_equal(commit_id, target.id())?
            .then(|| TrunkCandidate {
                branch: branch.to_owned(),
                commit_id: commit_id.clone(),
            }))
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

    /// Returns the distance when `descendant` is on a single-parent path from `ancestor`.
    pub(super) fn linear_descendant_distance(
        &self,
        ancestor: &Commit,
        descendant: &Commit,
    ) -> Result<Option<usize>, JjError> {
        if !self.is_ancestor_or_equal(ancestor.id(), descendant.id())? {
            return Ok(None);
        }

        let mut cursor = descendant.clone();
        let mut seen = HashSet::new();
        let mut distance = 0;

        loop {
            if cursor.id() == ancestor.id() {
                return Ok(Some(distance));
            }
            if !seen.insert(cursor.id().clone()) {
                return Ok(None);
            }

            let parents = cursor.parent_ids();
            if parents.len() != 1 {
                return Ok(None);
            }

            distance += 1;
            cursor = self.load_commit(&parents[0])?;
        }
    }

    pub(super) fn nearest_ancestor_bookmark(
        &self,
        _trunk: &Commit,
        stack_path: &[Commit],
    ) -> Option<String> {
        // Stacked PRs target bookmarked commits within the stack, but root PRs
        // must target the resolved trunk branch even when trunk has extra labels.
        stack_path.iter().rev().skip(1).find_map(|commit| {
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
            change_id: commit.change_id().reverse_hex(),
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

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().try_into().unwrap_or(u64::MAX)
}
