use super::*;

/// Parsed operator intent after clap has validated subcommand-specific flags.
pub(super) enum CommandRequest {
    Log,
    Status,
    Diff(DiffRequest),
    Clone(CloneRequest),
    Work(WorkRequest),
    Shell(ShellRequest),
    RemoteStatus(RemoteStatusRequest),
    RebaseOnTrunk(RebaseOnTrunkRequest),
    Push(PushRequest),
    Sync,
    Workflow {
        command: WorkflowCommand,
        task_id: Option<String>,
        commit: Option<String>,
        labels: Vec<String>,
        reviewers: Vec<ReviewerTarget>,
        draft: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DiffRequest {
    pub(super) revision: Option<String>,
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
    Remove(WorkRemoveRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkAddRequest {
    pub(super) name: String,
    pub(super) revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkListRequest {
    pub(super) all: bool,
    pub(super) prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkCompleteRequest {
    pub(super) prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkRootRequest {
    pub(super) key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkRemoveRequest {
    pub(super) name: String,
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
pub(super) struct RemoteStatusRequest {
    pub(super) all: bool,
    pub(super) repo_filters: Vec<String>,
    pub(super) changed: bool,
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
                no_tests: matches.get_flag("no-tests"),
                tool: diff_tool(matches)?,
                tool_args: diff_tool_args(matches),
            })),
            Some(("clone", matches)) => Ok(Self::Clone(CloneRequest {
                repository: required_arg(matches, "repository"),
                destination: matches.get_one::<PathBuf>("destination").cloned(),
            })),
            Some(("work", matches)) => Ok(Self::Work(work_request(matches)?)),
            Some(("shell", matches)) => Ok(Self::Shell(shell_request(matches)?)),
            Some(("status" | "st", _)) => Ok(Self::Status),
            Some(("check", _)) => Ok(Self::Workflow {
                command: WorkflowCommand::Check,
                task_id: None,
                commit: None,
                labels: Vec::new(),
                reviewers: Vec::new(),
                draft: false,
            }),
            Some(("remote-status" | "rs", matches)) => {
                Ok(Self::RemoteStatus(RemoteStatusRequest {
                    all: matches.get_flag("all"),
                    repo_filters: repo_filters(matches),
                    changed: matches.get_flag("changed"),
                }))
            }
            Some(("fetch" | "f", _)) => Ok(Self::Workflow {
                command: WorkflowCommand::Fetch,
                task_id: None,
                commit: None,
                labels: Vec::new(),
                reviewers: Vec::new(),
                draft: false,
            }),
            Some(("rebase-on-trunk" | "rt", matches)) => {
                Ok(Self::RebaseOnTrunk(RebaseOnTrunkRequest {
                    sources: sources(matches),
                }))
            }
            Some(("push", matches)) => Ok(Self::Push(PushRequest {
                revision: revision(matches),
                tracked: matches.get_flag("tracked"),
            })),
            Some(("sync", _)) => Ok(Self::Sync),
            Some(("pull-request" | "pr", matches)) => Ok(Self::Workflow {
                command: WorkflowCommand::PullRequest,
                task_id: task_id(matches),
                commit: commit(matches),
                labels: labels(matches),
                reviewers: reviewers(matches)?,
                draft: matches.get_flag("draft"),
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
        })),
        Some(("list", matches)) => Ok(WorkRequest::List(WorkListRequest {
            all: matches.get_flag("all"),
            prefix: string_arg(matches, "prefix").unwrap_or_default(),
        })),
        Some(("complete", matches)) => Ok(WorkRequest::Complete(WorkCompleteRequest {
            prefix: string_arg(matches, "prefix").unwrap_or_default(),
        })),
        Some(("root", matches)) => Ok(WorkRequest::Root(WorkRootRequest {
            key: required_arg(matches, "key"),
        })),
        Some(("remove", matches)) => Ok(WorkRequest::Remove(WorkRemoveRequest {
            name: required_arg(matches, "name"),
        })),
        _ => Err(clap::Error::raw(
            ErrorKind::MissingSubcommand,
            "`jx work` requires `add`, `list`, `complete`, `root`, or `remove`",
        )),
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

fn sources(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>("source")
        .into_iter()
        .flatten()
        .cloned()
        .collect()
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
                        .arg(workspace_revision_arg()),
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
                        .arg(work_prefix_arg()),
                )
                .subcommand(
                    ClapCommand::new("root")
                        .about("Resolve a global work-location key to its root path")
                        .arg(work_key_arg()),
                )
                .subcommand(
                    ClapCommand::new("remove")
                        .about("Forget and delete a managed workspace")
                        .arg(workspace_name_arg()),
                ),
        )
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
        .subcommand(ClapCommand::new("check").about("Check repository and PR readiness"))
        .subcommand(
            ClapCommand::new("remote-status")
                .visible_alias("rs")
                .about("Compare local remote trunks with GitHub")
                .arg(remote_status_all_arg())
                .arg(remote_status_repo_arg())
                .arg(remote_status_changed_arg()),
        )
        .subcommand(
            ClapCommand::new("fetch")
                .visible_alias("f")
                .about("Fetch origin and rebase/repair the jj stack"),
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
            ClapCommand::new("sync").about(
                "Fetch origin, or initialize/create the configured repository, then push bookmark state",
            ),
        )
        .subcommand(
            ClapCommand::new("pull-request")
                .visible_alias("pr")
                .about("Publish or update a GitHub pull request for a jj change")
                .arg(task_id_arg())
                .arg(commit_arg())
                .arg(label_arg())
                .arg(reviewer_arg())
                .arg(draft_arg()),
        )
}

fn clone_repository_arg() -> Arg {
    Arg::new("repository")
        .value_name("REPOSITORY")
        .required(true)
        .help("Repository shorthand or URL to clone")
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
        .help("Include a task identifier in generated PR bookmark names")
}

fn commit_arg() -> Arg {
    Arg::new("commit")
        .short('c')
        .long("commit")
        .value_name("COMMIT")
        .help("Publish a specific jj revision instead of the working copy")
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
        .help("Check every primary repository in configured layout roots")
}

fn remote_status_repo_arg() -> Arg {
    Arg::new("repo")
        .long("repo")
        .value_name("GLOB")
        .action(ArgAction::Append)
        .help("Check matching configured repository keys; repeat for multiple filters")
}

fn remote_status_changed_arg() -> Arg {
    Arg::new("changed")
        .short('c')
        .long("changed")
        .action(ArgAction::SetTrue)
        .help("Show only repositories with remote or local changes")
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
