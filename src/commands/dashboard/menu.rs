use super::*;
use crate::commands::pr_actions::AvailablePrAction;
use crate::domain::{PrActionContext, PreparedPrAction};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) struct PrActionMenu {
    target: String,
    title: String,
    entries: Vec<AvailablePrAction>,
    error: Option<String>,
    selected: usize,
    confirming: bool,
    detail_offset: usize,
}

pub(super) enum MenuIntent {
    None,
    Close,
    Run(PreparedPrAction),
}

impl PrActionMenu {
    pub(super) fn new(
        context: &PrActionContext,
        entries: Result<Vec<AvailablePrAction>, String>,
    ) -> Self {
        let (entries, error) = match entries {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Self {
            target: format!("{} #{}", context.repository.slug(), context.pr_number),
            title: context.title.clone(),
            entries,
            error,
            selected: 0,
            confirming: false,
            detail_offset: 0,
        }
    }

    /// Confirmation is bound to the frozen prepared invocation, never a refreshed row index.
    pub(super) fn handle_key(
        &mut self,
        key: KeyEvent,
        busy: bool,
        size: DashboardTerminalSize,
    ) -> MenuIntent {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q')
        ) {
            if self.confirming {
                self.confirming = false;
                return MenuIntent::None;
            }
            return MenuIntent::Close;
        }
        if size.width < 24 || size.height < 10 {
            return MenuIntent::None;
        }
        match key.code {
            KeyCode::PageDown => self.detail_offset = self.detail_offset.saturating_add(5),
            KeyCode::PageUp => self.detail_offset = self.detail_offset.saturating_sub(5),
            KeyCode::Up | KeyCode::Char('k') if !self.confirming => {
                self.selected = self.selected.saturating_sub(1);
                self.detail_offset = 0;
            }
            KeyCode::Down | KeyCode::Char('j') if !self.confirming => {
                self.selected = self
                    .selected
                    .saturating_add(1)
                    .min(self.entries.len().saturating_sub(1));
                self.detail_offset = 0;
            }
            KeyCode::Enter if key.kind == KeyEventKind::Press && !self.confirming => {
                let Some(entry) = self.entries.get(self.selected) else {
                    return MenuIntent::Close;
                };
                if !busy {
                    if let Ok(action) = &entry.prepared {
                        if action.requires_confirmation() {
                            self.confirming = true;
                            self.detail_offset = 0;
                        } else {
                            return MenuIntent::Run(action.clone());
                        }
                    }
                }
            }
            KeyCode::Char('y' | 'Y')
                if self.confirming && !busy && key.kind == KeyEventKind::Press =>
            {
                if let Some(Ok(action)) =
                    self.entries.get(self.selected).map(|entry| &entry.prepared)
                {
                    return MenuIntent::Run(action.clone());
                }
            }
            KeyCode::Char('n' | 'N') if self.confirming => self.confirming = false,
            _ => {}
        }
        MenuIntent::None
    }

