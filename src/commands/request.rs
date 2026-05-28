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
    RemoteStatus(RemoteStatusRequest),
    Fetch(FetchRequest),
    RebaseOnTrunk(RebaseOnTrunkRequest),
    Push(PushRequest),
    Sync(SyncRequest),
    Workflow {
        command: WorkflowCommand,
        task_id: Option<String>,
        no_task_id: bool,
        commit: Option<String>,
        labels: Vec<String>,
        reviewers: Vec<ReviewerTarget>,
        draft: bool,
        no_event_handlers: bool,
    },
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
            Some(("status" | "st", _)) => Ok(Self::Status),
            Some(("prev-commit" | "prev", _)) => Ok(Self::PreviousCommit),
            Some(("next-commit" | "next", _)) => Ok(Self::NextCommit),
            Some(("check", _)) => Ok(Self::Workflow {
                command: WorkflowCommand::Check,
                task_id: None,
                no_task_id: false,
                commit: None,
                labels: Vec::new(),
                reviewers: Vec::new(),
                draft: false,
                no_event_handlers: false,
            }),
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
            Some(("pull-request" | "pr", matches)) => Ok(Self::Workflow {
                command: WorkflowCommand::PullRequest,
                task_id: task_id(matches),
                no_task_id: matches.get_flag("no-task-id"),
                commit: commit(matches),
                labels: labels(matches),
                reviewers: reviewers(matches)?,
                draft: matches.get_flag("draft"),
                no_event_handlers: matches.get_flag("no-event-handlers"),
            }),
            None => Ok(Self::Log),
            _ => unreachable!("clap rejects unknown subcommands"),
        }
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
        _ => Ok(StackRequest::Show),
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

fn commit(matches: &ArgMatches) -> Option<String> {
    matches.get_one::<String>("commit").cloned()
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

pub(super) fn cli() -> ClapCommand {
    ClapCommand::new("jx")
        .about("Small, opinionated extensions for everyday jj workflows")
        .arg_required_else_help(false)
        .subcommand_required(false)
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
                        .arg(work_navigation_arg().conflicts_with_all(["repositories", "workspaces"])),
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
                .arg(sync_revision_arg()),
        )
        .subcommand(
            ClapCommand::new("pull-request")
                .visible_alias("pr")
                .about("Publish or update a GitHub pull request for a jj change")
                .arg(task_id_arg())
                .arg(no_task_id_arg())
                .arg(commit_arg())
                .arg(label_arg())
                .arg(reviewer_arg())
                .arg(draft_arg())
                .arg(no_event_handlers_arg()),
        )
}

fn stack_command() -> ClapCommand {
    ClapCommand::new("stack")
        .visible_alias("stk")
        .about("Show, move, or refresh repo-local pull request stack state")
        .long_about(
            "Show, move, or refresh repo-local pull request stack state.\n\nStack state is stored in .jx/stack.toml so stack-aware commands can keep parent/child PR relationships even when a parent PR has merged or its local bookmark disappeared. Without a subcommand or move option, jx stack shows the stored local stack without contacting GitHub. Use -o/--onto to move the current change and descendants onto a commit, change, or bookmark target, or -t/--trunk to move it onto trunk; stack moves sync affected PR branches by default unless --no-sync is set. Use refresh to rebuild metadata from local bookmarks and open GitHub PRs authored by you. Use -i/--interactive to choose a stored PR and open it.",
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
                    "Rebuild repo-local stack state from local PR bookmarks and open GitHub pull requests authored by you.\n\nThe command reads local PR bookmark heads, looks up matching open GitHub PRs for the authenticated login, refreshes durable PR-number metadata for stored ancestors, applies local jj ancestry, writes .jx/stack.toml, syncs affected PR bases/descriptions, and prints the resulting stack. It does not push branches or create, close, or delete pull requests.",
                ),
        )
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

fn commit_arg() -> Arg {
    Arg::new("commit")
        .short('c')
        .long("commit")
        .value_name("COMMIT")
        .help("Publish a specific jj revision instead of the working copy")
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
        .help("Apply a label to the pull request; repeat for multiple labels")
}

fn reviewer_arg() -> Arg {
    Arg::new("reviewer")
        .short('r')
        .long("reviewer")
        .value_name("REVIEWER")
        .action(ArgAction::Append)
        .help("Request a GitHub user or org/team reviewer; repeat for multiple reviewers")
}

fn draft_arg() -> Arg {
    Arg::new("draft")
        .short('d')
        .long("draft")
        .action(ArgAction::SetTrue)
        .help("Create a draft pull request when no open pull request exists")
}

fn no_event_handlers_arg() -> Arg {
    Arg::new("no-event-handlers")
        .long("no-event-handlers")
        .action(ArgAction::SetTrue)
        .help("Disable configured repository event handlers for this pull request")
}
