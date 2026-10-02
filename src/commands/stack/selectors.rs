use super::*;

/// Replaces cached PR numbers with current local commit IDs, leaving other jj selectors intact.
pub(in crate::commands) fn resolve_stack_publish_revisions(
    context: &RepositoryContext,
    services: &dyn CommandServices,
    revisions: &[String],
) -> Result<Vec<String>, CommandError> {
    if !revisions
        .iter()
        .any(|revision| pull_request_number(revision).is_some())
    {
        return Ok(revisions.to_vec());
    }

    let metadata = read_stack_metadata(&context.repository_root)?;
    revisions
        .iter()
        .map(|revision| {
            let Some(number) = pull_request_number(revision) else {
                return Ok(revision.clone());
            };
            let branches = metadata
                .nodes
                .iter()
                .filter(|node| node.pull_request == Some(number))
                .map(|node| node.branch.as_str())
                .collect::<BTreeSet<_>>();
            let Some(branch) = branches.first() else {
                return Ok(revision.clone());
            };
            if branches.len() > 1 {
                return Err(CommandError::Usage(clap::Error::raw(
                    clap::error::ErrorKind::InvalidValue,
                    format!(
                        "PR {number} is associated with multiple local bookmarks: {}; select a bookmark explicitly",
                        branches.into_iter().collect::<Vec<_>>().join(", "),
                    ),
                )));
            }
            services
                .local_bookmark_commit_id(context, branch)
                .map_err(CommandError::from)
        })
        .collect()
}

fn pull_request_number(revision: &str) -> Option<u64> {
    let revision = revision.trim();
    revision
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| revision.parse().ok())
        .flatten()
}
