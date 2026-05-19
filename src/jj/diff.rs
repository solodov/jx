use super::*;

/// Options for rendering a jj diff with optional revision and file path filters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffOptions {
    pub revision: Option<String>,
    pub paths: Vec<String>,
    pub no_tests: bool,
    pub tool: DiffToolInvocation,
}

/// Diff renderer selected for this invocation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DiffToolInvocation {
    #[default]
    Plain,
    External(ExternalDiffTool),
    Pipe(PipeDiffTool),
}

/// External diff command invoked by jj with generated left/right trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDiffTool {
    pub command: String,
    pub args: Vec<String>,
}

/// Renderer command that consumes a jj-produced diff stream on stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeDiffTool {
    pub producer_args: Vec<String>,
    pub command: String,
    pub args: Vec<String>,
}

/// Clones a Git repository through jj while keeping operator-facing output owned by `jx`.
pub fn run_jj_git_clone(
    current_dir: &Path,
    remote_url: &str,
    destination: &Path,
) -> Result<(), JjError> {
    let status = Command::new("jj")
        .arg("--no-pager")
        .arg("--quiet")
        .arg("git")
        .arg("clone")
        .arg(remote_url)
        .arg(destination)
        .current_dir(current_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| JjError::CloneStart { source })?;

    if status.success() {
        Ok(())
    } else {
        Err(JjError::CloneFailed {
            status: exit_status_summary(status),
        })
    }
}

/// Runs the selected diff using the chosen renderer and optional test exclusion.
pub fn run_current_diff(current_dir: &Path, options: &DiffOptions) -> Result<(), JjError> {
    let diff_files = if options.no_tests {
        let changed_files =
            current_diff_file_paths(current_dir, options.revision.as_deref(), &options.paths)?;
        let diff_files = diff_paths_without_tests(&changed_files);
        if diff_files.is_empty() {
            return Ok(());
        }
        diff_files
    } else {
        options.paths.clone()
    };

    run_jj_diff(
        current_dir,
        options.revision.as_deref(),
        &options.tool,
        &diff_files,
    )
}

