use super::*;

pub(super) fn run_jj_git_push_bookmark(workspace_root: &Path, branch: &str) -> Result<(), JjError> {
    let status = Command::new("jj")
        .arg("--repository")
        .arg(workspace_root)
        .arg("--no-pager")
        .arg("git")
        .arg("push")
        .arg("-b")
        .arg(branch)
        .current_dir(workspace_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| JjError::BootstrapPushStart {
            branch: branch.to_owned(),
            source,
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(JjError::BootstrapPushFailed {
            branch: branch.to_owned(),
            status: exit_status_summary(status),
        })
    }
}

pub(super) fn exit_status_summary(status: std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| status.to_string())
}

pub(super) fn collect_child_ids(
    repo: &dyn jj_lib::repo::Repo,
    commit_id: &CommitId,
) -> Result<Vec<CommitId>, JjError> {
    let revset = ResolvedRevsetExpression::commit(commit_id.clone())
        .children()
        .evaluate(repo)
        .map_err(|error| JjError::Backend {
            message: error.into_backend_error().to_string(),
        })?;

    pollster::block_on(revset.stream().try_collect::<Vec<_>>()).map_err(|error| JjError::Backend {
        message: error.into_backend_error().to_string(),
    })
}

pub(super) fn fetch_origin_refs(
    mut_repo: &mut MutableRepo,
    origin_branch: &str,
) -> Result<git::GitImportStats, JjError> {
    let git_settings =
        GitSettings::from_settings(mut_repo.base_repo().settings()).map_err(|error| {
            JjError::Settings {
                message: error.to_string(),
            }
        })?;
    let import_options = fetch_import_options();
    let bookmark_expression = StringExpression::union_all(
        tracked_origin_bookmarks(mut_repo, origin_branch)
            .into_iter()
            .map(StringExpression::exact)
            .collect(),
    );
    let mut fetcher = GitFetch::new(
        mut_repo,
        git_settings.to_subprocess_options(),
        &import_options,
    )
    .map_err(|error| JjError::Fetch {
        message: error.to_string(),
    })?;
    let ref_expression = GitFetchRefExpression {
        bookmark: bookmark_expression,
        tag: StringExpression::none(),
    };
    let refspecs = git::expand_fetch_refspecs(RemoteName::new(ORIGIN_REMOTE_NAME), ref_expression)
        .map_err(|error| JjError::Fetch {
            message: error.to_string(),
        })?;
    let mut callback = SilentGitCallback;

    fetcher
        .fetch(
            RemoteName::new(ORIGIN_REMOTE_NAME),
            refspecs,
            &mut callback,
            None,
            Some(FetchTagsOverride::NoTags),
        )
        .map_err(|error| JjError::Fetch {
            message: error.to_string(),
        })?;

    pollster::block_on(fetcher.import_refs()).map_err(|error| JjError::Import {
        message: error.to_string(),
    })
}

pub(super) fn tracked_origin_bookmarks(mut_repo: &MutableRepo, origin_branch: &str) -> Vec<String> {
    let mut bookmarks = BTreeSet::from([origin_branch.to_owned()]);
    bookmarks.extend(
        mut_repo
            .view()
            .local_remote_bookmarks(RemoteName::new(ORIGIN_REMOTE_NAME))
            .filter(|(_, targets)| targets.remote_ref.is_tracked())
            .map(|(name, _)| name.as_str().to_owned()),
    );
    bookmarks.into_iter().collect()
}

pub(super) fn fetch_import_options() -> GitImportOptions {
    GitImportOptions {
        auto_local_bookmark: false,
        // Fetch repair logic owns post-fetch rebases; importing should not let
        // jj's generic Git-abandon pass rewrite immutable trunk children first.
        abandon_unreachable_commits: false,
        remote_auto_track_bookmarks: HashMap::new(),
    }
}

/// Exports jj bookmark changes back to the backing Git refs before committing an operation.
pub(super) fn export_git_refs(mut_repo: &mut MutableRepo) -> Result<(), JjError> {
    let stats = git::export_refs(mut_repo).map_err(|error| JjError::Export {
        message: error.to_string(),
    })?;

    if stats.failed_bookmarks.is_empty() && stats.failed_tags.is_empty() {
        return Ok(());
    }

    Err(JjError::Export {
        message: export_failure_message(&stats),
    })
}

pub(super) fn export_failure_message(stats: &git::GitExportStats) -> String {
    format!(
        "{} bookmark(s) and {} tag(s) could not be exported",
        stats.failed_bookmarks.len(),
        stats.failed_tags.len()
    )
}

