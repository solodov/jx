use super::*;

/// Plans a bookmark push, creating a deterministic bookmark only when the selected change has none.
pub fn push_plan(
    context: &RepositoryContext,
    workspace: WorkspaceFacts,
    requested_revision: Option<&str>,
) -> Result<PushPlan, WorkflowError> {
    let bookmark = push_bookmark_plan(&workspace, requested_revision)?;
    let title = first_description_line(&workspace.target_change.description);

    Ok(PushPlan {
        repository: repository_summary(context),
        bookmark,
        target_commit_id: workspace.target_change.commit_id,
        target_short_commit_id: workspace.target_change.short_commit_id,
        title,
    })
}

/// Builds the operator-facing report for a completed selected-bookmark push.
pub fn push_report(
    context: &RepositoryContext,
    plan: PushPlan,
    bookmark_update: BookmarkUpdate,
    push: PushOutcome,
) -> PushReport {
    PushReport {
        repository: repository_summary(context),
        plan,
        bookmark_update,
        push,
    }
}

/// Builds the operator-facing report for a completed tracked-bookmark push.
pub fn tracked_push_report(
    context: &RepositoryContext,
    outcome: TrackedPushOutcome,
) -> TrackedPushReport {
    TrackedPushReport {
        repository: repository_summary(context),
        outcome,
    }
}

fn push_bookmark_plan(
    workspace: &WorkspaceFacts,
    requested_revision: Option<&str>,
) -> Result<BookmarkPlan, WorkflowError> {
    let requested_bookmark = requested_revision
        .map(str::trim)
        .filter(|revision| !revision.is_empty())
        .and_then(|revision| {
            workspace
                .local_bookmarks_at_target
                .iter()
                .find(|bookmark| bookmark.as_str() == revision)
        });

    if let Some(bookmark) =
        requested_bookmark.or_else(|| workspace.local_bookmarks_at_target.first())
    {
        return Ok(BookmarkPlan {
            branch: bookmark.clone(),
            action: BookmarkAction::Reuse,
        });
    }

    let generated = generated_push_bookmark_name(workspace);
    if workspace.local_bookmarks.contains(&generated) {
        return Err(WorkflowError::PushBookmarkExistsOnDifferentChange { branch: generated });
    }

    Ok(BookmarkPlan {
        branch: generated,
        action: BookmarkAction::Create,
    })
}

fn generated_push_bookmark_name(workspace: &WorkspaceFacts) -> String {
    first_ticket_id(&workspace.target_change.description).map_or_else(
        || {
            format!(
                "push-{}",
                short_change_id(&workspace.target_change.change_id)
            )
        },
        |ticket| {
            format!(
                "ps/{ticket}-{stack_index:02}-{short_change_id}",
                stack_index = workspace.stack_index,
                short_change_id = short_change_id(&workspace.target_change.change_id)
            )
        },
    )
}

fn first_ticket_id(description: &str) -> Option<String> {
    let bytes = description.as_bytes();
    let mut start = 0;

    while start < bytes.len() {
        if !bytes[start].is_ascii_alphabetic() {
            start += 1;
            continue;
        }

        let mut cursor = start + 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_alphanumeric() {
            cursor += 1;
        }

        if cursor > start + 1 && cursor < bytes.len() && bytes[cursor] == b'-' {
            let digit_start = cursor + 1;
            cursor = digit_start;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            if cursor > digit_start {
                return Some(description[start..cursor].to_ascii_lowercase());
            }
        }

        start += 1;
    }

    None
}

fn short_change_id(change_id: &str) -> String {
    change_id
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn first_description_line(description: &str) -> String {
    description
        .lines()
        .find_map(|line| {
            let line = line.trim();
            (!line.is_empty()).then_some(line)
        })
        .unwrap_or("(no description)")
        .to_owned()
}
