use super::*;

const WORKSPACE_METADATA_DIR: &str = ".jx";
const WORKSPACE_METADATA_FILE: &str = "workspace.toml";
const STACK_METADATA_FILE: &str = "stack.toml";
const WORKSPACE_METADATA_GITIGNORE: &str = ".gitignore";
const WORKSPACE_METADATA_GITIGNORE_CONTENT: &str = "/.gitignore\n/workspace.toml\n/stack.toml\n";

/// Local metadata stored inside one workspace checkout.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct WorkspaceMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// Durable local PR stack state used to render and synchronize stack context.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct StackMetadata {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_item_handler_runs: Vec<StackMetadataWorkItemHandlerRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<StackMetadataNode>,
}

/// Successful work-item handler application recorded to avoid duplicate external side effects.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
pub struct StackMetadataWorkItemHandlerRun {
    pub handler: String,
    pub work_id: String,
    pub pull_request: u64,
}

/// One pull-request node in a locally managed stack.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct StackMetadataNode {
    pub branch: String,
    pub base_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_pull_request: Option<u64>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub draft: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub merged: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixes_work_ids: Vec<String>,
}

impl WorkspaceMetadata {
    fn is_empty(&self) -> bool {
        self.task_id.is_none()
    }
}

/// Reads workspace-local metadata, treating a missing metadata file as empty metadata.
pub fn read_workspace_metadata(root: &Path) -> Result<WorkspaceMetadata, RepositoryError> {
    read_metadata_file(&workspace_metadata_file(root))
}

/// Reads repo-local stack metadata, treating a missing stack file as empty state.
pub fn read_stack_metadata(root: &Path) -> Result<StackMetadata, RepositoryError> {
    read_metadata_file(&stack_metadata_file(root))
}

fn read_metadata_file<T>(file: &Path) -> Result<T, RepositoryError>
where
    T: Default + serde::de::DeserializeOwned,
{
    let contents = match fs::read_to_string(file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(T::default());
        }
        Err(source) => {
            return Err(RepositoryError::WorkspaceMetadataRead {
                file: file.to_path_buf(),
                source,
            });
        }
    };

    toml::from_str(&contents).map_err(|source| RepositoryError::WorkspaceMetadataParse {
        file: file.to_path_buf(),
        source,
    })
}

/// Writes workspace-local metadata and keeps generated local metadata ignored by Git.
pub fn write_workspace_metadata(
    root: &Path,
    metadata: &WorkspaceMetadata,
) -> Result<(), RepositoryError> {
    let file = workspace_metadata_file(root);
    if metadata.is_empty() {
        ensure_metadata_directory_ignored(root)?;
        remove_metadata_file(file)?;
    } else {
        write_metadata_file(root, file, metadata)?;
    }
    Ok(())
}

/// Writes repo-local stack metadata and keeps generated local metadata ignored by Git.
pub fn write_stack_metadata(root: &Path, metadata: &StackMetadata) -> Result<(), RepositoryError> {
    let file = stack_metadata_file(root);
    if metadata.nodes.is_empty() && metadata.work_item_handler_runs.is_empty() {
        ensure_metadata_directory_ignored(root)?;
        remove_metadata_file(file)?;
    } else {
        write_metadata_file(root, file, metadata)?;
    }
    Ok(())
}

fn write_metadata_file<T>(root: &Path, file: PathBuf, metadata: &T) -> Result<(), RepositoryError>
where
    T: serde::Serialize,
{
    ensure_metadata_directory_ignored(root)?;
    let contents = toml::to_string(metadata).expect("workspace metadata serializes");
    fs::write(&file, contents)
        .map_err(|source| RepositoryError::WorkspaceMetadataWrite { file, source })
}

fn remove_metadata_file(file: PathBuf) -> Result<(), RepositoryError> {
    match fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(RepositoryError::WorkspaceMetadataWrite { file, source }),
    }
}

fn ensure_metadata_directory_ignored(root: &Path) -> Result<(), RepositoryError> {
    let directory = root.join(WORKSPACE_METADATA_DIR);
    fs::create_dir_all(&directory).map_err(|source| RepositoryError::WorkspaceMetadataWrite {
        file: directory.clone(),
        source,
    })?;

    let gitignore = directory.join(WORKSPACE_METADATA_GITIGNORE);
    fs::write(&gitignore, WORKSPACE_METADATA_GITIGNORE_CONTENT).map_err(|source| {
        RepositoryError::WorkspaceMetadataWrite {
            file: gitignore,
            source,
        }
    })
}

fn workspace_metadata_file(root: &Path) -> PathBuf {
    root.join(WORKSPACE_METADATA_DIR)
        .join(WORKSPACE_METADATA_FILE)
}

fn stack_metadata_file(root: &Path) -> PathBuf {
    root.join(WORKSPACE_METADATA_DIR).join(STACK_METADATA_FILE)
}

fn is_false(value: &bool) -> bool {
    !*value
}
