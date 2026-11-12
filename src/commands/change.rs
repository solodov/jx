//! Local revision-scoped commands. JJ owns checkout isolation and history rewriting.

use super::*;
use crate::jj::ChangeBatch;
use serde_json::{json, Value};

mod help;
pub(super) use help::change_command;

pub(super) struct ChangeRequest {
    action: ChangeAction,
    json: bool,
}

enum ChangeAction {
    New {
        from: String,
        message: String,
    },
    Read {
        revision: String,
        path: String,
    },
    Search {
        revision: String,
        pattern: String,
    },
    Apply {
        revision: String,
        expected: String,
    },
    Run {
        revision: String,
        command: Vec<String>,
    },
}

impl ChangeRequest {
    pub(super) fn from_matches(matches: &ArgMatches) -> Self {
        let json = matches.get_flag("json");
        let (name, args) = matches
            .subcommand()
            .expect("clap requires a change subcommand");
        let value = |key| {
            args.get_one::<String>(key)
                .expect("clap requires argument")
                .clone()
        };
        let action = match name {
            "new" => ChangeAction::New {
                from: value("from"),
                message: value("message"),
            },
            "read" => ChangeAction::Read {
                revision: value("revision"),
                path: value("path"),
            },
            "search" => ChangeAction::Search {
                revision: value("revision"),
                pattern: value("pattern"),
            },
            "apply" => ChangeAction::Apply {
                revision: value("revision"),
                expected: value("expect"),
            },
            "run" => ChangeAction::Run {
                revision: value("revision"),
                command: args
                    .get_many::<String>("command")
                    .expect("command required")
                    .cloned()
                    .collect(),
            },
            _ => unreachable!("clap validates change subcommands"),
        };
        Self { action, json }
    }

    pub(super) fn command_path(&self) -> &'static str {
        match self.action {
            ChangeAction::New { .. } => "change.new",
            ChangeAction::Read { .. } => "change.read",
            ChangeAction::Search { .. } => "change.search",
            ChangeAction::Apply { .. } => "change.apply",
            ChangeAction::Run { .. } => "change.run",
        }
    }
}

/// Dispatches local change operations without requiring a GitHub repository context.
pub(super) fn handle_change(
    request: ChangeRequest,
    environment: &RuntimeEnvironment,
) -> Result<CommandResult, CommandError> {
    let result = execute_change(request.action, environment, request.json);
    match result {
        Ok(result) => Ok(result),
        Err(error) if request.json => Ok(CommandResult::with_exit_code(
            format!("{}\n", json!({ "error": change_error_value(&error) })),
            1,
        )),
        Err(error) => Err(error.into()),
    }
}

fn execute_change(
    action: ChangeAction,
    environment: &RuntimeEnvironment,
    json_output: bool,
) -> Result<CommandResult, JjError> {
    let root = environment.current_dir();
    let result = match action {
        ChangeAction::New { from, message } => {
            let result = JjWorkspace::new_change(root, &from, &message, check_jj_interrupted)?;
            return render_change_result(result, json_output);
        }
        ChangeAction::Apply { revision, expected } => {
            // Read the request before taking the lock, so a slow pipe cannot block writers.
            let batch: ChangeBatch =
                serde_json::from_reader(io::stdin().lock()).map_err(change_input_error)?;
            let result = JjWorkspace::apply_change(
                root,
                &revision,
                &expected,
                &batch,
                check_jj_interrupted,
            )?;
            return render_change_result(result, json_output);
        }
        ChangeAction::Read { revision, path } => {
            let result =
                JjWorkspace::load_from_directory(root)?.read_change_file(&revision, &path)?;
            if json_output {
                json_result(&result)?
            } else {
                result.content
            }
        }
        ChangeAction::Search { revision, pattern } => {
            let (snapshot, output) =
                JjWorkspace::load_from_directory(root)?.search_change(&revision, &pattern)?;
            if json_output {
                json_result(&json!({ "snapshot": snapshot, "output": output }))?
            } else {
                output
            }
        }
        ChangeAction::Run { revision, command } => {
            let (snapshot, output) =
                JjWorkspace::load_from_directory(root)?.run_change_command(&revision, &command)?;
            let exit_code = output
                .status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .unwrap_or(1);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let rendered = if json_output {
                json_result(
                    &json!({ "snapshot": snapshot, "stdout": stdout, "stderr": stderr, "exit_code": exit_code }),
                )?
            } else {
                eprint!("{stderr}");
                stdout.into_owned()
            };
            return Ok(CommandResult::with_exit_code(rendered, exit_code));
        }
    };
    Ok(CommandResult::success(result))
}

fn render_change_result(
    result: crate::jj::ChangeResult,
    json_output: bool,
) -> Result<CommandResult, JjError> {
    let stdout = if json_output {
        json_result(&result)?
    } else {
        let mut output = format!(
            "{} {}{}\n",
            result.snapshot.change_id,
            result.snapshot.commit_id,
            if result.changed { "" } else { " (unchanged)" }
        );
        if !result.conflicted_changes.is_empty() {
            output.push_str(&format!(
                "Conflicted changes: {}\n",
                result.conflicted_changes.join(", ")
            ));
        }
        output
    };
    Ok(CommandResult::success(stdout))
}

fn json_result(value: &impl serde::Serialize) -> Result<String, JjError> {
    serde_json::to_string(value)
        .map(|text| format!("{text}\n"))
        .map_err(change_input_error)
}

fn change_error_value(error: &JjError) -> Value {
    let code = match error {
        JjError::StaleChange { .. } => "stale_change",
        JjError::MutationBusy { .. } => "repository_busy",
        JjError::ConcurrentOperations => "concurrent_operations",
        JjError::DivergentChange { .. } => "divergent_change",
        JjError::AbandonedChange { .. } => "abandoned_change",
        JjError::CheckedOutChanges { .. } => "checked_out_changes",
        JjError::Interrupted => "interrupted",
        JjError::ChangeRequest { .. } => "invalid_request",
        JjError::ChangeCommand { .. } => "jj_failed",
        _ => "repository_error",
    };
    let mut value = json!({ "code": code, "message": error.to_string() });
    if let JjError::StaleChange { expected, actual } = error {
        value["expected"] = json!(expected);
        value["actual"] = json!(actual);
    }
    if let JjError::CheckedOutChanges { workspaces } = error {
        value["workspaces"] = json!(workspaces);
    }
    value
}

fn change_input_error(error: impl ToString) -> JjError {
    JjError::ChangeRequest {
        message: error.to_string(),
    }
}