fn current_diff_file_paths(
    current_dir: &Path,
    revision: Option<&str>,
    files: &[String],
) -> Result<Vec<String>, JjError> {
    let mut command = Command::new("jj");
    command.arg("--no-pager").arg("diff").arg("--name-only");
    add_revision_arg(&mut command, revision);
    add_file_args(&mut command, files);
    let output = command
        .current_dir(current_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .and_then(|child| child.wait_with_output())
        .map_err(|source| JjError::DiffStart {
            command: "jj diff --name-only".to_owned(),
            source,
        })?;

    if !output.status.success() {
        return Err(JjError::DiffFailed {
            command: "jj diff --name-only".to_owned(),
            status: exit_status_summary(output.status),
        });
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|source| JjError::DiffPathDecode { source })?;
    Ok(stdout
        .lines()
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

fn run_jj_diff(
    current_dir: &Path,
    revision: Option<&str>,
    tool: &DiffToolInvocation,
    files: &[String],
) -> Result<(), JjError> {
    match tool {
        DiffToolInvocation::Plain => run_plain_jj_diff(current_dir, revision, files),
        DiffToolInvocation::External(tool) => {
            run_external_jj_diff(current_dir, revision, tool, files)
        }
        DiffToolInvocation::Pipe(tool) => run_piped_jj_diff(current_dir, revision, tool, files),
    }
}

fn run_plain_jj_diff(
    current_dir: &Path,
    revision: Option<&str>,
    files: &[String],
) -> Result<(), JjError> {
    let mut command = Command::new("jj");
    command.arg("diff");
    add_revision_arg(&mut command, revision);
    add_file_args(&mut command, files);
    run_inherited_command(command, current_dir, "jj diff")
}

fn run_external_jj_diff(
    current_dir: &Path,
    revision: Option<&str>,
    tool: &ExternalDiffTool,
    files: &[String],
) -> Result<(), JjError> {
    const JX_DIFF_TOOL_NAME: &str = "jx-diff-tool";

    let diff_args = external_diff_args(tool);

    let mut command = Command::new("jj");
    command
        .arg("--config")
        .arg(toml_config_assignment(
            &format!("merge-tools.{JX_DIFF_TOOL_NAME}.program"),
            toml::Value::String(tool.command.clone()),
        ))
        .arg("--config")
        .arg(toml_config_assignment(
            &format!("merge-tools.{JX_DIFF_TOOL_NAME}.diff-args"),
            toml_string_array(&diff_args),
        ))
        .arg("diff")
        .arg("--tool")
        .arg(JX_DIFF_TOOL_NAME);
    add_revision_arg(&mut command, revision);
    add_file_args(&mut command, files);

    run_inherited_command(command, current_dir, "jj diff")
}

pub(super) fn external_diff_args(tool: &ExternalDiffTool) -> Vec<String> {
    let mut diff_args = tool.args.clone();
    diff_args.extend(["$left".to_owned(), "$right".to_owned()]);
    diff_args
}

fn run_piped_jj_diff(
    current_dir: &Path,
    revision: Option<&str>,
    tool: &PipeDiffTool,
    files: &[String],
) -> Result<(), JjError> {
    let mut producer_command = Command::new("jj");
    producer_command.arg("diff");
    add_revision_arg(&mut producer_command, revision);
    producer_command.args(&tool.producer_args);
    add_file_args(&mut producer_command, files);
    let mut producer = producer_command
        .current_dir(current_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|source| JjError::DiffStart {
            command: "jj diff".to_owned(),
            source,
        })?;
    let producer_stdout = producer
        .stdout
        .take()
        .expect("producer stdout is piped for diff rendering");

    let mut consumer = match Command::new(&tool.command)
        .args(&tool.args)
        .current_dir(current_dir)
        .stdin(Stdio::from(producer_stdout))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(consumer) => consumer,
        Err(source) => {
            let _ = producer.kill();
            let _ = producer.wait();
            return Err(JjError::DiffStart {
                command: tool.command.clone(),
                source,
            });
        }
    };

    let consumer_status = consumer.wait().map_err(|source| JjError::DiffStart {
        command: tool.command.clone(),
        source,
    })?;
    let producer_status = producer.wait().map_err(|source| JjError::DiffStart {
        command: "jj diff".to_owned(),
        source,
    })?;

    if !producer_status.success() {
        return Err(JjError::DiffFailed {
            command: "jj diff".to_owned(),
            status: exit_status_summary(producer_status),
        });
    }
    if !consumer_status.success() {
        return Err(JjError::DiffFailed {
            command: tool.command.clone(),
            status: exit_status_summary(consumer_status),
        });
    }

    Ok(())
}

fn add_revision_arg(command: &mut Command, revision: Option<&str>) {
    if let Some(revision) = revision {
        command.arg("-r").arg(revision);
    }
}

fn add_file_args(command: &mut Command, files: &[String]) {
    if !files.is_empty() {
        command.arg("--").args(files);
    }
}

fn run_inherited_command(
    mut command: Command,
    current_dir: &Path,
    label: &str,
) -> Result<(), JjError> {
    let status = command
        .current_dir(current_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| JjError::DiffStart {
            command: label.to_owned(),
            source,
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(JjError::DiffFailed {
            command: label.to_owned(),
            status: exit_status_summary(status),
        })
    }
}

fn toml_config_assignment(key: &str, value: toml::Value) -> String {
    format!("{key}={value}")
}

fn toml_string_array(values: &[String]) -> toml::Value {
    toml::Value::Array(
        values
            .iter()
            .map(|value| toml::Value::String(value.clone()))
            .collect(),
    )
}

pub(super) fn diff_paths_without_tests(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| !is_test_path(path))
        .cloned()
        .collect()
}

fn is_test_path(path: &str) -> bool {
    let path = path.replace('\\', "/");
    if path
        .split('/')
        .any(|component| matches!(component, "test" | "tests" | "__tests__"))
    {
        return true;
    }

    let file_name = path.rsplit('/').next().unwrap_or(path.as_str());
    file_name.ends_with("_test.go")
        || file_name.ends_with("_test.py")
        || file_name.starts_with("test_") && file_name.ends_with(".py")
        || file_name.contains(".test.")
        || file_name.contains(".spec.")
        || file_name.ends_with("Test.java")
        || file_name.ends_with("Tests.java")
        || file_name.ends_with("Test.kt")
        || file_name.ends_with("Tests.kt")
}
