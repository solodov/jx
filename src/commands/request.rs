use super::*;

/// Parsed operator intent after clap has validated subcommand-specific flags.
pub(super) enum CommandRequest {
    Log,
    Status,
    PreviousCommit,
    NextCommit,
    Diff(DiffRequest),
    Clone(CloneRequest),
    Work(WorkRequest),
    Stack(StackRequest),
    Shell(ShellRequest),
    Open(OpenRequest),
    Review(ReviewRequest),
    RemoteStatus(RemoteStatusRequest),
    Fetch(FetchRequest),
    RebaseOnTrunk(RebaseOnTrunkRequest),
    Push(PushRequest),
    Sync(SyncRequest),
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DiffRequest {
    pub(super) revision: Option<String>,
    pub(super) paths: Vec<String>,
    pub(super) no_tests: bool,
    pub(super) tool: Option<String>,
    pub(super) tool_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CloneRequest {
    pub(super) repository: String,
    pub(super) destination: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WorkRequest {
    Add(WorkAddRequest),
    List(WorkListRequest),
    Complete(WorkCompleteRequest),
    Root(WorkRootRequest),
    Trunk(WorkTrunkRequest),
    Delete(WorkDeleteRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StackRequest {
    Show,
    Open {
        print: bool,
    },
    Refresh,
    Move {
        target: StackMoveTarget,
        no_sync: bool,
    },
    Plan(StackPlanRequest),
    Publish(StackPublishRequest),
    CompleteReviewers(StackReviewerCompleteRequest),
    Status(StackStatusRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StackReviewerCompleteRequest {
    pub(super) prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StackStatusRequest {
    pub(super) all: bool,
    pub(super) repo_filters: Vec<String>,
    pub(super) parallelism: usize,
    pub(super) format: StackStatusFormat,
    pub(super) interactive: bool,
    pub(super) refresh_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StackStatusFormat {
    Human,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StackPublishRequest {
    pub(super) revisions: Vec<String>,
    pub(super) task_id: Option<String>,
    pub(super) no_task_id: bool,
    pub(super) labels: Vec<String>,
    pub(super) reviewers: Vec<ReviewerTarget>,
    pub(super) fixes: Vec<String>,
    pub(super) fixes_attached: bool,
    pub(super) ready: Vec<StackPublishReadinessSelector>,
    pub(super) draft: Vec<StackPublishReadinessSelector>,
    pub(super) apply_to_stack: bool,
    pub(super) no_event_handlers: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StackPublishReadinessSelector {
    All,
    Revisions(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StackPlanRequest {
    pub(super) revisions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkAddRequest {
    pub(super) name: String,
    pub(super) revision: Option<String>,
    pub(super) task_id: Option<String>,
    pub(super) shell_cd_target: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkListRequest {
    pub(super) all: bool,
    pub(super) prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkCompleteRequest {
    pub(super) prefix: String,
    pub(super) repositories: bool,
    pub(super) workspaces: bool,
    pub(super) navigation: bool,
    pub(super) format: WorkCompleteFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkCompleteFormat {
    Simple,
    Picker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkRootRequest {
    pub(super) key: String,
    pub(super) navigation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkTrunkRequest {
    pub(super) shell_cd_target: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkDeleteRequest {
    pub(super) name: Option<String>,
    pub(super) shell_cd_target: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ShellRequest {
    Init(ShellInitRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShellInitRequest {
    pub(super) shell: ShellKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpenRequest {
    pub(super) target: OpenTarget,
    pub(super) repository: Option<String>,
    pub(super) repo_filters: Vec<String>,
    pub(super) print: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OpenTarget {
    Repository,
    PullRequest { selector: Option<String> },
    PullRequests { all: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewRequest {
    pub(super) repo_filters: Vec<String>,
    pub(super) interactive: bool,
    pub(super) refresh_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RemoteStatusRequest {
    pub(super) all: bool,
    pub(super) repository: Option<String>,
    pub(super) repo_filters: Vec<String>,
    pub(super) changed: bool,
    pub(super) parallelism: usize,
    pub(super) format: RemoteStatusFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RemoteStatusFormat {
    Human,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FetchRequest {
    pub(super) all: bool,
    pub(super) repository: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SyncRequest {
    pub(super) all: bool,
    pub(super) repo: bool,
    pub(super) stack: bool,
    pub(super) revision: Option<String>,
    pub(super) repo_filters: Vec<String>,
    pub(super) sync_push_options: SyncPushOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RebaseOnTrunkRequest {
    pub(super) sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PushRequest {
    pub(super) revision: Option<String>,
    pub(super) tracked: bool,
}

impl CommandRequest {
    pub(super) fn from_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        match matches.subcommand() {
            Some(("diff", matches)) => Ok(Self::Diff(DiffRequest {
                revision: revision(matches),
                paths: diff_paths(matches),
                no_tests: matches.get_flag("no-tests"),
                tool: diff_tool(matches)?,
                tool_args: diff_tool_args(matches),
            })),
            Some(("clone", matches)) => Ok(Self::Clone(CloneRequest {
                repository: required_arg(matches, "repository"),
                destination: matches.get_one::<PathBuf>("destination").cloned(),
            })),
            Some(("work", matches)) => Ok(Self::Work(work_request(matches)?)),
            Some(("stack" | "stk", matches)) => Ok(Self::Stack(stack_request(matches)?)),
            Some(("shell", matches)) => Ok(Self::Shell(shell_request(matches)?)),
            Some(("open" | "o", matches)) => Ok(Self::Open(open_request(matches))),
            Some(("review", matches)) => Ok(Self::Review(ReviewRequest {
                repo_filters: review_repo_filters(matches),
                interactive: matches.get_flag("interactive"),
                refresh_seconds: dashboard_refresh_seconds(matches)?,
            })),
            Some(("status" | "st", _)) => Ok(Self::Status),
            Some(("prev-commit" | "prev", _)) => Ok(Self::PreviousCommit),
            Some(("next-commit" | "next", _)) => Ok(Self::NextCommit),
            Some(("check", _)) => Ok(Self::Check),
            Some(("remote-status" | "rs", matches)) => {
                Ok(Self::RemoteStatus(RemoteStatusRequest {
                    all: matches.get_flag("all"),
                    repository: repository_arg(matches),
                    repo_filters: repo_filters(matches),
                    changed: matches.get_flag("changed"),
                    parallelism: remote_status_parallelism(matches)?,
                    format: remote_status_format(matches),
                }))
            }
            Some(("fetch" | "f", matches)) => Ok(Self::Fetch(FetchRequest {
                all: matches.get_flag("all"),
                repository: repository_arg(matches),
            })),
            Some(("rebase-on-trunk" | "rt", matches)) => {
                Ok(Self::RebaseOnTrunk(RebaseOnTrunkRequest {
                    sources: sources(matches),
                }))
            }
            Some(("push", matches)) => Ok(Self::Push(PushRequest {
                revision: revision(matches),
                tracked: matches.get_flag("tracked"),
            })),
            Some(("sync", matches)) => Ok(Self::Sync(sync_request(matches)?)),
            None => Ok(Self::Log),
            _ => unreachable!("clap rejects unknown subcommands"),
        }
    }

    pub(super) fn perf_attrs(&self) -> Vec<PerfAttr> {
        let command_path = self.command_path();
        let mut attrs = vec![
            perf_attr(
                "command",
                command_path
                    .split_once('.')
                    .map_or(command_path, |(command, _)| command),
            ),
            perf_attr("command_path", command_path),
        ];
        if let Some((_, subcommand)) = command_path.split_once('.') {
            attrs.push(perf_attr("subcommand", subcommand));
        }

        match self {
            Self::Log | Self::Status | Self::PreviousCommit | Self::NextCommit | Self::Check => {}
            Self::Diff(request) => attrs.extend([
                perf_attr("has_revision", request.revision.is_some()),
                perf_attr("path_count", request.paths.len()),
                perf_attr("no_tests", request.no_tests),
                perf_attr("has_tool", request.tool.is_some()),
                perf_attr("tool_arg_count", request.tool_args.len()),
            ]),
            Self::Clone(request) => {
                attrs.extend([perf_attr("has_destination", request.destination.is_some())])
            }
            Self::Work(request) => add_work_perf_attrs(&mut attrs, request),
            Self::Stack(request) => add_stack_perf_attrs(&mut attrs, request),
            Self::Shell(ShellRequest::Init(request)) => {
                attrs.extend([perf_attr("shell", shell_kind_name(request.shell))])
            }
            Self::Open(request) => add_open_perf_attrs(&mut attrs, request),
            Self::Review(request) => attrs.extend([
                perf_attr("repo_filter_count", request.repo_filters.len()),
                perf_attr("interactive", request.interactive),
                perf_attr("refresh_seconds", request.refresh_seconds),
            ]),
            Self::RemoteStatus(request) => attrs.extend([
                perf_attr("all", request.all),
                perf_attr("has_repository", request.repository.is_some()),
                perf_attr("repo_filter_count", request.repo_filters.len()),
                perf_attr("changed", request.changed),
                perf_attr("parallelism", request.parallelism),
                perf_attr("format", remote_status_format_name(request.format)),
            ]),
            Self::Fetch(request) => attrs.extend([
                perf_attr("all", request.all),
                perf_attr("has_repository", request.repository.is_some()),
            ]),
            Self::RebaseOnTrunk(request) => {
                attrs.extend([perf_attr("source_count", request.sources.len())])
            }
            Self::Push(request) => attrs.extend([
                perf_attr("has_revision", request.revision.is_some()),
                perf_attr("tracked", request.tracked),
            ]),
            Self::Sync(request) => attrs.extend([
                perf_attr("mode", sync_mode(request)),
                perf_attr("all", request.all),
                perf_attr("repo", request.repo),
                perf_attr("stack", request.stack),
                perf_attr("has_revision", request.revision.is_some()),
                perf_attr("repo_filter_count", request.repo_filters.len()),
                perf_attr(
                    "skip_same_tree_pushes",
                    request.sync_push_options.skip_same_tree_pushes,
                ),
            ]),
        }
        attrs
    }

    fn command_path(&self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Status => "status",
            Self::PreviousCommit => "prev-commit",
            Self::NextCommit => "next-commit",
            Self::Diff(_) => "diff",
            Self::Clone(_) => "clone",
            Self::Work(WorkRequest::Add(_)) => "work.add",
            Self::Work(WorkRequest::List(_)) => "work.list",
            Self::Work(WorkRequest::Complete(_)) => "work.complete",
            Self::Work(WorkRequest::Root(_)) => "work.root",
            Self::Work(WorkRequest::Trunk(_)) => "work.trunk",
            Self::Work(WorkRequest::Delete(_)) => "work.delete",
            Self::Stack(StackRequest::Show) => "stack.show",
            Self::Stack(StackRequest::Open { .. }) => "stack.open",
            Self::Stack(StackRequest::Refresh) => "stack.refresh",
            Self::Stack(StackRequest::Move { .. }) => "stack.move",
            Self::Stack(StackRequest::Plan(_)) => "stack.plan",
            Self::Stack(StackRequest::Publish(_)) => "stack.publish",
            Self::Stack(StackRequest::CompleteReviewers(_)) => "stack.complete-reviewers",
            Self::Stack(StackRequest::Status(_)) => "stack.status",
            Self::Shell(ShellRequest::Init(_)) => "shell.init",
            Self::Open(OpenRequest {
                target: OpenTarget::Repository,
                ..
            }) => "open.repository",
            Self::Open(OpenRequest {
                target: OpenTarget::PullRequest { .. },
                ..
            }) => "open.pr",
            Self::Open(OpenRequest {
                target: OpenTarget::PullRequests { .. },
                ..
            }) => "open.prs",
            Self::Review(_) => "review",
            Self::RemoteStatus(_) => "remote-status",
            Self::Fetch(_) => "fetch",
            Self::RebaseOnTrunk(_) => "rebase-on-trunk",
            Self::Push(_) => "push",
            Self::Sync(_) => "sync",
            Self::Check => "check",
        }
    }
}

fn add_work_perf_attrs(attrs: &mut Vec<PerfAttr>, request: &WorkRequest) {
    match request {
        WorkRequest::Add(request) => attrs.extend([
            perf_attr("has_revision", request.revision.is_some()),
            perf_attr("has_task_id", request.task_id.is_some()),
            perf_attr("shell_cd_target", request.shell_cd_target),
        ]),
        WorkRequest::List(request) => attrs.extend([
            perf_attr("all", request.all),
            perf_attr("has_prefix", !request.prefix.is_empty()),
        ]),
        WorkRequest::Complete(request) => attrs.extend([
            perf_attr("has_prefix", !request.prefix.is_empty()),
            perf_attr("repositories", request.repositories),
            perf_attr("workspaces", request.workspaces),
            perf_attr("navigation", request.navigation),
            perf_attr("format", work_complete_format_name(request.format)),
        ]),
        WorkRequest::Root(request) => attrs.extend([perf_attr("navigation", request.navigation)]),
        WorkRequest::Trunk(request) => {
            attrs.extend([perf_attr("shell_cd_target", request.shell_cd_target)])
        }
        WorkRequest::Delete(request) => attrs.extend([
            perf_attr("has_name", request.name.is_some()),
            perf_attr("shell_cd_target", request.shell_cd_target),
        ]),
    }
}

fn add_stack_perf_attrs(attrs: &mut Vec<PerfAttr>, request: &StackRequest) {
    match request {
        StackRequest::Show | StackRequest::Refresh => {}
        StackRequest::Open { print } => attrs.extend([perf_attr("print", *print)]),
        StackRequest::Move { target, no_sync } => attrs.extend([
            perf_attr("target", stack_move_target_name(target)),
            perf_attr("no_sync", *no_sync),
        ]),
        StackRequest::Plan(request) => attrs.extend([
            perf_attr("revision_count", request.revisions.len()),
            perf_attr("explicit_revisions", !request.revisions.is_empty()),
        ]),
        StackRequest::Publish(request) => attrs.extend([
            perf_attr("revision_count", request.revisions.len()),
            perf_attr("explicit_revisions", !request.revisions.is_empty()),
            perf_attr("has_task_id", request.task_id.is_some()),
            perf_attr("no_task_id", request.no_task_id),
            perf_attr("label_count", request.labels.len()),
            perf_attr("reviewer_arg_count", request.reviewers.len()),
            perf_attr("fixes_count", request.fixes.len()),
            perf_attr("fixes_attached", request.fixes_attached),
            perf_attr("ready_selector_count", request.ready.len()),
            perf_attr("draft_selector_count", request.draft.len()),
            perf_attr("apply_to_stack", request.apply_to_stack),
            perf_attr("event_handlers", !request.no_event_handlers),
        ]),
        StackRequest::CompleteReviewers(request) => {
            attrs.extend([perf_attr("has_prefix", !request.prefix.is_empty())])
        }
        StackRequest::Status(request) => attrs.extend([
            perf_attr("all", request.all),
            perf_attr("repo_filter_count", request.repo_filters.len()),
            perf_attr("parallelism", request.parallelism),
            perf_attr("format", stack_status_format_name(request.format)),
            perf_attr("interactive", request.interactive),
            perf_attr("refresh_seconds", request.refresh_seconds),
        ]),
    }
}

fn work_complete_format_name(format: WorkCompleteFormat) -> &'static str {
    match format {
        WorkCompleteFormat::Simple => "simple",
        WorkCompleteFormat::Picker => "picker",
    }
}

fn stack_status_format_name(format: StackStatusFormat) -> &'static str {
    match format {
        StackStatusFormat::Human => "human",
        StackStatusFormat::Json => "json",
    }
}

fn add_open_perf_attrs(attrs: &mut Vec<PerfAttr>, request: &OpenRequest) {
    attrs.extend([
        perf_attr("has_repository", request.repository.is_some()),
        perf_attr("repo_filter_count", request.repo_filters.len()),
        perf_attr("print", request.print),
    ]);
    match &request.target {
        OpenTarget::Repository => {}
        OpenTarget::PullRequest { selector } => {
            attrs.extend([perf_attr("has_selector", selector.is_some())]);
        }
        OpenTarget::PullRequests { all } => {
            attrs.extend([perf_attr("all", *all)]);
        }
    }
}

fn stack_move_target_name(target: &StackMoveTarget) -> &'static str {
    match target {
        StackMoveTarget::Onto(_) => "onto",
        StackMoveTarget::Trunk => "trunk",
    }
}

fn shell_kind_name(shell: ShellKind) -> &'static str {
    match shell {
        ShellKind::Bash => "bash",
    }
}

fn remote_status_format_name(format: RemoteStatusFormat) -> &'static str {
    match format {
        RemoteStatusFormat::Human => "human",
        RemoteStatusFormat::Json => "json",
    }
}

fn sync_mode(request: &SyncRequest) -> &'static str {
    if request.all {
        "all"
    } else if request.repo {
        "repo"
    } else if request.stack {
        "stack"
    } else if request.revision.is_some() {
        "revision"
    } else {
        "tracked"
    }
}

fn required_arg(matches: &ArgMatches, name: &str) -> String {
    matches
        .get_one::<String>(name)
        .cloned()
        .expect("clap enforces required arguments")
}

fn string_arg(matches: &ArgMatches, name: &str) -> Option<String> {
    matches.get_one::<String>(name).cloned()
}

fn work_request(matches: &ArgMatches) -> Result<WorkRequest, clap::Error> {
    match matches.subcommand() {
        Some(("add", matches)) => Ok(WorkRequest::Add(WorkAddRequest {
            name: required_arg(matches, "name"),
            revision: revision(matches),
            task_id: task_id(matches),
            shell_cd_target: matches.get_flag("shell-cd-target"),
        })),
        Some(("list", matches)) => Ok(WorkRequest::List(WorkListRequest {
            all: matches.get_flag("all"),
            prefix: string_arg(matches, "prefix").unwrap_or_default(),
        })),
        Some(("complete", matches)) => Ok(WorkRequest::Complete(WorkCompleteRequest {
            prefix: string_arg(matches, "prefix").unwrap_or_default(),
            repositories: matches.get_flag("repositories"),
            workspaces: matches.get_flag("workspaces"),
            navigation: matches.get_flag("navigation"),
            format: work_complete_format(matches),
        })),
        Some(("root", matches)) => Ok(WorkRequest::Root(WorkRootRequest {
            key: required_arg(matches, "key"),
            navigation: matches.get_flag("navigation"),
        })),
        Some(("trunk", matches)) => Ok(WorkRequest::Trunk(WorkTrunkRequest {
            shell_cd_target: matches.get_flag("shell-cd-target"),
        })),
        Some(("delete", matches)) => Ok(WorkRequest::Delete(WorkDeleteRequest {
            name: string_arg(matches, "name"),
            shell_cd_target: matches.get_flag("shell-cd-target"),
        })),
        _ => Err(clap::Error::raw(
            ErrorKind::MissingSubcommand,
            "`jx work` requires `add`, `list`, `complete`, `root`, `trunk`, or `delete`",
        )),
    }
}

fn stack_request(matches: &ArgMatches) -> Result<StackRequest, clap::Error> {
    if matches.get_flag("interactive") {
        return Ok(StackRequest::Open {
            print: matches.get_flag("print"),
        });
    }

    if let Some(target) = matches.get_one::<String>("onto") {
        return Ok(StackRequest::Move {
            target: StackMoveTarget::Onto(target.clone()),
            no_sync: matches.get_flag("no-sync"),
        });
    }
    if matches.get_flag("trunk") {
        return Ok(StackRequest::Move {
            target: StackMoveTarget::Trunk,
            no_sync: matches.get_flag("no-sync"),
        });
    }

    match matches.subcommand() {
        Some(("show", _)) => Ok(StackRequest::Show),
        Some(("refresh", _)) => Ok(StackRequest::Refresh),
        Some(("plan", matches)) => Ok(StackRequest::Plan(stack_plan_request(matches))),
        Some(("publish", matches)) => Ok(StackRequest::Publish(stack_publish_request(matches)?)),
        Some(("complete-reviewers", matches)) => Ok(StackRequest::CompleteReviewers(
            stack_reviewer_complete_request(matches),
        )),
        Some(("status", matches)) => Ok(StackRequest::Status(stack_status_request(matches)?)),
        _ => Ok(StackRequest::Show),
    }
}

fn stack_publish_request(matches: &ArgMatches) -> Result<StackPublishRequest, clap::Error> {
    Ok(StackPublishRequest {
        revisions: revisions(matches),
        task_id: task_id(matches),
        no_task_id: matches.get_flag("no-task-id"),
        labels: labels(matches),
        reviewers: reviewers(matches)?,
        fixes: fixes(matches),
        fixes_attached: fixes_attached(matches),
        ready: readiness_selectors(matches, "ready"),
        draft: readiness_selectors(matches, "draft"),
        apply_to_stack: matches.get_flag("apply-to-stack"),
        no_event_handlers: matches.get_flag("no-event-handlers"),
    })
}

fn stack_reviewer_complete_request(matches: &ArgMatches) -> StackReviewerCompleteRequest {
    StackReviewerCompleteRequest {
        prefix: string_arg(matches, "prefix").unwrap_or_default(),
    }
}

fn stack_status_request(matches: &ArgMatches) -> Result<StackStatusRequest, clap::Error> {
    let format = stack_status_format(matches);
    let interactive = matches.get_flag("interactive");
    if interactive && format == StackStatusFormat::Json {
        return Err(clap::Error::raw(
            ErrorKind::ArgumentConflict,
            "stack status --interactive cannot be used with --format json",
        ));
    }
    Ok(StackStatusRequest {
        all: matches.get_flag("all"),
        repo_filters: stack_status_repo_filters(matches),
        parallelism: stack_status_parallelism(matches)?,
        format,
        interactive,
        refresh_seconds: dashboard_refresh_seconds(matches)?,
    })
}

fn stack_status_repo_filters(matches: &ArgMatches) -> Vec<String> {
    repo_filter_values(matches, "repository-filter")
}

fn review_repo_filters(matches: &ArgMatches) -> Vec<String> {
    repo_filter_values(matches, "repository-filter")
}

fn repo_filter_values(matches: &ArgMatches, name: &str) -> Vec<String> {
    matches
        .get_many::<String>(name)
        .into_iter()
        .flatten()
        .map(|filter| filter.trim())
        .filter(|filter| !filter.is_empty())
        .map(str::to_owned)
        .collect()
}

fn readiness_selectors(matches: &ArgMatches, name: &str) -> Vec<StackPublishReadinessSelector> {
    matches
        .get_many::<String>(name)
        .into_iter()
        .flatten()
        .map(|value| {
            if value.is_empty() {
                StackPublishReadinessSelector::All
            } else {
                StackPublishReadinessSelector::Revisions(value.clone())
            }
        })
        .collect()
}

fn stack_plan_request(matches: &ArgMatches) -> StackPlanRequest {
    StackPlanRequest {
        revisions: revisions(matches),
    }
}

fn shell_request(matches: &ArgMatches) -> Result<ShellRequest, clap::Error> {
    match matches.subcommand() {
        Some(("init", matches)) => Ok(ShellRequest::Init(ShellInitRequest {
            shell: shell_kind(matches)?,
        })),
        _ => Err(clap::Error::raw(
            ErrorKind::MissingSubcommand,
            "`jx shell` requires `init`",
        )),
    }
}

fn open_request(matches: &ArgMatches) -> OpenRequest {
    match matches.subcommand() {
        Some(("pr", matches)) => OpenRequest {
            target: OpenTarget::PullRequest {
                selector: open_pr_selector(matches),
            },
            repository: None,
            repo_filters: Vec::new(),
            print: matches.get_flag("print"),
        },
        Some(("prs", matches)) => OpenRequest {
            target: OpenTarget::PullRequests {
                all: matches.get_flag("all"),
            },
            repository: repository_arg(matches),
            repo_filters: repo_filters(matches),
            print: matches.get_flag("print"),
        },
        _ => OpenRequest {
            target: OpenTarget::Repository,
            repository: repository_arg(matches),
            repo_filters: repo_filters(matches),
            print: matches.get_flag("print"),
        },
    }
}

fn shell_kind(matches: &ArgMatches) -> Result<ShellKind, clap::Error> {
    match required_arg(matches, "shell").as_str() {
        "bash" => Ok(ShellKind::Bash),
        shell => Err(clap::Error::raw(
            ErrorKind::ValueValidation,
            format!("unsupported shell `{shell}`; supported shells: bash"),
        )),
    }
}

fn task_id(matches: &ArgMatches) -> Option<String> {
    matches.get_one::<String>("task-id").cloned()
}

fn revisions(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>("revision")
        .into_iter()
        .flatten()
        .map(|revision| revision.trim())
        .filter(|revision| !revision.is_empty())
        .map(str::to_owned)
        .collect()
}

fn open_pr_selector(matches: &ArgMatches) -> Option<String> {
    matches
        .get_one::<String>("selector")
        .or_else(|| matches.get_one::<String>("commit"))
        .cloned()
}

fn sources(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>("source")
        .into_iter()
        .flatten()
        .cloned()
        .collect()
}

fn sync_request(matches: &ArgMatches) -> Result<SyncRequest, clap::Error> {
    let all = matches.get_flag("all");
    let targets = sync_targets(matches);
    let revision = if all {
        None
    } else {
        if targets.len() > 1 {
            return Err(clap::Error::raw(
                ErrorKind::ValueValidation,
                "jx sync accepts only one revision unless --all is set",
            ));
        }
        targets.first().cloned()
    };
    let repo_filters = if all { targets } else { Vec::new() };

    Ok(SyncRequest {
        all,
        repo: matches.get_flag("repo"),
        stack: matches.get_flag("stack"),
        revision,
        repo_filters,
        sync_push_options: SyncPushOptions {
            skip_same_tree_pushes: matches.get_flag("experimental-skip-same-tree-push"),
        },
    })
}

fn sync_targets(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>("target")
        .into_iter()
        .flatten()
        .map(|target| target.trim())
        .filter(|target| !target.is_empty())
        .map(str::to_owned)
        .collect()
}

fn repository_arg(matches: &ArgMatches) -> Option<String> {
    matches.get_one::<String>("repository").cloned()
}

fn repo_filters(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>("repo")
        .into_iter()
        .flatten()
        .map(|filter| filter.trim())
        .filter(|filter| !filter.is_empty())
        .map(str::to_owned)
        .collect()
}

fn remote_status_parallelism(matches: &ArgMatches) -> Result<usize, clap::Error> {
    let parallelism = *matches
        .get_one::<usize>("jobs")
        .expect("clap applies the remote-status jobs default");
    if parallelism == 0 {
        return Err(clap::Error::raw(
            ErrorKind::ValueValidation,
            "remote-status jobs must be at least 1",
        ));
    }

    Ok(parallelism)
}

fn remote_status_format(matches: &ArgMatches) -> RemoteStatusFormat {
    match matches
        .get_one::<String>("format")
        .map(String::as_str)
        .expect("clap applies the remote-status format default")
    {
        "human" => RemoteStatusFormat::Human,
        "json" => RemoteStatusFormat::Json,
        _ => unreachable!("clap rejects unsupported remote-status formats"),
    }
}

fn work_complete_format(matches: &ArgMatches) -> WorkCompleteFormat {
    match matches
        .get_one::<String>("format")
        .map(String::as_str)
        .expect("clap applies the work-complete format default")
    {
        "simple" => WorkCompleteFormat::Simple,
        "picker" => WorkCompleteFormat::Picker,
        _ => unreachable!("clap rejects unsupported work-complete formats"),
    }
}

fn stack_status_parallelism(matches: &ArgMatches) -> Result<usize, clap::Error> {
    let parallelism = *matches
        .get_one::<usize>("jobs")
        .expect("clap applies the stack-status jobs default");
    if parallelism == 0 {
        return Err(clap::Error::raw(
            ErrorKind::ValueValidation,
            "stack status jobs must be at least 1",
        ));
    }

    Ok(parallelism)
}

fn stack_status_format(matches: &ArgMatches) -> StackStatusFormat {
    match matches
        .get_one::<String>("format")
        .map(String::as_str)
        .expect("clap applies the stack-status format default")
    {
        "human" => StackStatusFormat::Human,
        "json" => StackStatusFormat::Json,
        _ => unreachable!("clap rejects unsupported stack-status formats"),
    }
}

fn revision(matches: &ArgMatches) -> Option<String> {
    matches.get_one::<String>("revision").cloned()
}

fn diff_tool(matches: &ArgMatches) -> Result<Option<String>, clap::Error> {
    matches
        .get_one::<String>("tool")
        .map(|tool| {
            let tool = tool.trim();
            if tool.is_empty() {
                Err(clap::Error::raw(
                    ErrorKind::ValueValidation,
                    "`--tool` must not be empty",
                ))
            } else {
                Ok(tool.to_owned())
            }
        })
        .transpose()
}

fn diff_paths(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>("path")
        .into_iter()
        .flatten()
        .cloned()
        .collect()
}

fn diff_tool_args(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>("tool-args")
        .into_iter()
        .flatten()
        .cloned()
        .collect()
}

pub(super) fn diff_options(
    config: &WorkflowConfig,
    request: DiffRequest,
) -> Result<DiffOptions, clap::Error> {
    let selected_tool = request.tool.or_else(|| config.diff.default_tool.clone());
    let Some(tool_name) = selected_tool else {
        if request.tool_args.is_empty() {
            return Ok(DiffOptions {
                revision: request.revision,
                paths: request.paths,
                no_tests: request.no_tests,
                tool: DiffToolInvocation::Plain,
            });
        }

        return Err(clap::Error::raw(
            ErrorKind::UnknownArgument,
            "trailing diff tool arguments require a configured `diff.default_tool` or `--tool`",
        ));
    };

    let Some(tool) = config.diff.tools.get(&tool_name) else {
        return Err(clap::Error::raw(
            ErrorKind::ValueValidation,
            unknown_diff_tool_message(&tool_name, config.diff.tools.keys()),
        ));
    };

    Ok(DiffOptions {
        revision: request.revision,
        paths: request.paths,
        no_tests: request.no_tests,
        tool: diff_tool_invocation(tool, request.tool_args),
    })
}

fn diff_tool_invocation(tool: &DiffToolConfig, extra_args: Vec<String>) -> DiffToolInvocation {
    match tool {
        DiffToolConfig::External(tool) => {
            let mut args = tool.args.clone();
            args.extend(extra_args);
            DiffToolInvocation::External(ExternalDiffTool {
                command: tool.command.clone(),
                args,
            })
        }
        DiffToolConfig::Pipe(tool) => {
            let mut args = tool.args.clone();
            args.extend(extra_args);
            DiffToolInvocation::Pipe(PipeDiffTool {
                producer_args: tool.producer_args.clone(),
                command: tool.command.clone(),
                args,
            })
        }
    }
}

fn unknown_diff_tool_message<'a>(
    tool_name: &str,
    configured_tools: impl Iterator<Item = &'a String>,
) -> String {
    let configured_tools = configured_tools.cloned().collect::<Vec<_>>();
    if configured_tools.is_empty() {
        format!("diff tool `{tool_name}` is not configured")
    } else {
        format!(
            "diff tool `{tool_name}` is not configured; configured tools: {}",
            configured_tools.join(", ")
        )
    }
}

fn labels(matches: &ArgMatches) -> Vec<String> {
    let mut labels = Vec::new();
    for label in matches
        .get_many::<String>("label")
        .into_iter()
        .flatten()
        .map(|label| label.trim())
        .filter(|label| !label.is_empty())
    {
        if !labels.iter().any(|existing| existing == label) {
            labels.push(label.to_owned());
        }
    }
    labels
}

fn reviewers(matches: &ArgMatches) -> Result<Vec<ReviewerTarget>, clap::Error> {
    matches
        .get_many::<String>("reviewer")
        .into_iter()
        .flatten()
        .map(|reviewer| {
            ReviewerTarget::parse(reviewer).ok_or_else(|| {
                clap::Error::raw(
                    ErrorKind::ValueValidation,
                    format!(
                        "`{reviewer}` is not a valid reviewer name; use a GitHub login or `org/team`"
                    ),
                )
            })
        })
        .collect()
}

fn fixes(matches: &ArgMatches) -> Vec<String> {
    let mut fixes = Vec::new();
    for work_id in matches
        .get_many::<String>("fixes")
        .into_iter()
        .flatten()
        .map(|work_id| work_id.trim())
        .filter(|work_id| !work_id.is_empty())
    {
        if !fixes.iter().any(|existing| existing == work_id) {
            fixes.push(work_id.to_owned());
        }
    }
    fixes
}

fn fixes_attached(matches: &ArgMatches) -> bool {
    matches
        .get_many::<String>("fixes")
        .into_iter()
        .flatten()
        .any(|work_id| work_id.trim().is_empty())
}

pub(super) fn cli() -> ClapCommand {
    ClapCommand::new("jx")
        .about("Small, opinionated extensions for everyday jj workflows")
        .arg_required_else_help(false)
        .subcommand_required(false)
        .arg(yes_arg())
        .subcommand(
            ClapCommand::new("diff")
                .about("Show a jj diff")
                .arg(diff_revision_arg())
                .arg(no_tests_arg())
                .arg(diff_tool_arg())
                .arg(diff_path_arg())
                .arg(diff_tool_args_arg()),
        )
        .subcommand(
            ClapCommand::new("status")
                .visible_alias("st")
                .about("Show current jj commit status with description"),
        )
        .subcommand(
            ClapCommand::new("clone")
                .about("Clone a repository into the configured jx layout")
                .arg(clone_repository_arg())
                .arg(clone_destination_arg()),
        )
        .subcommand(
            ClapCommand::new("work")
                .about("Manage layout workspaces")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    ClapCommand::new("add")
                        .about("Add a workspace under the configured hidden layout")
                        .arg(workspace_name_arg())
                        .arg(workspace_revision_arg())
                        .arg(task_id_arg())
                        .arg(work_shell_cd_target_arg()),
                )
                .subcommand(
                    ClapCommand::new("list")
                        .about("List jj workspaces and roots")
                        .arg(work_all_arg())
                        .arg(work_prefix_arg()),
                )
                .subcommand(
                    ClapCommand::new("complete")
                        .about("List global work-location completion keys")
                        .arg(work_prefix_arg())
                        .arg(work_repositories_arg())
                        .arg(workspaces_arg())
                        .arg(work_navigation_arg().conflicts_with_all(["repositories", "workspaces"]))
                        .arg(work_complete_format_arg()),
                )
                .subcommand(
                    ClapCommand::new("root")
                        .about("Resolve a global work-location key to its root path")
                        .arg(work_key_arg())
                        .arg(work_navigation_arg()),
                )
                .subcommand(
                    ClapCommand::new("trunk")
                        .about("Print or enter the trunk checkout for the current workspace")
                        .arg(work_shell_cd_target_arg()),
                )
                .subcommand(
                    ClapCommand::new("delete")
                        .about("Delete a managed workspace")
                        .arg(workspace_name_arg().required(false))
                        .arg(work_shell_cd_target_arg()),
                ),
        )
        .subcommand(stack_command())
        .subcommand(
            ClapCommand::new("shell")
                .about("Generate shell integration scripts")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    ClapCommand::new("init")
                        .about("Print shell integration for eval")
                        .arg(shell_arg()),
                ),
        )
        .subcommand(
            ClapCommand::new("open")
                .visible_alias("o")
                .about("Open a configured repository or pull request in the browser")
                .arg(open_print_arg())
                .arg(open_repo_arg())
                .arg(repository_arg_definition().conflicts_with("repo"))
                .subcommand(
                    ClapCommand::new("pr")
                        .visible_alias("pull-request")
                        .about("Open a pull request for the current repository")
                        .arg(open_pr_selector_arg())
                        .arg(open_commit_arg())
                        .arg(open_print_arg()),
                )
                .subcommand(
                    ClapCommand::new("prs")
                        .visible_alias("pull-requests")
                        .about("Open pull requests for a configured repository")
                        .arg(open_all_pull_requests_arg())
                        .arg(open_print_arg())
                        .arg(open_repo_arg())
                        .arg(repository_arg_definition().conflicts_with("repo")),
                ),
        )
        .subcommand(ClapCommand::new("check").about("Check repository and PR readiness"))
        .subcommand(
            ClapCommand::new("review")
                .about("Show pull requests requesting your review")
                .long_about(
                    "Show open GitHub pull requests requesting review from the authenticated user.\n\nThe command fetches live GitHub review requests, groups them by repository, applies repo-specific status policy such as review-gate checks, and renders check status, review-request state, labels, and reviewer state using the same compact conventions as stack status.",
                )
                .arg(dashboard_interactive_arg())
                .arg(dashboard_refresh_seconds_arg())
                .arg(review_repo_filter_arg()),
        )
        .subcommand(
            ClapCommand::new("prev-commit")
                .visible_alias("prev")
                .about("Move to the previous commit and show the surrounding chain"),
        )
        .subcommand(
            ClapCommand::new("next-commit")
                .visible_alias("next")
                .about("Move to the next commit and show the surrounding chain"),
        )
        .subcommand(
            ClapCommand::new("remote-status")
                .visible_alias("rs")
                .about("Compare local remote trunks with GitHub")
                .arg(remote_status_all_arg())
                .arg(remote_status_repo_arg())
                .arg(remote_status_changed_arg())
                .arg(remote_status_jobs_arg())
                .arg(remote_status_format_arg())
                .arg(repository_arg_definition().conflicts_with("repo")),
        )
        .subcommand(
            ClapCommand::new("fetch")
                .visible_alias("f")
                .about("Fetch origin and rebase/repair the jj stack")
                .arg(fetch_all_arg())
                .arg(repository_arg_definition()),
        )
        .subcommand(
            ClapCommand::new("rebase-on-trunk")
                .visible_alias("rt")
                .about("Rebase jj source revisions onto origin trunk")
                .arg(source_arg()),
        )
        .subcommand(
            ClapCommand::new("push")
                .about("Push a selected jj change or tracked bookmark state")
                .arg(push_revision_arg())
                .arg(tracked_arg()),
        )
        .subcommand(
            ClapCommand::new("sync")
                .about("Fetch origin and push repository, stack, or selected bookmark state")
                .long_about(
                    "Fetch origin and push repository, stack, or selected bookmark state.\n\nBy default, sync tracked bookmarks in the current repository, including setup/bootstrap behavior and configured trunk advancement. Use -s/--stack to sync every bookmark in the current pull-request stack. Pass a jj revision or bookmark to sync one bookmarked target instead. Use -r/--repo to force repository mode explicitly. Use -a/--all to sync eligible primary repositories from configured layout roots without prompting; optional repository globs filter provider/owner/repo identities, so `solodov/*` matches `github.com/solodov/foo`.",
                )
                .arg(sync_all_arg())
                .arg(sync_repo_arg())
                .arg(sync_stack_arg())
                .arg(sync_experimental_skip_same_tree_push_arg())
                .arg(sync_revision_arg()),
        )
}

fn stack_command() -> ClapCommand {
    ClapCommand::new("stack")
        .visible_alias("stk")
        .about("Show, move, publish, or refresh repo-local pull request stack state")
        .long_about(
            "Show, move, publish, status-check, or refresh repo-local pull request stack state.\n\nStack state is stored in .jx/stack.toml so stack-aware commands can keep parent/child PR relationships even when a parent PR has merged or its local bookmark disappeared. Without a subcommand or move option, jx stack shows the stored local stack without contacting GitHub. Use status to fetch GitHub check and review summaries for the stored stack, or status -a with optional repository filters such as `example-owner/*` or `service-*` to scan configured repositories. Use plan to preview the local stack neighbourhood for the working copy or selected revsets. Use publish to create or update pull requests for a local stack; pass -r/--revision to publish exactly the selected revset. Use -o/--onto to move the current change and descendants onto a commit, change, or bookmark target, or -t/--trunk to move it onto trunk; stack moves sync affected PR branches by default unless --no-sync is set. Use refresh to rebuild metadata from local bookmarks and open GitHub PRs authored by you. Use -i/--interactive to choose a stored PR and open it.",
        )
        .args_conflicts_with_subcommands(true)
        .group(
            ArgGroup::new("stack-move-target")
                .args(["onto", "trunk"])
                .multiple(false),
        )
        .arg(stack_interactive_arg())
        .arg(stack_onto_arg())
        .arg(stack_trunk_arg())
        .arg(stack_no_sync_arg().requires("stack-move-target"))
        .arg(open_print_arg().requires("interactive"))
        .subcommand(
            ClapCommand::new("show")
                .about("Show stored pull request stack state")
                .long_about(
                    "Show stored pull request stack state from .jx/stack.toml without contacting GitHub.\n\nThis is the default when no stack subcommand is provided. It reports the last refreshed or PR-maintained stack snapshot.",
                ),
        )
        .subcommand(
            ClapCommand::new("refresh")
                .about("Rebuild stack state from local bookmarks and authored open PRs")
                .long_about(
                    "Rebuild repo-local stack state from local PR bookmarks and open GitHub pull requests authored by you.\n\nThe command searches open GitHub PRs authored by the authenticated login, also checks local PR bookmark heads for matching authored PRs, refreshes durable PR-number metadata for stored ancestors, applies local jj ancestry, writes .jx/stack.toml, syncs affected PR bases/descriptions, and prints the resulting stack. It does not push branches or create, close, or delete pull requests.",
                ),
        )
        .subcommand(
            ClapCommand::new("status")
                .about("Show trunk, check, and review status for pull request stacks")
                .long_about(
                    "Show origin trunk freshness plus GitHub check and review status for pull request stacks while keeping remote state unchanged.\n\nBy default, jx reads the current repository's stored .jx/stack.toml stack, fetches a batched GitHub status summary for its pull requests, checks whether the local origin trunk matches GitHub's branch head, refreshes cached PR state, removes closed PRs, and prunes fully merged cached stack trees. Use -a/--all to scan configured primary repositories that have stack metadata; optional positional filters match repository keys and provider/owner/repo identities, for example `example-owner/*` or `service-*`.",
                )
                .arg(stack_status_all_arg())
                .arg(stack_status_jobs_arg())
                .arg(stack_status_format_arg())
                .arg(dashboard_interactive_arg())
                .arg(dashboard_refresh_seconds_arg())
                .arg(stack_status_repo_filter_arg()),
        )
        .subcommand(
            ClapCommand::new("plan")
                .about("Preview the local stack neighbourhood for publishing")
                .long_about(
                    "Preview the local stack neighbourhood for publishing without contacting GitHub or mutating local state.\n\nWithout -r/--revision, jx plans the neighbourhood containing the working copy. With one or more -r/--revision revsets, jx shows the common-root neighbourhood and marks exactly the selected changes. Selected revisions must share one stack root.",
                )
                .arg(stack_plan_revision_arg()),
        )
        .subcommand(
            ClapCommand::new("publish")
                .visible_alias("pub")
                .about("Publish or update GitHub pull requests for a local stack")
                .long_about(
                    "Publish or update GitHub pull requests for a local stack.\n\nWithout -r/--revision, jx publishes every change in the linear stack containing the working copy. With one or more -r/--revision revsets, jx publishes exactly the selected changes, which must belong to one linear stack. A single selected revision reproduces the old one-PR workflow while preserving stack-aware base selection. Task IDs, labels, reviewers, fix intent, and bare --ready/--draft apply only to the current commit or single selected revision by default; pass -A/--apply-to-stack to apply publish intent to every published revision. Use --ready=REVSET / --draft=REVSET for explicit readiness subsets.",
                )
                .arg(stack_publish_revision_arg())
                .arg(task_id_arg())
                .arg(no_task_id_arg())
                .arg(label_arg())
                .arg(reviewer_arg())
                .arg(fixes_arg())
                .arg(ready_arg())
                .arg(draft_arg())
                .arg(apply_to_stack_arg())
                .arg(no_event_handlers_arg()),
        )
        .subcommand(
            ClapCommand::new("complete-reviewers")
                .hide(true)
                .arg(reviewer_completion_prefix_arg()),
        )
}

fn yes_arg() -> Arg {
    Arg::new("yes")
        .short('y')
        .long("yes")
        .global(true)
        .action(ArgAction::SetTrue)
        .help("Answer yes to confirmation prompts")
}

fn clone_repository_arg() -> Arg {
    Arg::new("repository")
        .value_name("REPOSITORY")
        .required(true)
        .help("Repository shorthand, repo from a layout prefix, or URL to clone")
}

fn clone_destination_arg() -> Arg {
    Arg::new("destination")
        .value_name("DESTINATION")
        .value_parser(clap::value_parser!(PathBuf))
        .help("Override the configured layout destination")
}

fn workspace_name_arg() -> Arg {
    Arg::new("name")
        .value_name("NAME")
        .required(true)
        .help("Workspace name to manage")
}

fn work_shell_cd_target_arg() -> Arg {
    Arg::new("shell-cd-target")
        .long("shell-cd-target")
        .hide(true)
        .action(ArgAction::SetTrue)
}

fn workspace_revision_arg() -> Arg {
    Arg::new("revision")
        .short('r')
        .long("revision")
        .value_name("REVISION")
        .help("Create the workspace working-copy change on the selected revision")
}

fn work_all_arg() -> Arg {
    Arg::new("all")
        .long("all")
        .action(ArgAction::SetTrue)
        .help("List global work locations from configured layout roots")
}

fn work_prefix_arg() -> Arg {
    Arg::new("prefix")
        .long("prefix")
        .value_name("PREFIX")
        .help("Filter global work-location keys by prefix")
}

fn work_repositories_arg() -> Arg {
    Arg::new("repositories")
        .long("repositories")
        .action(ArgAction::SetTrue)
        .conflicts_with("workspaces")
        .help("Complete only primary repository keys")
}

fn workspaces_arg() -> Arg {
    Arg::new("workspaces")
        .long("workspaces")
        .hide(true)
        .action(ArgAction::SetTrue)
        .conflicts_with("repositories")
}

fn work_navigation_arg() -> Arg {
    Arg::new("navigation")
        .long("navigation")
        .hide(true)
        .action(ArgAction::SetTrue)
}

fn work_complete_format_arg() -> Arg {
    Arg::new("format")
        .long("format")
        .value_name("FORMAT")
        .default_value("simple")
        .value_parser(["simple", "picker"])
        .hide(true)
}

fn work_key_arg() -> Arg {
    Arg::new("key")
        .value_name("KEY")
        .required(true)
        .help("Work-location key such as `repo` or `repo@workspace`")
}

fn shell_arg() -> Arg {
    Arg::new("shell")
        .value_name("SHELL")
        .required(true)
        .help("Shell to initialize; currently supports `bash`")
}

fn task_id_arg() -> Arg {
    Arg::new("task-id")
        .short('t')
        .long("task-id")
        .value_name("TASK_ID")
        .help("Associate a task identifier with generated workspace or PR bookmark names")
}

fn no_task_id_arg() -> Arg {
    Arg::new("no-task-id")
        .long("no-task-id")
        .action(ArgAction::SetTrue)
        .conflicts_with("task-id")
        .help("Ignore workspace metadata task IDs when generating a pull request bookmark")
}

fn stack_publish_revision_arg() -> Arg {
    Arg::new("revision")
        .short('r')
        .long("revision")
        .value_name("REVSET")
        .action(ArgAction::Append)
        .help("Publish exactly the selected jj revset; repeat for multiple revsets")
}

fn stack_plan_revision_arg() -> Arg {
    Arg::new("revision")
        .short('r')
        .long("revision")
        .value_name("REVSET")
        .action(ArgAction::Append)
        .help("Plan exactly the selected jj revset; repeat for multiple revsets")
}

fn open_commit_arg() -> Arg {
    Arg::new("commit")
        .short('c')
        .long("commit")
        .value_name("COMMIT_OR_BOOKMARK")
        .conflicts_with("selector")
        .help("Open the pull request for a specific jj revision or local bookmark")
}

fn open_pr_selector_arg() -> Arg {
    Arg::new("selector")
        .value_name("COMMIT_OR_BOOKMARK")
        .conflicts_with("commit")
        .help("Open the pull request for a specific jj revision or local bookmark")
}

fn stack_interactive_arg() -> Arg {
    Arg::new("interactive")
        .short('i')
        .long("interactive")
        .action(ArgAction::SetTrue)
        .conflicts_with_all(["onto", "trunk", "no-sync"])
        .help("Select a stored pull request from the stack and open it")
}

fn stack_onto_arg() -> Arg {
    Arg::new("onto")
        .short('o')
        .long("onto")
        .value_name("COMMIT_OR_BOOKMARK")
        .conflicts_with_all(["interactive", "trunk"])
        .help("Move the current change and descendants onto a commit, change, or bookmark target, then sync")
}

fn stack_trunk_arg() -> Arg {
    Arg::new("trunk")
        .short('t')
        .long("trunk")
        .action(ArgAction::SetTrue)
        .conflicts_with_all(["interactive", "onto"])
        .help("Move the current change and descendants onto trunk, then sync")
}

fn stack_no_sync_arg() -> Arg {
    Arg::new("no-sync")
        .long("no-sync")
        .action(ArgAction::SetTrue)
        .help("Update local stack state without pushing branches or updating GitHub PRs")
}

fn stack_status_all_arg() -> Arg {
    Arg::new("all")
        .short('a')
        .long("all")
        .action(ArgAction::SetTrue)
        .help("Check pull request stack status across configured primary repositories")
}

fn stack_status_jobs_arg() -> Arg {
    Arg::new("jobs")
        .short('j')
        .long("jobs")
        .value_name("N")
        .default_value("8")
        .value_parser(clap::value_parser!(usize))
        .help("Limit concurrent GitHub status checks for --all")
}

fn stack_status_format_arg() -> Arg {
    Arg::new("format")
        .long("format")
        .value_name("FORMAT")
        .default_value("human")
        .value_parser(["human", "json"])
        .help("Select stack status output format")
}

fn stack_status_repo_filter_arg() -> Arg {
    Arg::new("repository-filter")
        .value_name("REPO_GLOB")
        .num_args(0..)
        .requires("all")
        .help("Filter provider/owner/repo identities when --all is set")
}

fn dashboard_interactive_arg() -> Arg {
    Arg::new("interactive")
        .short('i')
        .long("interactive")
        .action(ArgAction::SetTrue)
        .help("Continuously refresh this dashboard until interrupted")
}

fn dashboard_refresh_seconds_arg() -> Arg {
    Arg::new("refresh-seconds")
        .long("refresh-seconds")
        .value_name("SECONDS")
        .default_value("300")
        .value_parser(clap::value_parser!(u64).range(1..))
        .help("Seconds between interactive dashboard refreshes")
}

fn dashboard_refresh_seconds(matches: &ArgMatches) -> Result<u64, clap::Error> {
    matches
        .get_one::<u64>("refresh-seconds")
        .copied()
        .ok_or_else(|| clap::Error::raw(ErrorKind::InvalidValue, "refresh interval is required"))
}

fn review_repo_filter_arg() -> Arg {
    Arg::new("repository-filter")
        .value_name("REPO_GLOB")
        .num_args(0..)
        .help("Filter review requests by configured key or provider/owner/repo glob")
}

fn source_arg() -> Arg {
    Arg::new("source")
        .short('s')
        .long("source")
        .value_name("COMMIT")
        .action(ArgAction::Append)
        .help("Rebase a specific jj revision and its descendants; repeat for multiple sources")
}

fn remote_status_all_arg() -> Arg {
    Arg::new("all")
        .short('a')
        .long("all")
        .action(ArgAction::SetTrue)
        .conflicts_with("repository")
        .help("Check every primary repository in configured layout roots")
}

fn fetch_all_arg() -> Arg {
    Arg::new("all")
        .short('a')
        .long("all")
        .action(ArgAction::SetTrue)
        .conflicts_with("repository")
        .help("Fetch every safe primary repository in configured layout roots")
}

fn sync_all_arg() -> Arg {
    Arg::new("all")
        .short('a')
        .long("all")
        .action(ArgAction::SetTrue)
        .conflicts_with_all(["repo", "stack"])
        .help("Sync every eligible primary repository in configured layout roots, optionally filtered by provider/owner/repo globs")
}

fn sync_repo_arg() -> Arg {
    Arg::new("repo")
        .short('r')
        .long("repo")
        .action(ArgAction::SetTrue)
        .conflicts_with_all(["all", "stack", "target"])
        .help("Sync all tracked bookmarks in the current repository")
}

fn sync_stack_arg() -> Arg {
    Arg::new("stack")
        .short('s')
        .long("stack")
        .action(ArgAction::SetTrue)
        .conflicts_with_all(["all", "repo", "target"])
        .help("Sync every bookmark in the current pull-request stack")
}

fn sync_experimental_skip_same_tree_push_arg() -> Arg {
    Arg::new("experimental-skip-same-tree-push")
        .long("experimental-skip-same-tree-push")
        .action(ArgAction::SetTrue)
        .help(
            "Experimentally preserve same-code GitHub PR heads instead of pushing local commit ids",
        )
}

fn sync_revision_arg() -> Arg {
    Arg::new("target")
        .value_name("COMMIT_OR_BOOKMARK_OR_REPO_GLOB")
        .num_args(0..)
        .conflicts_with_all(["repo", "stack"])
        .help("Sync one bookmarked jj revision, or filter provider/owner/repo identities when --all is set")
}

fn repository_arg_definition() -> Arg {
    Arg::new("repository")
        .value_name("REPOSITORY")
        .help("Run against a configured primary repository key")
}

fn remote_status_repo_arg() -> Arg {
    Arg::new("repo")
        .long("repo")
        .value_name("GLOB")
        .action(ArgAction::Append)
        .help("Check matching configured repository keys; repeat for multiple filters")
}

fn open_repo_arg() -> Arg {
    Arg::new("repo")
        .long("repo")
        .value_name("GLOB")
        .action(ArgAction::Append)
        .help("Open matching configured repository keys; repeat for multiple filters")
}

fn open_print_arg() -> Arg {
    Arg::new("print")
        .long("print")
        .action(ArgAction::SetTrue)
        .help("Print resolved URLs instead of opening the browser")
}

fn open_all_pull_requests_arg() -> Arg {
    Arg::new("all")
        .short('a')
        .long("all")
        .action(ArgAction::SetTrue)
        .help("Show all open pull requests instead of only pull requests authored by you")
}

fn remote_status_changed_arg() -> Arg {
    Arg::new("changed")
        .short('c')
        .long("changed")
        .action(ArgAction::SetTrue)
        .help("Show only repositories with remote or local changes")
}

fn remote_status_jobs_arg() -> Arg {
    Arg::new("jobs")
        .short('j')
        .long("jobs")
        .value_name("N")
        .default_value("8")
        .value_parser(clap::value_parser!(usize))
        .help("Limit concurrent GitHub checks for global remote status")
}

fn remote_status_format_arg() -> Arg {
    Arg::new("format")
        .long("format")
        .value_name("FORMAT")
        .default_value("human")
        .value_parser(["human", "json"])
        .help("Select remote-status output format")
}

fn push_revision_arg() -> Arg {
    Arg::new("revision")
        .short('r')
        .long("revision")
        .value_name("COMMIT_OR_BOOKMARK")
        .conflicts_with("tracked")
        .help("Push a specific jj revision or local bookmark instead of the working copy")
}

fn diff_revision_arg() -> Arg {
    Arg::new("revision")
        .short('r')
        .long("revision")
        .value_name("COMMIT_OR_BOOKMARK")
        .help("Show the diff for a specific jj revision or local bookmark instead of the working copy")
}

fn tracked_arg() -> Arg {
    Arg::new("tracked")
        .long("tracked")
        .action(ArgAction::SetTrue)
        .help("Push all tracked origin bookmarks, including deleted bookmarks")
}

fn no_tests_arg() -> Arg {
    Arg::new("no-tests")
        .short('n')
        .long("no-tests")
        .action(ArgAction::SetTrue)
        .help("Exclude test files from the selected diff")
}

fn diff_tool_arg() -> Arg {
    Arg::new("tool")
        .long("tool")
        .value_name("TOOL")
        .help("Use a configured jx diff tool")
}

fn diff_path_arg() -> Arg {
    Arg::new("path")
        .value_name("PATH")
        .num_args(0..)
        .help("Limit the diff to one or more file paths")
}

fn diff_tool_args_arg() -> Arg {
    Arg::new("tool-args")
        .value_name("TOOL_ARG")
        .num_args(0..)
        .last(true)
        .allow_hyphen_values(true)
        .help("Append arguments to the selected diff tool after `--`")
}

fn label_arg() -> Arg {
    Arg::new("label")
        .short('l')
        .long("label")
        .value_name("LABEL")
        .action(ArgAction::Append)
        .help("Apply a label to the current or single selected pull request; repeat for multiple labels")
}

fn reviewer_arg() -> Arg {
    Arg::new("reviewer")
        .short('R')
        .long("reviewer")
        .value_name("REVIEWER")
        .action(ArgAction::Append)
        .help("Request a GitHub user or org/team reviewer for the current or single selected pull request; repeat for multiple reviewers")
}

fn fixes_arg() -> Arg {
    Arg::new("fixes")
        .short('F')
        .long("fixes")
        .value_name("WORK_ID")
        .num_args(0..=1)
        .default_missing_value("")
        .action(ArgAction::Append)
        .help("Record that the current or single selected pull request fixes WORK_ID; omit WORK_ID to fix the attached work ID")
}

fn reviewer_completion_prefix_arg() -> Arg {
    Arg::new("prefix")
        .long("prefix")
        .value_name("PREFIX")
        .hide(true)
        .help("Filter configured reviewer names by prefix")
}

fn ready_arg() -> Arg {
    Arg::new("ready")
        .long("ready")
        .value_name("REVSET")
        .num_args(0..=1)
        .require_equals(true)
        .default_missing_value("")
        .action(ArgAction::Append)
        .help("Mark the current or single selected pull request, or only REVSET, ready for review")
}

fn draft_arg() -> Arg {
    Arg::new("draft")
        .short('d')
        .long("draft")
        .value_name("REVSET")
        .num_args(0..=1)
        .require_equals(true)
        .default_missing_value("")
        .action(ArgAction::Append)
        .help("Mark the current or single selected pull request, or only REVSET, as draft")
}

fn apply_to_stack_arg() -> Arg {
    Arg::new("apply-to-stack")
        .short('A')
        .long("apply-to-stack")
        .action(ArgAction::SetTrue)
        .help("Apply task IDs, labels, reviewers, and bare readiness intent to every published pull request")
}

fn no_event_handlers_arg() -> Arg {
    Arg::new("no-event-handlers")
        .long("no-event-handlers")
        .action(ArgAction::SetTrue)
        .help("Disable configured repository event handlers for this pull request")
}
