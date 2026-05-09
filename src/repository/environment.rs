use super::*;

/// Process state used by command orchestration and repository discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEnvironment {
    current_dir: PathBuf,
    variables: BTreeMap<String, String>,
}

impl RuntimeEnvironment {
    /// Captures the current process directory and environment variables.
    pub fn from_process() -> io::Result<Self> {
        Ok(Self {
            current_dir: env::current_dir()?,
            variables: env::vars().collect(),
        })
    }

    /// Builds an environment for tests or embedded callers.
    pub fn new(
        current_dir: impl Into<PathBuf>,
        variables: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            current_dir: current_dir.into(),
            variables: variables.into_iter().collect(),
        }
    }

    /// Current working directory used as the starting point for workspace discovery.
    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    pub(super) fn variable(&self, name: &str) -> Option<&str> {
        self.variables.get(name).map(String::as_str)
    }
}
