use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use jj_lib::{
    backend::{CopyId, TreeValue},
    commit::Commit,
    merge::Merge,
    merged_tree_builder::MergedTreeBuilder,
    op_store::{RefTarget, RemoteRef, RemoteRefState},
    ref_name::{RefName, RemoteName},
    repo::{MutableRepo, Repo as _},
    repo_path::RepoPathBuf,
};

use super::*;

mod bookmarks;
mod description;
mod diff;
mod facts;
mod fetch;
mod git_transport;
mod log;
mod navigation;
mod pull_request;
mod push;
mod stack;
mod status;
mod workspace;

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

fn log_test_settings() -> Result<UserSettings, JjError> {
    let mut config = StackedConfig::with_defaults();
    config.extend_layers(default_config_layers());
    jj_lib::config::migrate(&mut config, &default_config_migrations()).map_err(log_error)?;
    UserSettings::from_config(config).map_err(|error| JjError::Settings {
        message: error.to_string(),
    })
}

async fn write_child(repo: &mut MutableRepo, parent: &Commit, description: &str) -> Commit {
    repo.new_commit(vec![parent.id().clone()], parent.tree())
        .set_description(description)
        .write()
        .await
        .expect("write child commit")
}

async fn write_child_with_files(
    repo: &mut MutableRepo,
    parent: &Commit,
    description: &str,
    files: &[(&str, &[u8])],
) -> Commit {
    let mut tree_builder = MergedTreeBuilder::new(parent.tree());
    for (path, contents) in files {
        let path = RepoPathBuf::from_internal_string(*path).expect("valid repo path");
        let id = repo
            .store()
            .write_file(&path, &mut &contents[..])
            .await
            .expect("write file contents");
        tree_builder.set_or_remove(
            path,
            Merge::normal(TreeValue::File {
                id,
                executable: false,
                copy_id: CopyId::placeholder(),
            }),
        );
    }
    let tree = tree_builder.write_tree().await.expect("write tree");

    repo.new_commit(vec![parent.id().clone()], tree)
        .set_description(description)
        .write()
        .await
        .expect("write child commit with files")
}

fn set_origin_bookmark(repo: &mut MutableRepo, branch: &str, commit_id: &CommitId) {
    set_remote_bookmark(repo, ORIGIN_REMOTE_NAME, branch, commit_id);
}

fn set_remote_bookmark(repo: &mut MutableRepo, remote: &str, branch: &str, commit_id: &CommitId) {
    set_remote_bookmark_with_state(repo, remote, branch, commit_id, RemoteRefState::Tracked);
}

fn set_untracked_origin_bookmark(repo: &mut MutableRepo, branch: &str, commit_id: &CommitId) {
    set_remote_bookmark_with_state(
        repo,
        ORIGIN_REMOTE_NAME,
        branch,
        commit_id,
        RemoteRefState::New,
    );
}

fn set_remote_bookmark_with_state(
    repo: &mut MutableRepo,
    remote: &str,
    branch: &str,
    commit_id: &CommitId,
    state: RemoteRefState,
) {
    repo.set_remote_bookmark(
        RefName::new(branch).to_remote_symbol(RemoteName::new(remote)),
        RemoteRef {
            target: RefTarget::normal(commit_id.clone()),
            state,
        },
    );
}

fn set_local_bookmark(repo: &mut MutableRepo, bookmark: &str, commit_id: &CommitId) {
    repo.set_local_bookmark_target(RefName::new(bookmark), RefTarget::normal(commit_id.clone()));
}

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "jx-jj-test-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create test workspace");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
