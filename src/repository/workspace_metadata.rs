use super::*;

const WORKSPACE_METADATA_DIR: &str = ".jx";
const WORKSPACE_METADATA_FILE: &str = "workspace.toml";
const WORKSPACE_METADATA_GITIGNORE: &str = ".gitignore";

/// Local metadata stored inside one workspace checkout.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct WorkspaceMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

impl WorkspaceMetadata {
    fn is_empty(&self) -> bool {
        self.task_id.is_none()
    }
}

/// Reads workspace-local metadata, treating a missing metadata file as empty metadata.
pub fn read_workspace_metadata(root: &Path) -> Result<WorkspaceMetadata, RepositoryError> {
    let file = workspace_metadata_file(root);
    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(WorkspaceMetadata::default());
        }
        Err(source) => return Err(RepositoryError::WorkspaceMetadataRead { file, source }),
    };

    toml::from_str(&contents)
        .map_err(|source| RepositoryError::WorkspaceMetadataParse { file, source })
}

/// Writes workspace-local metadata and keeps the metadata directory ignored by Git.
pub fn write_workspace_metadata(
    root: &Path,
    metadata: &WorkspaceMetadata,
) -> Result<(), RepositoryError> {
    let directory = root.join(WORKSPACE_METADATA_DIR);
    fs::create_dir_all(&directory).map_err(|source| RepositoryError::WorkspaceMetadataWrite {
        file: directory.clone(),
        source,
    })?;

    let gitignore = directory.join(WORKSPACE_METADATA_GITIGNORE);
    fs::write(&gitignore, "*\n").map_err(|source| RepositoryError::WorkspaceMetadataWrite {
        file: gitignore,
        source,
    })?;

    let file = directory.join(WORKSPACE_METADATA_FILE);
    if metadata.is_empty() {
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(RepositoryError::WorkspaceMetadataWrite { file, source }),
        }
        return Ok(());
    }

    let contents = toml::to_string(metadata).expect("workspace metadata serializes");
    fs::write(&file, contents)
        .map_err(|source| RepositoryError::WorkspaceMetadataWrite { file, source })
}

fn workspace_metadata_file(root: &Path) -> PathBuf {
    root.join(WORKSPACE_METADATA_DIR)
        .join(WORKSPACE_METADATA_FILE)
}
