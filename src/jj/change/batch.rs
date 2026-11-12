use super::*;
use futures::AsyncReadExt as _;
use jj_lib::{
    backend::{CopyId, TreeValue},
    merge::Merge,
    merged_tree::MergedTree,
    merged_tree_builder::MergedTreeBuilder,
    repo_path::RepoPathBuf,
};

/// An ordered batch of file operations applied atomically to one JJ revision.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeBatch {
    pub operations: Vec<FileOperation>,
}

/// A text-file operation within a guarded batch. Moves and writes preserve existing modes.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum FileOperation {
    Edit {
        path: String,
        edits: Vec<FileReplacement>,
    },
    Write {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Move {
        from: String,
        to: String,
    },
}

/// One unique exact replacement, matched against the original contents of its edit operation.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileReplacement {
    pub old_text: String,
    pub new_text: String,
}

impl ChangeBatch {
    pub(super) fn validate(&self) -> Result<(), JjError> {
        if self.operations.is_empty() {
            return Err(change_request_error(
                "the batch needs at least one operation",
            ));
        }
        for operation in &self.operations {
            match operation {
                FileOperation::Edit { path, edits } => {
                    validate_path(path)?;
                    if edits.is_empty() || edits.iter().any(|edit| edit.old_text.is_empty()) {
                        return Err(change_request_error(
                            "edit operations need non-empty exact matches",
                        ));
                    }
                }
                FileOperation::Write { path, .. } | FileOperation::Delete { path } => {
                    validate_path(path)?
                }
                FileOperation::Move { from, to } => {
                    validate_path(from)?;
                    validate_path(to)?;
                    if from == to {
                        return Err(change_request_error(
                            "a move needs different source and destination paths",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Writes content-addressed tree objects, publishing nothing until the caller commits.
    pub(super) async fn apply(&self, mut tree: MergedTree) -> Result<MergedTree, JjError> {
        for operation in &self.operations {
            let mut builder = MergedTreeBuilder::new(tree.clone());
            match operation {
                FileOperation::Edit { path, edits } => {
                    let path = checked_path(&tree, path).await?;
                    let value = regular_file(&tree, &path).await?;
                    let TreeValue::File { id, .. } = &value else {
                        unreachable!()
                    };
                    let mut input = tree
                        .store()
                        .read_file(&path, id)
                        .await
                        .map_err(change_request_error)?;
                    let mut original = String::new();
                    input
                        .read_to_string(&mut original)
                        .await
                        .map_err(change_request_error)?;
                    let content = replace_exact(&original, edits)?;
                    write_text(&tree, &mut builder, path, &content, Some(value)).await?;
                }
                FileOperation::Write { path, content } => {
                    let path = checked_path(&tree, path).await?;
                    let value = tree.path_value(&path).await.map_err(change_request_error)?;
                    let value = value
                        .into_resolved()
                        .map_err(|_| change_request_error("the path is conflicted"))?;
                    if value
                        .as_ref()
                        .is_some_and(|value| !matches!(value, TreeValue::File { .. }))
                    {
                        return Err(change_request_error(
                            "writes require a regular file or an absent path",
                        ));
                    }
                    write_text(&tree, &mut builder, path, content, value).await?;
                }
                FileOperation::Delete { path } => {
                    let path = checked_path(&tree, path).await?;
                    regular_file(&tree, &path).await?;
                    builder.set_or_remove(path, Merge::absent());
                }
                FileOperation::Move { from, to } => {
                    let from = checked_path(&tree, from).await?;
                    let to = checked_path(&tree, to).await?;
                    let value = regular_file(&tree, &from).await?;
                    if tree
                        .path_value(&to)
                        .await
                        .map_err(change_request_error)?
                        .as_resolved()
                        != Some(&None)
                    {
                        return Err(change_request_error(
                            "move destination already exists or is conflicted",
                        ));
                    }
                    builder.set_or_remove(from, Merge::absent());
                    builder.set_or_remove(to, Merge::normal(value));
                }
            }
            tree = builder.write_tree().await.map_err(change_request_error)?;
        }
        Ok(tree)
    }
}

pub(super) fn validate_path(path: &str) -> Result<(), JjError> {
    if path.is_empty()
        || path.contains('\\')
        || path.contains('\0')
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.eq_ignore_ascii_case(".git")
                || part.eq_ignore_ascii_case(".jj")
        })
        || Path::new(path).is_absolute()
    {
        return Err(change_request_error(format!(
            "not a safe repository-relative path: {path:?}"
        )));
    }
    Ok(())
}

async fn checked_path(tree: &MergedTree, path: &str) -> Result<RepoPathBuf, JjError> {
    validate_path(path)?;
    let path = RepoPathBuf::from_internal_string(path).map_err(change_request_error)?;
    let mut parent = path.parent();
    while let Some(directory) = parent.filter(|directory| !directory.is_root()) {
        match tree
            .path_value(directory)
            .await
            .map_err(change_request_error)?
            .as_resolved()
        {
            Some(None | Some(TreeValue::Tree(_))) => {}
            _ => {
                return Err(change_request_error(
                    "path parents must be ordinary directories, not files, symlinks, or conflicts",
                ))
            }
        }
        parent = directory.parent();
    }
    Ok(path)
}

async fn regular_file(tree: &MergedTree, path: &RepoPath) -> Result<TreeValue, JjError> {
    match tree
        .path_value(path)
        .await
        .map_err(change_request_error)?
        .into_resolved()
    {
        Ok(Some(value @ TreeValue::File { .. })) => Ok(value),
        _ => Err(change_request_error(
            "the path must name an existing, unconflicted regular file",
        )),
    }
}

async fn write_text(
    tree: &MergedTree,
    builder: &mut MergedTreeBuilder,
    path: RepoPathBuf,
    content: &str,
    previous: Option<TreeValue>,
) -> Result<(), JjError> {
    let (executable, copy_id) = match previous {
        Some(TreeValue::File {
            executable,
            copy_id,
            ..
        }) => (executable, copy_id),
        _ => (false, CopyId::placeholder()),
    };
    let id = tree
        .store()
        .write_file(&path, &mut content.as_bytes())
        .await
        .map_err(change_request_error)?;
    builder.set_or_remove(
        path,
        Merge::normal(TreeValue::File {
            id,
            executable,
            copy_id,
        }),
    );
    Ok(())
}

fn replace_exact(original: &str, edits: &[FileReplacement]) -> Result<String, JjError> {
    let mut replacements = Vec::new();
    for edit in edits {
        if edit.old_text.is_empty() {
            return Err(change_request_error("old_text must not be empty"));
        }
        let Some(start) = original.find(&edit.old_text) else {
            return Err(change_request_error("an edit's old_text was not found"));
        };
        // Start after the first character, not after the whole match, so
        // overlapping occurrences such as "aa" in "aaa" are ambiguous too.
        let next = start
            + original[start..]
                .chars()
                .next()
                .expect("non-empty match")
                .len_utf8();
        if original[next..].contains(&edit.old_text) {
            return Err(change_request_error(
                "an edit's old_text must match exactly once",
            ));
        }
        replacements.push((start, start + edit.old_text.len(), &edit.new_text));
    }
    replacements.sort_by_key(|(start, _, _)| *start);
    if replacements.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(change_request_error(
            "edits overlap; combine them into one replacement",
        ));
    }
    let mut result = original.to_owned();
    for (start, end, text) in replacements.into_iter().rev() {
        result.replace_range(start..end, text);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "../tests/change_batch.rs"]
mod tests;
