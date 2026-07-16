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
    process::{Command as ProcessCommand, Stdio},
    time::Duration,
};

use clap::{error::ErrorKind, Arg, ArgAction, ArgGroup, ArgMatches, Command as ClapCommand};
use dialoguer::{theme::Theme, Confirm, MultiSelect, Select};
use indicatif::{ProgressBar, ProgressStyle};
use jj_cli::formatter::{Formatter, PlainTextFormatter};
use termimad::MadSkin;
use terminal_size::{terminal_size, Width};
use thiserror::Error;

use crate::{
    domain::{
        self, apply_local_stack_branches, prune_merged_stack_metadata_trees,
        refresh_stack_metadata_pull_requests, stack_metadata_from_pull_requests,
        upsert_stack_metadata_pull_requests, BookmarkAction, CheckReport, FetchReport,
        ForkStatusReport, ForkStatusState, PullRequestAction, PullRequestEventEffect,
        PullRequestEventEffectKind, PullRequestPlan, PullRequestPublishOptions,
        PullRequestReadiness, PullRequestReport, PullRequestStackNode, PullRequestStackRow,
        PullRequestStackSelection, PullRequestStackSnapshot, PullRequestStackStatusReport,
        PushPlan, PushReport, RemoteStatusReport, RepositorySummary, StatusReport, StatusState,
        SyncReport, TrackedPushReport, WorkflowError,
    },
    github::{
        GitHubClient, GitHubUserProfile, OctocrabGitHubClient, PullRequestCheckStatus,
        PullRequestHead, PullRequestLabel, PullRequestRecord, PullRequestReviewActivity,
        PullRequestReviewRequests, PullRequestReviewStatus, PullRequestReviewerResponse,
        PullRequestStatusRecord, PullRequestTimelineEvent, PullRequestTimelineEventKind,
        RepositoryCreation, ReviewerCandidate, ReviewerSelection, ReviewerTarget,
    },
    jj::{
        current_workspace_entry, jj_workspace_entries, remove_jj_workspace, run_current_diff,
        run_jj_git_clone, run_jj_git_init, run_jj_workspace_add, AdvanceTrunkOutcome,
        BookmarkUpdate, BootstrapPushOutcome, CommitDescriptionRewrite, DiffOptions,
        DiffToolInvocation, ExternalDiffTool, FetchOutcome, FetchTraceAttr, FetchTraceStep,
        FetchTraceValue, InitialPublishTarget, JjError, JjWorkspace, LocalStackBranch,
        LocalStackBranchFacts, LocalStackBranchMetrics, PipeDiffTool, PushBookmarksMetrics,
        PushBookmarksOutcome, PushOutcome, PushedBookmarkSummary, StackMoveOutcome,
        StackMoveTarget, StackPlanFacts, StackPlanSelection, StackPublishFacts,
        StackPublishSelection, StatusWorkspaceFacts, StatusWorkspaceMetrics, SyncPushMetrics,
        SyncPushMetricsOutcome, SyncPushOptions, SyncPushOutcome, TrackedPushOutcome,
        WorkingCopySnapshot, WorkspaceAddOptions, WorkspaceEntry, WorkspaceFacts,
        WorkspaceRemoveOptions, WorkspaceStatus, WorkspaceVisibility,
    },
    repository::{
        read_github_user_name_cache, read_stack_metadata, read_workspace_metadata,
        validate_workspace_name, write_github_user_name_cache, write_stack_metadata,
        write_workspace_metadata, ClonePlan, DiffToolConfig, GitHubRemote, GitHubRepository,
        LayoutConfig, LocalRepositoryContext, PullRequestStore, PullRequestWithHistory,
        RepoCheckConfig, RepoCheckTrigger, RepoHook, RepoHookEvent, RepoWorkItemEvent,
        RepoWorkItemHandler, RepositoryContext, RepositoryError, RepositoryIdentity,
        RuntimeEnvironment, ShellConfig, ShellZoxideMode, StackMetadata, TokenSource,
        WorkflowConfig, WorkspaceMetadata,
    },
};

