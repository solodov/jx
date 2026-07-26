use super::*;
use std::time::{Duration, Instant};

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

/// Reads the live Git remote HEAD branch used only to recover from cached trunk ambiguity.
pub(super) fn live_remote_default_branch(workspace_root: &Path, remote: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("ls-remote")
        .arg("--symref")
        .arg(remote)
        .arg("HEAD")
        .current_dir(workspace_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_remote_default_branch(&stdout)
}

/// Extracts the HEAD branch from `git ls-remote --symref` output.
pub(super) fn parse_remote_default_branch(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.strip_prefix("ref: refs/heads/")?;
        let (branch, target) = line.split_once('\t')?;
        (target == "HEAD").then(|| branch.to_owned())
    })
}

/// Fetches the selected trunk, tracked origin bookmarks, and any explicit stale-candidate refreshes.
pub(super) fn fetch_origin_refs(
    mut_repo: &mut MutableRepo,
    origin_branch: &str,
    refresh_bookmarks: &[String],
    trace: &mut dyn FnMut(FetchTraceStep),
) -> Result<git::GitImportStats, JjError> {
    let git_settings = measure_git_fetch_step(
        trace,
        "load_git_settings",
        Vec::new(),
        || {
            GitSettings::from_settings(mut_repo.base_repo().settings()).map_err(|error| {
                JjError::Settings {
                    message: error.to_string(),
                }
            })
        },
        |_| Vec::new(),
    )?;
    let import_options = fetch_import_options(&git_settings);
    let tracked_bookmarks = measure_git_fetch_step(
        trace,
        "select_fetch_bookmarks",
        [
            fetch_trace_attr("branch", origin_branch),
            fetch_trace_attr("refresh_bookmark_count", refresh_bookmarks.len()),
        ],
        || {
            Ok(tracked_origin_bookmarks(
                mut_repo,
                origin_branch,
                refresh_bookmarks,
            ))
        },
        |result: &Result<Vec<String>, JjError>| match result {
            Ok(bookmarks) => vec![
                fetch_trace_attr("bookmark_count", bookmarks.len()),
                fetch_trace_attr("bookmarks", joined_git_fetch_values(bookmarks)),
            ],
            Err(_) => Vec::new(),
        },
    )?;
    let bookmark_expression = StringExpression::union_all(
        tracked_bookmarks
            .iter()
            .cloned()
            .map(StringExpression::exact)
            .collect(),
    );
    let mut fetcher = measure_git_fetch_step(
        trace,
        "create_git_fetcher",
        Vec::new(),
        || {
            GitFetch::new(
                mut_repo,
                git_settings.to_subprocess_options(),
                &import_options,
            )
            .map_err(|error| JjError::Fetch {
                message: error.to_string(),
            })
        },
        |_| Vec::new(),
    )?;
    let ref_expression = GitFetchRefExpression {
        bookmark: bookmark_expression,
        tag: StringExpression::none(),
    };
    let refspecs = measure_git_fetch_step(
        trace,
        "expand_fetch_refspecs",
        [fetch_trace_attr("bookmark_count", tracked_bookmarks.len())],
        || {
            git::expand_fetch_refspecs(RemoteName::new(ORIGIN_REMOTE_NAME), ref_expression).map_err(
                |error| JjError::Fetch {
                    message: error.to_string(),
                },
            )
        },
        |result| match result {
            Ok(_) => vec![fetch_trace_attr("refspec_count", tracked_bookmarks.len())],
            Err(_) => Vec::new(),
        },
    )?;
    let mut callback = SilentGitCallback;

    measure_git_fetch_step(
        trace,
        "git_fetch",
        [
            fetch_trace_attr("remote", ORIGIN_REMOTE_NAME),
            fetch_trace_attr("refspec_count", tracked_bookmarks.len()),
            fetch_trace_attr("fetch_tags", "no_tags"),
            fetch_trace_attr("bookmark_count", tracked_bookmarks.len()),
        ],
        || {
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
                })
        },
        |_| Vec::new(),
    )?;

    measure_git_fetch_step(
        trace,
        "import_refs",
        [
            fetch_trace_attr("remote", ORIGIN_REMOTE_NAME),
            fetch_trace_attr("bookmark_count", tracked_bookmarks.len()),
        ],
        || {
            pollster::block_on(fetcher.import_refs()).map_err(|error| JjError::Import {
                message: error.to_string(),
            })
        },
        import_refs_result_attrs,
    )
}