    /// Renders a bounded Plan 9-colored panel. Long previews page without hiding argv boundaries.
    pub(super) fn screen(&mut self, size: DashboardTerminalSize, busy: bool) -> MenuScreen {
        if size.width == 0 || size.height == 0 {
            return MenuScreen {
                x: 0,
                y: 0,
                lines: Vec::new(),
            };
        }
        let width = size.width.min(92);
        let height = size.height.min(28);
        if width < 24 || height < 10 {
            return MenuScreen {
                x: 0,
                y: 0,
                lines: vec![panel_line(
                    "Enlarge terminal; Esc closes actions",
                    width,
                    HEADER,
                )],
            };
        }
        let inner = width - 4;
        let mut body = vec![
            (format!("PR actions — {}", self.target), HEADER),
            (plain_text(&self.title), BODY),
        ];
        if self.confirming {
            body.push((
                "Repository-local command: run this exact invocation?".to_owned(),
                HEADER,
            ));
        } else if !self.entries.is_empty() {
            let count = self
                .entries
                .len()
                .min(6)
                .min(height.saturating_sub(9).max(1));
            let start = self.selected.saturating_add(1).saturating_sub(count);
            for (index, entry) in self.entries.iter().enumerate().skip(start).take(count) {
                let selected = index == self.selected;
                body.push((
                    format!(
                        "{} {}{}",
                        if selected { "❯" } else { " " },
                        plain_text(&entry.definition.action.title),
                        if entry.prepared.is_err() {
                            " (unavailable)"
                        } else {
                            ""
                        }
                    ),
                    if selected { SELECTED } else { BODY },
                ));
            }
        }
        body.push((String::new(), BODY));
        let detail_height = height.saturating_sub(body.len() + 4).max(1);
        let details = self
            .details()
            .into_iter()
            .flat_map(|line| wrap_plain(&line, inner))
            .collect::<Vec<_>>();
        self.detail_offset = self
            .detail_offset
            .min(details.len().saturating_sub(detail_height));
        body.extend(
            details
                .iter()
                .skip(self.detail_offset)
                .take(detail_height)
                .cloned()
                .map(|line| (line, BODY)),
        );
        while body.len() < height - 4 {
            body.push((String::new(), BODY));
        }
        body.push((
            format!(
                "PgUp/PgDn details  {}–{}/{}{}",
                self.detail_offset + 1,
                (self.detail_offset + detail_height).min(details.len()),
                details.len(),
                if busy {
                    "  • refreshing; execution waits"
                } else {
                    ""
                }
            ),
            HEADER,
        ));
        body.push((
            (if self.confirming {
                "y: run   n/Esc: back"
            } else {
                "↑↓: choose   Enter: run   Esc: close"
            })
            .to_owned(),
            HEADER,
        ));
        let border = format!("+{}+", "-".repeat(width - 2));
        let mut lines = vec![panel_line(&border, width, HEADER)];
        lines.extend(body.into_iter().map(|(text, style)| {
            panel_line(&format!("| {} |", pad_plain(&text, inner)), width, style)
        }));
        lines.push(panel_line(&border, width, HEADER));
        MenuScreen {
            x: (size.width - width) / 2,
            y: (size.height - lines.len()) / 2,
            lines,
        }
    }

    fn details(&self) -> Vec<String> {
        let Some(entry) = self.entries.get(self.selected) else {
            return vec![
                self.error
                    .as_ref()
                    .map(|error| format!("Cannot load actions: {}", plain_text(error)))
                    .unwrap_or_else(|| "No actions configured for this repository.".to_owned()),
                "Configure [[repo.actions]] in ~/.config/jx/*.toml or .jx/config.toml.".to_owned(),
            ];
        };
        let mut lines = vec![format!(
            "Source: {} ({:?})",
            plain_text(&entry.definition.source.path.display().to_string()),
            entry.definition.source.scope
        )];
        let command = match &entry.prepared {
            Ok(action) => {
                lines.push(format!(
                    "cwd: {}",
                    plain_text(&action.cwd.display().to_string())
                ));
                &action.command
            }
            Err(error) => {
                lines.push(format!("Unavailable: {}", plain_text(&error.reason)));
                &entry.definition.action.command
            }
        };
        lines.extend(command.iter().enumerate().map(|(index, argument)| {
            format!(
                "argv[{index}]: {}",
                plain_text(&serde_json::to_string(argument).expect("strings serialize"))
            )
        }));
        lines
    }
}

pub(super) struct MenuScreen {
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) lines: Vec<String>,
}

const BODY: &str = "\x1b[0;38;2;0;0;0;48;2;255;255;234m";
const HEADER: &str = "\x1b[0;38;2;0;85;85;48;2;234;255;255m";
const SELECTED: &str = "\x1b[0;38;2;0;0;0;48;2;158;238;238m";

fn panel_line(text: &str, width: usize, style: &str) -> String {
    format!("{style}{}\x1b[0m", pad_plain(text, width))
}

/// Makes untrusted titles, paths, and errors visible rather than interpreting terminal controls.
pub(super) fn plain_text(text: &str) -> String {
    text.chars()
        .flat_map(|ch| {
            if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ch.escape_default().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect()
}

fn pad_plain(text: &str, width: usize) -> String {
    let text = plain_text(text);
    let mut line = String::new();
    for ch in text.chars() {
        if line.width() + ch.width().unwrap_or(0) > width {
            break;
        }
        line.push(ch);
    }
    let padding = width.saturating_sub(line.width());
    line.push_str(&" ".repeat(padding));
    line
}

fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for ch in text.chars() {
        if !line.is_empty() && line.width() + ch.width().unwrap_or(0) > width {
            lines.push(std::mem::take(&mut line));
        }
        line.push(ch);
    }
    lines.push(line);
    lines
}

#[cfg(test)]
#[path = "tests/menu.rs"]
mod tests;
