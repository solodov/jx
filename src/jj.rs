//! jj-only workspace boundary.
//!
//! `JjWorkspace` is the single local-VCS boundary for workflow services. It
//! loads a Jujutsu workspace through `jj-lib`, exposes domain facts about the
//! selected jj change, resolved trunk, remote-backed status state, local
//! bookmarks, and stack position. Fetch, bookmark, rebase, push transport, and
//! working-copy mutations stay behind this jj-native boundary so command/domain
//! services only coordinate high-level workflow intent. The diff passthrough is
//! the intentional exception: it shells out to `jj diff` so user diff renderer
//! configuration remains authoritative.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use chrono::{Local, TimeZone as _};
use futures::{StreamExt as _, TryStreamExt as _};
use jj_cli::{
    cli_util::{
        default_ignored_remote_name, format_template, load_fileset_aliases, load_revset_aliases,
        load_template_aliases, LogContentFormat,
    },
    command_error::print_parse_diagnostics,
    commit_templater::{CommitTemplateLanguage, CommitTemplateLanguageExtension},
    config::{config_from_environment, default_config_layers, default_config_migrations},
    formatter::{Formatter, FormatterExt as _},
    graphlog::{get_graphlog, GraphStyle},
    revset_util::{parse_immutable_heads_expression, RevsetExpressionEvaluator},
    template_builder,
    template_parser::TemplateDiagnostics,
    templater::TemplateRenderer,
    ui::Ui,
};
use jj_lib::{
    backend::{BackendError, CommitId},
    commit::Commit,
    config::{ConfigGetResultExt as _, ConfigLayer, ConfigSource, StackedConfig},
    git::{
        self, FetchTagsOverride, GitFetch, GitFetchRefExpression, GitImportOptions, GitProgress,
        GitPushOptions, GitPushRefTargets, GitSettings, GitSidebandLineTerminator,
        GitSubprocessCallback,
    },
    graph::{GraphEdge, GraphEdgeType, TopoGroupedGraph},
    id_prefix::IdPrefixContext,
    matchers::EverythingMatcher,
    merge::Diff,
    object_id::ObjectId,
    op_store::RefTarget,
    ref_name::{RefName, RefNameBuf, RemoteName, WorkspaceName, WorkspaceNameBuf},
    refs::{classify_ref_push_action, LocalAndRemoteRef, RefPushAction},
    repo::{MutableRepo, ReadonlyRepo, Repo as _, StoreFactories},
    repo_path::{RepoPath, RepoPathUiConverter},
    revset::{
        self, ResolvedRevsetExpression, RevsetDiagnostics, RevsetExpression, RevsetExtensions,
        RevsetParseContext, RevsetWorkspaceContext,
    },
    rewrite::{
        compute_move_commits, rebase_commit_with_options, CommitRewriter, EmptyBehavior,
        MoveCommitsLocation, MoveCommitsTarget, RebaseOptions, RebasedCommit, RewriteRefsOptions,
    },
    settings::UserSettings,
    str_util::StringExpression,
    workspace::{
        default_working_copy_factories, DefaultWorkspaceLoaderFactory, Workspace, WorkspaceLoader,
        WorkspaceLoaderFactory as _,
    },
};
use thiserror::Error;

use crate::repository::ORIGIN_REMOTE_NAME;

mod bookmarks;
mod description;
mod diff;
mod error;
mod facts;
mod fetch;
mod git_transport;
mod log;
mod navigation;
mod push;
mod rebase;
mod status;
mod types;
mod workspace;
mod workspace_management;

#[cfg(test)]
use diff::{diff_paths_without_tests, external_diff_args};
pub use diff::{
    run_current_diff, run_jj_git_clone, DiffOptions, DiffToolInvocation, ExternalDiffTool,
    PipeDiffTool,
};
pub use error::JjError;
use error::*;
use facts::*;
#[cfg(test)]
use fetch::*;
use git_transport::*;
use log::*;
use push::*;
#[cfg(test)]
use status::*;
pub use types::*;
pub use workspace::JjWorkspace;
pub use workspace_management::{
    current_workspace_entry, jj_workspace_entries, remove_jj_workspace, run_jj_git_init,
    run_jj_workspace_add,
};
#[cfg(test)]
use workspace_management::{
    remove_empty_workspace_dirs, validate_workspace_shared_paths_untracked,
    workspace_names_from_jj_list, workspace_root_is_missing_recorded_path,
};

const SHORT_COMMIT_ID_LEN: usize = 8;
const GIT_BACKEND_NAME: &str = "git";
const PREFERRED_TRUNK_BRANCHES: [&str; 2] = ["main", "master"];

type BookmarkPushUpdate = (RefNameBuf, Diff<Option<CommitId>>);

#[cfg(test)]
mod tests;
