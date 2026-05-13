//! Command orchestration for the `jx` CLI surface.
//!
//! Command handlers stay thin: they parse CLI requests, load fixed-origin
//! repository context, acquire jj/GitHub boundary facts, delegate readiness,
//! freshness, bookmark, PR, and diff decisions to domain services, and render
//! concise operator output. Remote-status reports every configured GitHub
//! remote; fetch, push, sync, and PR mutation stay behind the jj and GitHub
//! boundaries.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fmt, io,
    io::IsTerminal as _,
    io::Write as _,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::Duration,
};

use clap::{error::ErrorKind, Arg, ArgAction, ArgMatches, Command as ClapCommand};
use dialoguer::{theme::Theme, MultiSelect, Select};
use indicatif::{ProgressBar, ProgressStyle};
use jj_cli::formatter::{Formatter, PlainTextFormatter};
use termimad::MadSkin;
use thiserror::Error;

use crate::{
    domain::{
        self, BookmarkAction, CheckReport, FetchReport, ForkStatusReport, ForkStatusState,
        PullRequestAction, PullRequestPlan, PullRequestReport, PushPlan, PushReport,
        RebaseOnTrunkReport, RemoteStatusReport, StatusReport, SyncReport, TrackedPushReport,
        WorkflowCommand, WorkflowError,
    },
    github::{
        GitHubClient, OctocrabGitHubClient, PullRequestHead, PullRequestRecord, RepositoryCreation,
        ReviewerCandidate, ReviewerSelection, ReviewerTarget,
    },
    jj::{
        current_workspace_entry, jj_workspace_entries, remove_jj_workspace, run_current_diff,
        run_jj_git_clone, run_jj_git_init, run_jj_workspace_add, AdvanceTrunkOutcome,
        BookmarkUpdate, BootstrapPushOutcome, DiffOptions, DiffToolInvocation, ExternalDiffTool,
        FetchOutcome, InitialPublishTarget, JjError, JjWorkspace, PipeDiffTool, PushOutcome,
        RebaseOnTrunkOutcome, StatusWorkspaceFacts, TrackedPushOutcome, WorkspaceAddOptions,
        WorkspaceEntry, WorkspaceFacts, WorkspaceRemoveOptions, WorkspaceStatus,
    },
    repository::{
        read_workspace_metadata, validate_workspace_name, write_workspace_metadata, ClonePlan,
        DiffToolConfig, GitHubRemote, GitHubRepository, LayoutConfig, LocalRepositoryContext,
        RepositoryContext, RepositoryError, RepositoryIdentity, RuntimeEnvironment, ShellConfig,
        ShellZoxideMode, TokenSource, WorkflowConfig, WorkspaceMetadata,
    },
};

mod handlers;
mod progress;
mod prompts;
mod render;
mod request;
mod services;
mod shell;
mod work;

use handlers::*;
use progress::*;
use prompts::*;
pub use prompts::{
    PullRequestConfirmationError, PushConfirmationError, RepositoryCreationConfirmationError,
    ReviewerSelectionError, WorkspaceRemoveConfirmationError,
};
use render::*;
use request::*;
use services::*;
use shell::*;
use work::*;

const SHELL_CD_TARGET_PREFIX: &str = "__jx_cd_target=";

/// Structured output returned by command orchestration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub stdout: String,
}