fn measure_git_fetch_step<T>(
    trace: &mut dyn FnMut(FetchTraceStep),
    name: impl Into<String>,
    attrs: impl IntoIterator<Item = FetchTraceAttr>,
    operation: impl FnOnce() -> Result<T, JjError>,
    result_attrs: impl FnOnce(&Result<T, JjError>) -> Vec<FetchTraceAttr>,
) -> Result<T, JjError> {
    let name = name.into();
    let started = Instant::now();
    let result = operation();
    let mut attrs = attrs.into_iter().collect::<Vec<_>>();
    attrs.extend(result_attrs(&result));
    trace(FetchTraceStep {
        name,
        duration_us: git_fetch_duration_us(started.elapsed()),
        attrs,
        error: result.as_ref().err().map(ToString::to_string),
    });
    result
}

fn import_refs_result_attrs(result: &Result<git::GitImportStats, JjError>) -> Vec<FetchTraceAttr> {
    match result {
        Ok(stats) => vec![
            fetch_trace_attr(
                "changed_remote_bookmarks",
                stats.changed_remote_bookmarks.len(),
            ),
            fetch_trace_attr("changed_remote_tags", stats.changed_remote_tags.len()),
            fetch_trace_attr("abandoned_commits", stats.abandoned_commits.len()),
            fetch_trace_attr("rewritten_commits", stats.rewritten_commit_ids.len()),
        ],
        Err(error) => git_ref_error_attrs(error),
    }
}

fn git_ref_error_attrs(error: &JjError) -> Vec<FetchTraceAttr> {
    let message = error.to_string();
    let Some(path) = quoted_git_ref_path(&message) else {
        return Vec::new();
    };

    let mut attrs = vec![fetch_trace_attr("git_ref_path", path.display().to_string())];
    let metadata = fs::metadata(&path);
    attrs.push(fetch_trace_attr("git_ref_exists", metadata.is_ok()));
    if let Ok(metadata) = metadata {
        attrs.push(fetch_trace_attr("git_ref_size", metadata.len()));
    }

    let lock_path = PathBuf::from(format!("{}.lock", path.display()));
    let lock_metadata = fs::metadata(&lock_path);
    attrs.push(fetch_trace_attr(
        "git_ref_lock_exists",
        lock_metadata.is_ok(),
    ));
    if let Ok(metadata) = lock_metadata {
        attrs.push(fetch_trace_attr("git_ref_lock_size", metadata.len()));
    }

    attrs
}

fn quoted_git_ref_path(message: &str) -> Option<PathBuf> {
    let (_, after_prefix) = message.split_once("The ref file \"")?;
    let (path, _) = after_prefix.split_once('"')?;
    Some(PathBuf::from(path))
}

fn joined_git_fetch_values(values: &[String]) -> String {
    const MAX_VALUES: usize = 20;
    if values.len() <= MAX_VALUES {
        return values.join(",");
    }

    format!(
        "{},…(+{})",
        values[..MAX_VALUES].join(","),
        values.len() - MAX_VALUES
    )
}

fn git_fetch_duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Returns the narrow bookmark set fetch should refresh for sync/fetch correctness.
pub(super) fn tracked_origin_bookmarks(
    mut_repo: &MutableRepo,
    origin_branch: &str,
    refresh_bookmarks: &[String],
) -> Vec<String> {
    let mut bookmarks = BTreeSet::from([origin_branch.to_owned()]);
    bookmarks.extend(refresh_bookmarks.iter().cloned());
    bookmarks.extend(
        mut_repo
            .view()
            .local_remote_bookmarks(RemoteName::new(ORIGIN_REMOTE_NAME))
            .filter(|(_, targets)| targets.remote_ref.is_tracked())
            .map(|(name, _)| name.as_str().to_owned()),
    );
    bookmarks.into_iter().collect()
}

pub(super) fn fetch_import_options(git_settings: &GitSettings) -> GitImportOptions {
    GitImportOptions {
        abandon_unreachable_commits: git_settings.abandon_unreachable_commits,
        record_synthetic_predecessors: git_settings.record_synthetic_predecessors,
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
            if parent_id == self.repo.store().root_commit().id() {
                current
            } else {
                self.load_commit(parent_id)?
            }
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
        let _planned_target_id = initial_publish_commit_id(target)?;
        self.snapshot_working_copy_for_initial_publish()?;
        let target = self.initial_publish_target()?;
        let target_id = initial_publish_commit_id(&target)?;
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

    fn snapshot_working_copy_for_initial_publish(&mut self) -> Result<(), JjError> {
        self.reload_after_working_copy_snapshot()
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