pub(super) fn load_origin_branch(
    repo: &dyn jj_lib::repo::Repo,
    branch: &str,
) -> Result<Commit, JjError> {
    let symbol = RefName::new(branch).to_remote_symbol(RemoteName::new(ORIGIN_REMOTE_NAME));
    let target = &repo.view().get_remote_bookmark(symbol).target;

    if target.has_conflict() {
        return Err(JjError::ConflictedTrunk {
            remote: ORIGIN_REMOTE_NAME.to_owned(),
            branches: vec![branch.to_owned()],
        });
    }

    let Some(commit_id) = target.as_normal() else {
        return Err(JjError::MissingTrunk {
            remote: ORIGIN_REMOTE_NAME.to_owned(),
        });
    };

    load_commit_from_repo(repo, commit_id)
}

pub(super) fn load_commit_from_repo(
    repo: &dyn jj_lib::repo::Repo,
    commit_id: &CommitId,
) -> Result<Commit, JjError> {
    repo.store()
        .get_commit(commit_id)
        .map_err(|error| JjError::Backend {
            message: error.to_string(),
        })
}

pub(super) fn is_ancestor_or_equal_in_repo(
    repo: &dyn jj_lib::repo::Repo,
    ancestor: &CommitId,
    descendant: &CommitId,
) -> Result<bool, JjError> {
    repo.index()
        .is_ancestor(ancestor, descendant)
        .map_err(|error| JjError::Index {
            message: error.to_string(),
        })
}

pub(super) struct SilentGitCallback;

impl GitSubprocessCallback for SilentGitCallback {
    fn needs_progress(&self) -> bool {
        false
    }

    fn progress(&mut self, _progress: &GitProgress) -> io::Result<()> {
        Ok(())
    }