/// Errors returned by command orchestration before rendering in the binary.
#[derive(Debug, Error)]
pub enum CommandError {
    #[error(transparent)]
    Usage(#[from] clap::Error),
    #[error(transparent)]
    Environment(#[from] io::Error),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Jj(#[from] JjError),
    #[error(transparent)]
    Workflow(#[from] WorkflowError),
    #[error(transparent)]
    ReviewerSelection(#[from] ReviewerSelectionError),
    #[error(transparent)]
    PullRequestConfirmation(#[from] PullRequestConfirmationError),
    #[error(transparent)]
    PushConfirmation(#[from] PushConfirmationError),
    #[error(transparent)]
    RepositoryCreationConfirmation(#[from] RepositoryCreationConfirmationError),
    #[error(transparent)]
    WorkspaceRemoveConfirmation(#[from] WorkspaceRemoveConfirmationError),
}

/// Runs command orchestration using process arguments and environment.
pub fn run_from_process() -> Result<CommandResult, CommandError> {
    install_interrupt_cursor_restore()?;
    let environment = RuntimeEnvironment::from_process()?;
    run_with_args(env::args_os(), &environment)
}

/// Runs command orchestration with injected arguments and environment.
pub fn run_with_args<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let services = ProductionServices::new(environment)?;
    let progress = SpinnerProgress::new();
    let previewer = TerminalPullRequestPreviewer;
    let reviewer_selector = TerminalReviewerSelector;
    let pull_request_confirmer = TerminalPullRequestConfirmer;
    let push_confirmer = TerminalPushConfirmer;
    let repository_creation_confirmer = TerminalRepositoryCreationConfirmer;
    let workspace_remove_confirmer = TerminalWorkspaceRemoveConfirmer;
    let prompts = PromptHandlers {
        pull_request_previewer: &previewer,
        reviewer_selector: &reviewer_selector,
        pull_request_confirmer: &pull_request_confirmer,
        push_confirmer: &push_confirmer,
        repository_creation_confirmer: &repository_creation_confirmer,
        workspace_remove_confirmer: &workspace_remove_confirmer,
    };
    run_with_args_and_progress(
        args,
        environment,
        &services,
        &progress,
        prompts,
        OutputMode::from_process(),
    )
}

#[cfg(test)]
fn run_with_args_and_services<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_with_args_and_prompts(
        args,
        environment,
        services,
        &SelectAllReviewers,
        &AlwaysConfirmPullRequest,
    )
}

#[cfg(test)]
fn run_with_args_and_reviewer_selector<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    reviewer_selector: &dyn ReviewerSelector,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_with_args_and_prompts(
        args,
        environment,
        services,
        reviewer_selector,
        &AlwaysConfirmPullRequest,
    )
}

#[cfg(test)]
fn run_with_args_and_prompts<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    reviewer_selector: &dyn ReviewerSelector,
    pull_request_confirmer: &dyn PullRequestConfirmer,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        reviewer_selector,
        pull_request_confirmer,
        push_confirmer: &AlwaysConfirmPush,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };
    run_with_args_and_progress(
        args,
        environment,
        services,
        &NoProgress,
        prompts,
        OutputMode::plain(),
    )
}

#[cfg(test)]
fn run_with_args_and_push_confirmer<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    push_confirmer: &dyn PushConfirmer,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };
    run_with_args_and_progress(
        args,
        environment,
        services,
        &NoProgress,
        prompts,
        OutputMode::plain(),
    )
}

#[cfg(test)]
fn run_with_args_and_repository_creation_confirmer<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    repository_creation_confirmer: &dyn RepositoryCreationConfirmer,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_creation_confirmer,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    };
    run_with_args_and_progress(
        args,
        environment,
        services,
        &NoProgress,
        prompts,
        OutputMode::plain(),
    )
}

#[cfg(test)]
fn run_with_args_and_workspace_remove_confirmer<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    workspace_remove_confirmer: &dyn WorkspaceRemoveConfirmer,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer,
    };
    run_with_args_and_progress(
        args,
        environment,
        services,
        &NoProgress,
        prompts,
        OutputMode::plain(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutputMode {
    color: bool,
}

impl OutputMode {
    fn from_process() -> Self {
        Self {
            color: io::stdout().is_terminal(),
        }
    }

    #[cfg(test)]
    fn plain() -> Self {
        Self { color: false }
    }
}

fn run_with_args_and_progress<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    prompts: PromptHandlers<'_>,
    output: OutputMode,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let matches = cli().try_get_matches_from(args)?;
    let request = CommandRequest::from_matches(&matches)?;
    handle_request(request, environment, services, progress, prompts, output)
}

#[cfg(test)]
mod tests;
