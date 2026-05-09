use super::*;

/// Optional shell integration preferences used by `jx shell init`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellConfig {
    pub navigation: Option<String>,
    pub zoxide: ShellZoxideMode,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            navigation: None,
            zoxide: ShellZoxideMode::Auto,
        }
    }
}

impl ShellConfig {
    pub(super) fn apply_layer(&mut self, layer: ShellConfigLayer) {
        if let Some(navigation) = layer.navigation {
            self.navigation = Some(navigation);
        }
        if let Some(zoxide) = layer.zoxide {
            self.zoxide = zoxide;
        }
    }

    pub(super) fn validate(&self) -> Result<(), RepositoryError> {
        let Some(command) = self.navigation_command() else {
            return Ok(());
        };

        if is_valid_shell_function_name(command) {
            Ok(())
        } else {
            Err(RepositoryError::InvalidConfig {
                file: "jx config".to_owned(),
                message: "`shell.navigation` must be empty or a safe shell function name"
                    .to_owned(),
            })
        }
    }

    /// Returns the configured navigation command name, if shell navigation is enabled.
    pub fn navigation_command(&self) -> Option<&str> {
        self.navigation
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
    }
}

/// Controls optional zoxide fallback in generated shell navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellZoxideMode {
    Auto,
    Never,
}

#[derive(Debug, Default)]
pub(super) struct ShellConfigLayer {
    pub(super) navigation: Option<String>,
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