mod checks;
mod dashboard;
mod handlers;
mod hooks;
mod perf;
mod progress;
mod prompts;
mod pull_request_manager;
mod render;
mod request;
mod review;
mod services;
mod shell;
mod stack;
mod work;

use checks::*;
use dashboard::*;
use handlers::*;
use hooks::*;
use perf::*;
use progress::*;
use prompts::*;
pub use prompts::{
    PullRequestConfirmationError, PullRequestSelectionError, PushConfirmationError,
    RepositoryCreationConfirmationError, RepositoryInitializationConfirmationError,
    ReviewerSelectionError, WorkspaceRemoveConfirmationError,
};
use pull_request_manager::*;
use render::*;
use request::*;
use review::*;
use services::*;
use shell::*;
use stack::*;
use work::*;

static YES_CONFIRMER: YesConfirmer = YesConfirmer;

const SHELL_CD_TARGET_PREFIX: &str = "__jx_cd_target=";

/// Structured output returned by command orchestration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub stdout: String,
    pub exit_code: u8,
}

impl CommandResult {
    fn success(stdout: String) -> Self {
        Self {
            stdout,
            exit_code: 0,
        }
    }

    fn with_exit_code(stdout: String, exit_code: u8) -> Self {
        Self { stdout, exit_code }
    }
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
    Workflow(Box<WorkflowError>),
    #[error(transparent)]
    PullRequestSelection(#[from] PullRequestSelectionError),
    #[error(transparent)]
    ReviewerSelection(#[from] ReviewerSelectionError),
    #[error(transparent)]
    PullRequestConfirmation(#[from] PullRequestConfirmationError),
    #[error(transparent)]
    PushConfirmation(#[from] PushConfirmationError),
    #[error(transparent)]
    RepositoryInitializationConfirmation(#[from] RepositoryInitializationConfirmationError),
    #[error(transparent)]
    RepositoryCreationConfirmation(#[from] RepositoryCreationConfirmationError),
    #[error(transparent)]
    WorkspaceRemoveConfirmation(#[from] WorkspaceRemoveConfirmationError),
    #[error("Work item handler `{handler}` failed for {work_id}: {message}")]
    WorkItemHandler {
        handler: String,
        work_id: String,
        message: String,
    },
    #[error("{message}")]
    Check { message: String },
    #[error("{message}")]
    Hook { message: String },
    #[error("Workspace `{workspace}` was created at {destination}, but post-create setup failed: {message}. The workspace was not rolled back; repair or delete it manually.")]
    WorkAddSetup {
        workspace: String,
        destination: PathBuf,
        message: String,
    },
}

impl From<WorkflowError> for CommandError {
    fn from(error: WorkflowError) -> Self {
        Self::Workflow(Box::new(error))
    }
}

impl From<WorkAddSetupError> for CommandError {
    fn from(error: WorkAddSetupError) -> Self {
        Self::WorkAddSetup {
            workspace: error.workspace().to_owned(),
            destination: error.destination().to_path_buf(),
            message: error.to_string(),
        }
    }
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
    let pull_request_selector = TerminalPullRequestSelector;
    let reviewer_selector = TerminalReviewerSelector;
    let pull_request_confirmer = TerminalPullRequestConfirmer;
    let push_confirmer = TerminalPushConfirmer;
    let repository_initialization_confirmer = TerminalRepositoryInitializationConfirmer;
    let repository_creation_confirmer = TerminalRepositoryCreationConfirmer;
    let workspace_remove_confirmer = TerminalWorkspaceRemoveConfirmer;
    let prompts = PromptHandlers {
        pull_request_previewer: &previewer,
        pull_request_selector: &pull_request_selector,
        reviewer_selector: &reviewer_selector,
        pull_request_confirmer: &pull_request_confirmer,
        push_confirmer: &push_confirmer,
        repository_initialization_confirmer: &repository_initialization_confirmer,
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
fn run_with_args_and_pull_request_selector<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    pull_request_selector: &dyn PullRequestSelector,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        pull_request_selector,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
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
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector,
        pull_request_confirmer,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
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
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
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
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
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
fn run_with_args_and_repository_initialization_confirmer<I, T>(
    args: I,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    repository_initialization_confirmer: &dyn RepositoryInitializationConfirmer,
) -> Result<CommandResult, CommandError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let prompts = PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer,
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
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
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
    terminal_width: Option<usize>,
}

impl OutputMode {
    fn from_process() -> Self {
        let is_terminal = io::stdout().is_terminal();
        Self {
            color: is_terminal,
            terminal_width: is_terminal.then(detected_terminal_width).flatten(),
        }
    }

    #[cfg(test)]
    fn plain() -> Self {
        Self {
            color: false,
            terminal_width: None,
        }
    }

    #[cfg(test)]
    fn plain_with_width(width: usize) -> Self {
        Self {
            color: false,
            terminal_width: Some(width),
        }
    }
}

fn detected_terminal_width() -> Option<usize> {
    terminal_size().map(|(Width(width), _)| usize::from(width))
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
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let perf = PerfLog::from_environment(environment);
    let mut span = perf.start(
        "command.run",
        [
            perf_attr("arg_count", args.len().saturating_sub(1)),
            perf_attr("color", output.color),
            perf_attr("terminal_width", output.terminal_width.unwrap_or_default()),
        ],
    );

    let parse_step = span.start_step("parse_args", Vec::new());
    let matches = cli().try_get_matches_from(args);
    let parse_error = matches
        .as_ref()
        .err()
        .filter(|error| clap_exit_code(error) != 0);
    span.finish_step(parse_step, Vec::new(), parse_error);
    let matches = match matches {
        Ok(matches) => matches,
        Err(error) => {
            record_clap_error(&mut span, &error);
            return Err(error.into());
        }
    };

    let yes = matches.get_flag("yes");
    span.set([perf_attr("yes", yes)]);
    let prompts = if yes {
        PromptHandlers {
            pull_request_previewer: prompts.pull_request_previewer,
            pull_request_selector: prompts.pull_request_selector,
            reviewer_selector: prompts.reviewer_selector,
            pull_request_confirmer: &YES_CONFIRMER,
            push_confirmer: &YES_CONFIRMER,
            repository_initialization_confirmer: &YES_CONFIRMER,
            repository_creation_confirmer: &YES_CONFIRMER,
            workspace_remove_confirmer: &YES_CONFIRMER,
        }
    } else {
        prompts
    };

    let request_step = span.start_step("build_request", Vec::new());
    let request = CommandRequest::from_matches(&matches);
    let request_error = request
        .as_ref()
        .err()
        .filter(|error| clap_exit_code(error) != 0);
    span.finish_step(request_step, Vec::new(), request_error);
    let request = match request {
        Ok(request) => request,
        Err(error) => {
            record_clap_error(&mut span, &error);
            return Err(error.into());
        }
    };
    span.set(request.perf_attrs());

    let result = span.measure("handle_request", Vec::new(), || {
        handle_request(request, environment, services, progress, prompts, output)
    });
    record_command_result(&mut span, &result);
    result
}

fn record_command_result(span: &mut PerfSpan, result: &Result<CommandResult, CommandError>) {
    match result {
        Ok(result) => {
            span.set([perf_attr("exit_code", u64::from(result.exit_code))]);
            if result.exit_code != 0 {
                span.record_error(format!("exit code {}", result.exit_code));
            }
        }
        Err(CommandError::Usage(error)) => record_clap_error(span, error),
        Err(error) => {
            span.set([perf_attr("exit_code", 1_u64)]);
            span.record_error(error);
        }
    }
}

fn record_clap_error(span: &mut PerfSpan, error: &clap::Error) {
    let exit_code = clap_exit_code(error);
    span.set([
        perf_attr("exit_code", u64::from(exit_code)),
        perf_attr("error_kind", format!("{:?}", error.kind())),
    ]);
    if exit_code != 0 {
        span.record_error(error);
    }
}

fn clap_exit_code(error: &clap::Error) -> u8 {
    u8::try_from(error.exit_code()).unwrap_or(1)
}

#[cfg(test)]
mod tests;
