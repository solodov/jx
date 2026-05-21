use super::*;

/// Selects or generates a user-scoped bookmark name for the selected change.
pub fn plan_bookmark(request: BookmarkPlanRequest<'_>) -> Result<BookmarkPlan, WorkflowError> {
    let login = normalize_github_login(request.github_login)?;
    let task_id = normalize_task_id(request.task_id)?;
    let generated = generated_bookmark_name(
        &login,
        task_id.as_deref(),
        request.workspace.stack_index,
        &short_change_id(&request.workspace.target_change.change_id),
    );
    let selected_planner_bookmarks = planner_bookmarks_for_login(
        &login,
        request
            .workspace
            .local_bookmarks_at_target
            .iter()
            .map(String::as_str),
    );

    // A selected PR bookmark is the clearest expression of operator intent, even
    // if a task-specific request would generate a different branch name.
    if selected_planner_bookmarks.len() == 1 {
        return Ok(BookmarkPlan {
            branch: selected_planner_bookmarks[0].branch.clone(),
            action: BookmarkAction::Reuse,
        });
    }

    if selected_planner_bookmarks.len() > 1 {
        return Err(WorkflowError::AmbiguousSelectedBookmarks {
            bookmarks: selected_planner_bookmarks
                .into_iter()
                .map(|bookmark| bookmark.branch)
                .collect(),
        });
    }

    if request.workspace.local_bookmarks.contains(&generated) {
        return Err(WorkflowError::BookmarkExistsOnDifferentChange { branch: generated });
    }

    Ok(BookmarkPlan {
        branch: generated,
        action: BookmarkAction::Create,
    })
}

fn normalize_github_login(login: &str) -> Result<String, WorkflowError> {
    let login = login.trim();
    if login.is_empty() {
        return Err(WorkflowError::MissingGitHubLogin);
    }

    if !is_branch_namespace_component(login) {
        return Err(WorkflowError::InvalidGitHubLogin {
            login: login.to_owned(),
        });
    }

    Ok(login.to_ascii_lowercase())
}

/// Normalizes a task identifier used in generated bookmark and workspace names.
pub fn normalize_task_id(task_id: Option<&str>) -> Result<Option<String>, WorkflowError> {
    let Some(task_id) = task_id else {
        return Ok(None);
    };
    let task_id = task_id.trim();

    if task_id.is_empty() || !is_branch_namespace_component(task_id) {
        return Err(WorkflowError::InvalidTaskId {
            task_id: task_id.to_owned(),
        });
    }

    Ok(Some(task_id.to_owned()))
}

fn generated_bookmark_name(
    login: &str,
    task_id: Option<&str>,
    stack_index: usize,
    short_change_id: &str,
) -> String {
    match task_id {
        Some(task_id) => format!(
            "{login}/{task_id}-{stack_index:02}-{short_change_id}",
            task_id = task_id.to_ascii_lowercase()
        ),
        None => format!("{login}/{stack_index:02}-{short_change_id}"),
    }
}

fn short_change_id(change_id: &str) -> String {
    change_id
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn is_branch_namespace_component(value: &str) -> bool {
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedBookmark {
    branch: String,
    task_id: Option<String>,
}

fn planner_bookmarks_for_login<'a>(
    login: &str,
    bookmarks: impl IntoIterator<Item = &'a str>,
) -> Vec<ParsedBookmark> {
    bookmarks
        .into_iter()
        .filter_map(|bookmark| parse_planner_bookmark(login, bookmark))
        .collect()
}

fn parse_planner_bookmark(login: &str, bookmark: &str) -> Option<ParsedBookmark> {
    let (namespace, rest) = bookmark.split_once('/')?;
    if !namespace.eq_ignore_ascii_case(login) {
        return None;
    }
    let (prefix, short_id) = rest.rsplit_once('-')?;
    let (task_id, stack_index) = match prefix.rsplit_once('-') {
        Some((task_id, stack_index)) => (Some(task_id), stack_index),
        None => (None, prefix),
    };

    if !is_stack_index_component(stack_index)
        || !is_short_id_component(short_id)
        || task_id
            .is_some_and(|task_id| task_id.is_empty() || !is_branch_namespace_component(task_id))
    {
        return None;
    }

    Some(ParsedBookmark {
        branch: bookmark.to_owned(),
        task_id: task_id.map(str::to_owned),
    })
}

fn is_stack_index_component(value: &str) -> bool {
    value.len() >= 2 && value.chars().all(|character| character.is_ascii_digit())
}

fn is_short_id_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}
