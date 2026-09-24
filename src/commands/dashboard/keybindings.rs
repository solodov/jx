use super::*;
use crate::repository::{DashboardCommand, DashboardKey, DashboardKeyBindings};

/// Resolves configured sequences only in the main list; menus retain fixed confirmation keys.
pub(super) struct DashboardKeyboard {
    bindings: DashboardKeyBindings,
    prefix: Vec<DashboardKey>,
    help_offset: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DashboardInput {
    None,
    Cancel,
    Command(DashboardCommand),
}

impl DashboardKeyboard {
    pub(super) fn new(bindings: DashboardKeyBindings) -> Self {
        Self {
            bindings,
            prefix: Vec::new(),
            help_offset: None,
        }
    }

    /// Escape cancels local input state first, then offers cancellation to the running action.
    /// Invalid continuations are consumed rather than accidentally executing another command.
    pub(super) fn handle_key(&mut self, event: KeyEvent) -> DashboardInput {
        if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return DashboardInput::None;
        }
        if let Some(offset) = &mut self.help_offset {
            match event.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')
                    if event.kind == KeyEventKind::Press =>
                {
                    self.help_offset = None
                }
                KeyCode::Down | KeyCode::Char('j') => *offset = offset.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => *offset = offset.saturating_sub(1),
                KeyCode::PageDown => *offset = offset.saturating_add(5),
                KeyCode::PageUp => *offset = offset.saturating_sub(5),
                _ => {}
            }
            return DashboardInput::None;
        }
        if event.code == KeyCode::Esc {
            if event.kind != KeyEventKind::Press {
                return DashboardInput::None;
            }
            if !self.prefix.is_empty() {
                self.prefix.clear();
                return DashboardInput::None;
            }
            return DashboardInput::Cancel;
        }
        let Some(key) = DashboardKey::from_event(event) else {
            self.prefix.clear();
            return DashboardInput::None;
        };
        if event.kind == KeyEventKind::Repeat {
            if !self.prefix.is_empty() {
                return DashboardInput::None;
            }
            return self
                .bindings
                .sequences()
                .find_map(|(command, keys)| {
                    (keys == [key.clone()] && command.is_navigation())
                        .then_some(DashboardInput::Command(command))
                })
                .unwrap_or(DashboardInput::None);
        }
        self.prefix.push(key);
        let mut matched = None;
        let mut pending = false;
        for (command, keys) in self.bindings.sequences() {
            if keys == self.prefix {
                matched = Some(command);
                break;
            }
            pending |= keys.starts_with(&self.prefix);
        }
        if let Some(command) = matched {
            self.prefix.clear();
            if command == DashboardCommand::Help {
                self.help_offset = Some(0);
                DashboardInput::None
            } else {
                DashboardInput::Command(command)
            }
        } else {
            if !pending {
                self.prefix.clear();
            }
            DashboardInput::None
        }
    }

    pub(super) fn help_open(&self) -> bool {
        self.help_offset.is_some()
    }

    pub(super) fn help_screen(&mut self, size: DashboardTerminalSize) -> Option<menu::MenuScreen> {
        if !self.help_open() {
            return None;
        }
        let lines = self.help_lines();
        let offset = self.help_offset.as_mut()?;
        Some(menu::keybinding_help_screen(&lines, offset, size))
    }

    fn help_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "Dashboard bindings".to_owned(),
            "Esc/Enter/q closes help; j/k or PgUp/PgDn scrolls".to_owned(),
        ];
        lines.extend(DashboardCommand::ALL.into_iter().map(|command| {
            let keys = self.bindings.labels(command);
            format!(
                "{}: {}",
                command.label(),
                if keys.is_empty() {
                    "unbound".to_owned()
                } else {
                    keys.join(", ")
                }
            )
        }));
        lines.push("Esc: cancel prefix, close menu/help, or cancel action; never exits".to_owned());
        lines.push("Ctrl-C: interrupt/cancel action; otherwise exits".to_owned());
        lines.push(
            "Menu: j/k or arrows move; Enter selects; Tab/? previews; Esc closes; y/n confirms"
                .to_owned(),
        );
        lines
    }
}

#[cfg(test)]
#[path = "tests/keybindings.rs"]
mod tests;
