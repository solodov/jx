use super::*;
use crate::jj::LogTimings;

/// Renders the workspace log and records local phases without changing output or error handling.
pub(super) fn handle_log(
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
) -> Result<String, CommandError> {
    let mut span = PerfLog::from_environment(environment).start(
        "log.run",
        [perf_attr(
            "current_dir",
            environment.current_dir().display().to_string(),
        )],
    );
    let mut timings = LogTimings::default();
    let result = (|| {
        let annotations = span.measure("load_annotations", Vec::new(), || {
            workspace_log_annotations(environment)
        })?;
        span.set([perf_attr("annotation_count", annotations.len())]);
        span.measure_with_result_attrs(
            "workspace_log",
            Vec::new(),
            || services.workspace_log(&annotations, &mut timings),
            |result| {
                result
                    .as_ref()
                    .map(|output| vec![perf_attr("output_bytes", output.len())])
                    .unwrap_or_default()
            },
        )
        .map_err(CommandError::from)
    })();
    record_log_timings(&mut span, timings);
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn record_log_timings(span: &mut PerfSpan, timings: LogTimings) {
    if let Some(root) = timings.workspace_root {
        span.set([perf_attr("workspace_root", root.display().to_string())]);
    }
    if let Some(count) = timings.immutable_commit_count {
        span.set([perf_attr("immutable_commit_count", count)]);
    }
    for step in timings.steps {
        span.record_step_us(step.name, step.duration_us, Vec::new(), step.error.as_ref());
    }
}

fn workspace_log_annotations(
    environment: &RuntimeEnvironment,
) -> Result<Vec<LogBookmarkAnnotation>, CommandError> {
    let context = match RepositoryContext::discover(environment) {
        Ok(context) => context,
        Err(error) if workspace_log_annotations_are_optional(&error) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let metadata = read_stack_metadata(&context.repository_root)?;
    let repository_url = context.origin.github.https_url();

    Ok(metadata
        .nodes
        .iter()
        .filter_map(|node| workspace_log_annotation(&repository_url, node))
        .collect())
}

fn workspace_log_annotations_are_optional(error: &RepositoryError) -> bool {
    matches!(
        error,
        RepositoryError::WorkspaceNotFound
            | RepositoryError::MissingOrigin
            | RepositoryError::OriginNotGitHub { .. }
    )
}

fn workspace_log_annotation(
    repository_url: &str,
    node: &StackMetadataNode,
) -> Option<LogBookmarkAnnotation> {
    let pull_request = node.pull_request?;
    Some(LogBookmarkAnnotation {
        bookmark: node.branch.clone(),
        label: pull_request.to_string(),
        draft: node.draft,
        url: Some(
            node.url
                .clone()
                .unwrap_or_else(|| format!("{repository_url}/pull/{pull_request}")),
        ),
    })
}