    fn local_sideband(
        &mut self,
        _message: &[u8],
        _term: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()> {
        Ok(())
    }

    fn remote_sideband(
        &mut self,
        _message: &[u8],
        _term: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()> {
        Ok(())
    }
}

const DEFAULT_INITIAL_COMMIT_DESCRIPTION: &str = "initial commit";

impl JjWorkspace {
    /// Returns the commit that should seed a newly created remote repository.
    pub fn initial_publish_target(&self) -> Result<InitialPublishTarget, JjError> {
        self.ensure_git_backed()?;
        let current = self.current_commit()?;
        let current_is_empty =
            pollster::block_on(current.is_empty(self.repo.as_ref())).map_err(|error| {
                JjError::Backend {
                    message: error.to_string(),
                }
            })?;
        let target = if current_is_empty {
            let parents = current.parent_ids();
            let Some(parent_id) = parents.first() else {
                return Err(JjError::NoPublishableCommit {
                    message: "empty working-copy commit has no parent".to_owned(),
                });
            };
            if parents.len() != 1 {
                return Err(JjError::NoPublishableCommit {
                    message: "empty working-copy commit has multiple parents".to_owned(),
                });
            }
            self.load_commit(parent_id)?
        } else {
            current
        };

        if target.id() == self.repo.store().root_commit().id() {
            return Err(JjError::NoPublishableCommit {
                message: "the repository has no non-root commit to push".to_owned(),
            });
        }

        Ok(initial_publish_target_summary(&target))
    }

    /// Gives a fresh root-child commit a description so Git export can publish it.
    pub fn prepare_initial_publish_target(
        &mut self,
        target: &InitialPublishTarget,
    ) -> Result<InitialPublishTarget, JjError> {
        self.ensure_git_backed()?;
        let target_id = initial_publish_commit_id(target)?;
        let target_commit = self.load_commit(&target_id)?;

        if !self.should_describe_initial_publish_target(&target_commit) {
            return Ok(initial_publish_target_summary(&target_commit));
        }

        let current_before = self.current_commit()?;
        let current_before_tree = current_before.tree();
        let workspace_name = self.workspace.workspace_name().to_owned();

        let mut tx = self.repo.start_transaction();
        let rewritten = pollster::block_on(
            tx.repo_mut()
                .rewrite_commit(&target_commit)
                .set_description(DEFAULT_INITIAL_COMMIT_DESCRIPTION)
                .write(),
        )
        .map_err(|error| JjError::Backend {
            message: error.to_string(),
        })?;
        pollster::block_on(tx.repo_mut().rebase_descendants()).map_err(|error| {
            JjError::Backend {
                message: error.to_string(),
            }
        })?;
        export_git_refs(tx.repo_mut())?;
        let final_current_id = tx
            .repo()
            .view()
            .get_wc_commit_id(&workspace_name)
            .cloned()
            .ok_or_else(|| JjError::MissingWorkingCopy {
                workspace: workspace_name.as_str().to_owned(),
            })?;
        let repo =
            pollster::block_on(tx.commit("jx describe initial commit")).map_err(|error| {
                JjError::Transaction {
                    message: error.to_string(),
                }
            })?;

        if final_current_id != *current_before.id() {
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

        Ok(initial_publish_target_summary(&rewritten))
    }

    fn should_describe_initial_publish_target(&self, commit: &Commit) -> bool {
        let root_id = self.repo.store().root_commit().id().clone();

        commit.description().trim().is_empty()
            && matches!(commit.parent_ids(), [parent] if parent == &root_id)
    }

    /// Adds `origin`, points `main` at `target`, then pushes the initial branch through jj.
    pub fn bootstrap_origin_main(
        &mut self,
        remote_url: &str,
        target: &InitialPublishTarget,
    ) -> Result<BootstrapPushOutcome, JjError> {
        self.ensure_git_backed()?;
        let target_id = initial_publish_commit_id(target)?;
        let target_commit = self.load_commit(&target_id)?;
        let branch = "main";
        let current_before = self.current_commit()?;
        let current_before_tree = current_before.tree();
        let workspace_name = self.workspace.workspace_name().to_owned();
        let should_create_empty_child = current_before.id() == target_commit.id();

        let mut tx = self.repo.start_transaction();
        git::add_remote(
            tx.repo_mut(),
            RemoteName::new(ORIGIN_REMOTE_NAME),
            remote_url,
            None,
            gix::remote::fetch::Tags::None,
        )
        .map_err(|error| JjError::RemoteAdd {
            remote: ORIGIN_REMOTE_NAME.to_owned(),
            message: error.to_string(),
        })?;
        tx.repo_mut().set_local_bookmark_target(
            RefName::new(branch),
            RefTarget::normal(target_commit.id().clone()),
        );
        if should_create_empty_child {
            pollster::block_on(
                tx.repo_mut()
                    .check_out(workspace_name.clone(), &target_commit),
            )
            .map_err(|error| JjError::WorkingCopyCheckout {
                message: error.to_string(),
            })?;
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
        let repo = pollster::block_on(tx.commit("jx bootstrap origin main")).map_err(|error| {
            JjError::Transaction {
                message: error.to_string(),
            }
        })?;

        if should_create_empty_child {
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

        run_jj_git_push_bookmark(self.workspace.workspace_root(), branch)?;

        Ok(BootstrapPushOutcome {
            branch: branch.to_owned(),
            short_commit_id: target.short_commit_id.clone(),
            description: target.description.clone(),
            working_copy_short_commit_id: (final_current_id != target_id)
                .then(|| short_commit_id(&final_current_id)),
        })
    }

    /// Returns Git remotes from the Git backend associated with this jj workspace.
    pub fn git_remotes(&self) -> Result<Vec<GitRemote>, JjError> {
        self.ensure_git_backed()?;
        let git_repo = git::get_git_repo(self.repo.store()).map_err(|error| JjError::Backend {
            message: error.to_string(),
        })?;
        let names =
            git::get_all_remote_names(self.repo.store()).map_err(|error| JjError::Backend {
                message: error.to_string(),
            })?;
        let mut remotes = Vec::new();

        for name in names {
            let Some(remote) = git_repo.try_find_remote(name.as_str()) else {
                continue;
            };
            let remote = remote.map_err(|error| JjError::Backend {
                message: error.to_string(),
            })?;
            let Some(url) = remote.url(gix::remote::Direction::Fetch) else {
                continue;
            };

            remotes.push(GitRemote {
                name: name.as_str().to_owned(),
                url: url.to_string(),
            });
        }

        Ok(remotes)
    }
}

fn initial_publish_commit_id(target: &InitialPublishTarget) -> Result<CommitId, JjError> {
    CommitId::try_from_hex(&target.commit_id).ok_or_else(|| JjError::InvalidTargetCommitId {
        commit_id: target.commit_id.clone(),
    })
}

fn initial_publish_target_summary(commit: &Commit) -> InitialPublishTarget {
    InitialPublishTarget {
        commit_id: commit.id().hex(),
        short_commit_id: short_commit_id(commit.id()),
        description: commit.description().to_owned(),
    }
}
