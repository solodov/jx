use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Named dashboard operations shared by review and stack status, not shell commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DashboardCommand {
    Refresh,
    Up,
    Down,
    PageUp,
    PageDown,
    First,
    Last,
    Menu,
    Help,
    Quit,
}

impl DashboardCommand {
    pub const ALL: [Self; 10] = [
        Self::Refresh,
        Self::Up,
        Self::Down,
        Self::PageUp,
        Self::PageDown,
        Self::First,
        Self::Last,
        Self::Menu,
        Self::Help,
        Self::Quit,
    ];

    /// Configuration name for this operation.
    pub fn name(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::Up => "up",
            Self::Down => "down",
            Self::PageUp => "page_up",
            Self::PageDown => "page_down",
            Self::First => "first",
            Self::Last => "last",
            Self::Menu => "menu",
            Self::Help => "help",
            Self::Quit => "quit",
        }
    }

    /// Human-readable operation name used by the effective-keymap help.
    pub fn label(self) -> &'static str {
        match self {
            Self::Refresh => "Refresh",
            Self::Up => "Previous PR",
            Self::Down => "Next PR",
            Self::PageUp => "Page up",
            Self::PageDown => "Page down",
            Self::First => "First PR",
            Self::Last => "Last PR",
            Self::Menu => "Action menu",
            Self::Help => "Show bindings",
            Self::Quit => "Exit dashboard",
        }
    }

    /// Only navigation can repeat when a key is held down.
    pub fn is_navigation(self) -> bool {
        matches!(
            self,
            Self::Up | Self::Down | Self::PageUp | Self::PageDown | Self::First | Self::Last
        )
    }
}

/// Normalized keystroke; printable uppercase characters already encode Shift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardKey {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl DashboardKey {
    /// Normalizes terminal key events and rejects unsupported modifier combinations.
    pub fn from_event(event: KeyEvent) -> Option<Self> {
        let mut code = event.code;
        let mut modifiers = event.modifiers;
        if !modifiers
            .difference(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT)
            .is_empty()
        {
            return None;
        }
        if code == KeyCode::BackTab {
            code = KeyCode::Tab;
            modifiers.insert(KeyModifiers::SHIFT);
        }
        if let KeyCode::Char(mut ch) = code {
            if modifiers.contains(KeyModifiers::SHIFT) {
                ch = ch.to_ascii_uppercase();
            }
            if modifiers.contains(KeyModifiers::CONTROL) {
                ch = ch.to_ascii_lowercase();
            }
            modifiers.remove(KeyModifiers::SHIFT);
            code = KeyCode::Char(ch);
        }
        Some(Self { code, modifiers })
    }

    /// Parses a named key or character, optionally prefixed by Ctrl-, Alt-, or Shift-.
    pub fn parse(token: &str) -> Result<Self, String> {
        let mut rest = token;
        let mut modifiers = KeyModifiers::NONE;
        loop {
            let modifier = [
                ("Ctrl-", KeyModifiers::CONTROL),
                ("Alt-", KeyModifiers::ALT),
                ("Shift-", KeyModifiers::SHIFT),
            ]
            .into_iter()
            .find(|(prefix, _)| rest.starts_with(prefix));
            let Some((prefix, modifier)) = modifier else {
                break;
            };
            if modifiers.contains(modifier) {
                return Err(format!("duplicate modifier in `{token}`"));
            }
            modifiers.insert(modifier);
            rest = &rest[prefix.len()..];
        }
        let code = match rest {
            "Up" => KeyCode::Up,
            "Down" => KeyCode::Down,
            "Left" => KeyCode::Left,
            "Right" => KeyCode::Right,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            "PageUp" => KeyCode::PageUp,
            "PageDown" => KeyCode::PageDown,
            "Enter" => KeyCode::Enter,
            "Tab" => KeyCode::Tab,
            "BackTab" => KeyCode::BackTab,
            "Backspace" => KeyCode::Backspace,
            "Delete" => KeyCode::Delete,
            "Space" => KeyCode::Char(' '),
            "Esc" => KeyCode::Esc,
            _ if rest.chars().count() == 1 => {
                let ch = rest.chars().next().expect("one character");
                if ch.is_control() || ch.is_whitespace() {
                    return Err(format!("invalid key `{token}`"));
                }
                KeyCode::Char(ch)
            }
            _ => match rest
                .strip_prefix('F')
                .and_then(|number| number.parse::<u8>().ok())
            {
                Some(number @ 1..=12) => KeyCode::F(number),
                _ => return Err(format!("unknown key `{token}`")),
            },
        };
        let key = Self::from_event(KeyEvent::new(code, modifiers)).expect("supported modifiers");
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            return Err(format!(
                "`{token}` is reserved for cancellation/interruption"
            ));
        }
        Ok(key)
    }

    /// Canonical display spelling, shared by help and pending-prefix hints.
    pub fn label(&self) -> String {
        let mut label = String::new();
        for (modifier, prefix) in [
            (KeyModifiers::CONTROL, "Ctrl-"),
            (KeyModifiers::ALT, "Alt-"),
            (KeyModifiers::SHIFT, "Shift-"),
        ] {
            if self.modifiers.contains(modifier) {
                label.push_str(prefix);
            }
        }
        label.push_str(&match self.code {
            KeyCode::Char(' ') => "Space".to_owned(),
            KeyCode::Char(ch) => ch.to_string(),
            KeyCode::F(number) => format!("F{number}"),
            code => format!("{code:?}"),
        });
        label
    }
}

