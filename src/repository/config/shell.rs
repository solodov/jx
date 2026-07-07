use super::*;

/// Optional shell integration preferences used by `jx shell init`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellConfig {
    pub navigation: Option<String>,
    pub navigation_tab: Option<String>,
    pub title: bool,
    pub slug_repositories: Vec<String>,
    pub zoxide: ShellZoxideMode,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            navigation: None,
            navigation_tab: None,
            title: false,
            slug_repositories: Vec::new(),
            zoxide: ShellZoxideMode::Auto,
        }
    }
}

impl ShellConfig {
    pub(super) fn apply_layer(&mut self, layer: ShellConfigLayer) {
        if let Some(navigation) = layer.navigation {
            self.navigation = Some(navigation);
        }
        if let Some(navigation_tab) = layer.navigation_tab {
            self.navigation_tab = Some(navigation_tab);
        }
        if let Some(title) = layer.title {
            self.title = title;
        }
        if let Some(slug_repositories) = layer.slug_repositories {
            self.slug_repositories.extend(slug_repositories);
            self.slug_repositories.sort();
            self.slug_repositories.dedup();
        }
        if let Some(zoxide) = layer.zoxide {
            self.zoxide = zoxide;
        }
    }

    pub(super) fn validate(&self) -> Result<(), RepositoryError> {
        self.validate_navigation()?;
        self.validate_slug_repository_patterns()?;
        Ok(())
    }

    fn validate_navigation(&self) -> Result<(), RepositoryError> {
        let Some(command) = self.navigation_command() else {
            return if self.navigation_tab_command().is_none() {
                Ok(())
            } else {
                Err(RepositoryError::InvalidConfig {
                    file: "jx config".to_owned(),
                    message: "`shell.navigation_tab` requires `shell.navigation`".to_owned(),
                })
            };
        };

        if !is_valid_shell_function_name(command) {
            return Err(RepositoryError::InvalidConfig {
                file: "jx config".to_owned(),
                message: "`shell.navigation` must be empty or a safe shell function name"
                    .to_owned(),
            });
        }

        let Some(tab_command) = self.navigation_tab_command() else {
            return Ok(());
        };
        if !is_valid_shell_function_name(tab_command) {
            return Err(RepositoryError::InvalidConfig {
                file: "jx config".to_owned(),
                message: "`shell.navigation_tab` must be empty or a safe shell function name"
                    .to_owned(),
            });
        }
        if tab_command == command {
            return Err(RepositoryError::InvalidConfig {
                file: "jx config".to_owned(),
                message: "`shell.navigation_tab` must be different from `shell.navigation`"
                    .to_owned(),
            });
        }

        Ok(())
    }

    fn validate_slug_repository_patterns(&self) -> Result<(), RepositoryError> {
        for pattern in &self.slug_repositories {
            Glob::new(pattern).map_err(|source| RepositoryError::InvalidConfig {
                file: "jx config".to_owned(),
                message: format!(
                    "`shell.slug_repositories` contains invalid glob `{pattern}`: {source}"
                ),
            })?;
        }

        Ok(())
    }

    /// Returns the configured navigation command name, if shell navigation is enabled.
    pub fn navigation_command(&self) -> Option<&str> {
        self.navigation
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
    }

    /// Returns the configured tab-opening navigation command name, if enabled.
    pub fn navigation_tab_command(&self) -> Option<&str> {
        self.navigation_tab
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
    }

    /// Returns whether generated shell integration should own terminal titles.
    pub fn title_enabled(&self) -> bool {
        self.title
    }

    /// Returns repository globs that should render as `owner/repo` in shell titles.
    pub fn slug_repository_patterns(&self) -> &[String] {
        &self.slug_repositories
    }
}

/// Controls optional zoxide fallback in generated shell navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellZoxideMode {
    Auto,
    Never,
    Prefer,
}

#[derive(Debug, Default)]
pub(super) struct ShellConfigLayer {
    pub(super) navigation: Option<String>,
    pub(super) navigation_tab: Option<String>,
    pub(super) title: Option<bool>,
    pub(super) slug_repositories: Option<Vec<String>>,
    pub(super) zoxide: Option<ShellZoxideMode>,
}

fn is_valid_shell_function_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !is_shell_reserved_word(name)
}

fn is_shell_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "!" | "case"
            | "coproc"
            | "do"
            | "done"
            | "elif"
            | "else"
            | "esac"
            | "fi"
            | "for"
            | "function"
            | "if"
            | "in"
            | "select"
            | "then"
            | "time"
            | "until"
            | "while"
            | "{"
            | "}"
    )
}