/// Effective dashboard bindings. Each configured operation replaces its inherited sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardKeyBindings {
    bindings: BTreeMap<DashboardCommand, Vec<Vec<DashboardKey>>>,
}

impl Default for DashboardKeyBindings {
    fn default() -> Self {
        use DashboardCommand::*;
        let bindings = [
            (Refresh, vec!["g r"]),
            (Up, vec!["k", "Up"]),
            (Down, vec!["j", "Down"]),
            (PageUp, vec!["PageUp"]),
            (PageDown, vec!["PageDown"]),
            (First, vec!["g g", "Home"]),
            (Last, vec!["G", "End"]),
            (Menu, vec!["Enter"]),
            (Help, vec!["?"]),
            (Quit, vec!["q"]),
        ]
        .into_iter()
        .map(|(command, sequences)| {
            (
                command,
                sequences
                    .into_iter()
                    .map(|sequence| parse_sequence(sequence).expect("valid default sequence"))
                    .collect(),
            )
        })
        .collect();
        Self { bindings }
    }
}

impl DashboardKeyBindings {
    pub(super) fn apply_layer(
        &mut self,
        bindings: BTreeMap<DashboardCommand, Vec<Vec<DashboardKey>>>,
    ) {
        self.bindings.extend(bindings);
    }

    /// Iterates complete sequences in stable operation order.
    pub fn sequences(&self) -> impl Iterator<Item = (DashboardCommand, &[DashboardKey])> {
        self.bindings.iter().flat_map(|(command, sequences)| {
            sequences
                .iter()
                .map(move |sequence| (*command, sequence.as_slice()))
        })
    }

    /// Returns the configured alternatives for one operation, or an empty list when unbound.
    pub fn labels(&self, command: DashboardCommand) -> Vec<String> {
        self.sequences()
            .filter(|(candidate, _)| *candidate == command)
            .map(|(_, keys)| sequence_label(keys))
            .collect()
    }

    /// Rejects duplicate or prefix-overlapping sequences after all global layers are merged.
    pub(super) fn validate(&self) -> Result<(), RepositoryError> {
        let sequences = self.sequences().collect::<Vec<_>>();
        for (index, (left_command, left)) in sequences.iter().enumerate() {
            for (right_command, right) in &sequences[index + 1..] {
                if left.starts_with(right) || right.starts_with(left) {
                    return Err(invalid(
                        "jx config",
                        format!(
                            "ambiguous dashboard bindings: `{} = {}` and `{} = {}`",
                            left_command.name(),
                            sequence_label(left),
                            right_command.name(),
                            sequence_label(right)
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

pub(super) type DashboardKeyBindingsLayer = BTreeMap<DashboardCommand, Vec<Vec<DashboardKey>>>;

/// Parses only named dashboard operations; no arbitrary commands or terminal escape strings.
pub(super) fn parse_dashboard_keys(
    file: &str,
    value: &toml::Value,
) -> Result<DashboardKeyBindingsLayer, RepositoryError> {
    let table = value
        .as_table()
        .ok_or_else(|| invalid(file, "`ui.dashboard.keys` must be a table".to_owned()))?;
    let mut bindings = BTreeMap::new();
    for (name, value) in table {
        let command = DashboardCommand::ALL
            .into_iter()
            .find(|command| command.name() == name)
            .ok_or_else(|| RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("ui.dashboard.keys.{name}"),
            })?;
        let sequences = value.as_array().ok_or_else(|| {
            invalid(
                file,
                format!("`ui.dashboard.keys.{name}` must be an array of key sequences"),
            )
        })?;
        let sequences = sequences
            .iter()
            .map(|value| {
                let sequence = value.as_str().ok_or_else(|| {
                    invalid(
                        file,
                        format!("`ui.dashboard.keys.{name}` entries must be strings"),
                    )
                })?;
                parse_sequence(sequence).map_err(|message| {
                    invalid(file, format!("`ui.dashboard.keys.{name}`: {message}"))
                })
            })
            .collect::<Result<_, _>>()?;
        bindings.insert(command, sequences);
    }
    Ok(bindings)
}

fn parse_sequence(sequence: &str) -> Result<Vec<DashboardKey>, String> {
    let keys = sequence
        .split_whitespace()
        .map(DashboardKey::parse)
        .collect::<Result<Vec<_>, _>>()?;
    if keys.is_empty() {
        return Err("key sequences cannot be empty; use [] to unbind an operation".to_owned());
    }
    Ok(keys)
}

fn sequence_label(keys: &[DashboardKey]) -> String {
    keys.iter()
        .map(DashboardKey::label)
        .collect::<Vec<_>>()
        .join(" ")
}

fn invalid(file: &str, message: String) -> RepositoryError {
    RepositoryError::InvalidConfig {
        file: file.to_owned(),
        message,
    }
}
