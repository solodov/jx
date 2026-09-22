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
    showing_details: bool,
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
            showing_details: false,
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
            if self.confirming || self.showing_details {
                self.confirming = false;
                self.showing_details = false;
                self.detail_offset = 0;
                return MenuIntent::None;
            }
            return MenuIntent::Close;
        }
        if self.entries.is_empty() && key.code == KeyCode::Enter && key.kind == KeyEventKind::Press
        {
            return MenuIntent::Close;
        }
        if size.width < 24 || size.height < 10 {
            return MenuIntent::None;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Char('?')
                if !self.confirming
                    && key.kind == KeyEventKind::Press
                    && self.selected < self.entries.len() =>
            {
                self.showing_details = !self.showing_details;
                self.detail_offset = 0;
            }
            KeyCode::PageDown
                if self.showing_details || self.confirming || self.entries.is_empty() =>
            {
                self.detail_offset = self.detail_offset.saturating_add(5);
            }
            KeyCode::PageUp
                if self.showing_details || self.confirming || self.entries.is_empty() =>
            {
                self.detail_offset = self.detail_offset.saturating_sub(5);
            }
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
                match &entry.prepared {
                    Ok(action) if !busy => {
                        if action.requires_confirmation() {
                            self.confirming = true;
                            self.showing_details = false;
                            self.detail_offset = 0;
                        } else {
                            return MenuIntent::Run(action.clone());
                        }
                    }
                    Ok(_) => {}
                    Err(_) => self.showing_details = true,
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

    /// Renders the frozen action set without transient dashboard refresh state.
    pub(super) fn screen(
        &mut self,
        size: DashboardTerminalSize,
        anchor_row: Option<usize>,
    ) -> MenuScreen {
        if size.width == 0 || size.height == 0 {
            return MenuScreen {
                x: 0,
                y: 0,
                lines: Vec::new(),
            };
        }
        if self.entries.is_empty() && self.error.is_none() {
            let message = "no actions configured";
            let width = (message.width() + 4).min(size.width);
            let desired_height = if width >= 4 && size.height >= 3 { 3 } else { 1 };
            let (y, height) = menu_placement(size.height, desired_height, anchor_row);
            let lines = if height >= 3 {
                bordered_lines(&[(message, BODY)], width)
            } else if height > 0 {
                vec![panel_line(message, width, BODY)]
            } else {
                Vec::new()
            };
            return MenuScreen {
                x: 2.min(size.width - width),
                y,
                lines,
            };
        }
        if size.width < 24 || size.height < 10 {
            let (y, height) = menu_placement(size.height, 1, anchor_row);
            return MenuScreen {
                x: 0,
                y,
                lines: if height > 0 {
                    vec![panel_line(
                        "Enlarge terminal; Esc closes actions",
                        size.width,
                        BODY,
                    )]
                } else {
                    Vec::new()
                },
            };
        }
        if self.confirming || self.showing_details || self.entries.is_empty() {
            self.detail_screen(size)
        } else {
            self.list_screen(size, anchor_row)
        }
    }

    fn list_screen(&self, size: DashboardTerminalSize, anchor_row: Option<usize>) -> MenuScreen {
        let labels = self
            .entries
            .iter()
            .map(|entry| {
                format!(
                    "{}{}",
                    plain_text(&entry.definition.action.title),
                    if entry.prepared.is_err() {
                        " (unavailable)"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>();
        let width = labels
            .iter()
            .map(|label| label.width())
            .max()
            .unwrap_or(0)
            .saturating_add(4)
            .min(size.width);
        let (y, height) = menu_placement(size.height, labels.len() + 2, anchor_row);
        let count = labels.len().min(height - 2);
        let start = self.selected.saturating_add(1).saturating_sub(count);
        let rows = labels
            .iter()
            .enumerate()
            .skip(start)
            .take(count)
            .map(|(index, label)| {
                (
                    label.as_str(),
                    if index == self.selected {
                        SELECTED
                    } else {
                        BODY
                    },
                )
            })
            .collect::<Vec<_>>();
        let lines = bordered_lines(&rows, width);
        MenuScreen {
            x: 2.min(size.width - width),
            y,
            lines,
        }
    }

    fn detail_screen(&mut self, size: DashboardTerminalSize) -> MenuScreen {
        let mut details = vec![self.target.clone(), plain_text(&self.title), String::new()];
        if let Some(entry) = self.entries.get(self.selected) {
            details.push(plain_text(&entry.definition.action.title));
        }
        if self.confirming {
            details.push("Repository-local command: run this exact invocation?".to_owned());
        }
        details.extend(self.details());
        let footer = if self.confirming {
            "y: run   n/Esc: back"
        } else if self.entries.is_empty() {
            "Enter/Esc: close"
        } else {
            "Enter: run   Tab/Esc: back"
        };
        let width = details
            .iter()
            .map(|line| line.width())
            .chain([footer.width()])
            .max()
            .unwrap_or(0)
            .saturating_add(4)
            .min(size.width)
            .min(92);
        let details = details
            .iter()
            .flat_map(|line| wrap_plain(line, width - 4))
            .collect::<Vec<_>>();
        let capacity = size.height.min(28) - 4;
        self.detail_offset = self
            .detail_offset
            .min(details.len().saturating_sub(capacity));
        let mut rows = details
            .iter()
            .skip(self.detail_offset)
            .take(capacity)
            .map(|line| (line.as_str(), BODY))
            .collect::<Vec<_>>();
        let pagination = format!(
            "PgUp/PgDn  {}–{}/{}",
            self.detail_offset + 1,
            (self.detail_offset + capacity).min(details.len()),
            details.len()
        );
        if details.len() > capacity {
            rows.push((&pagination, BODY));
        }
        rows.push((footer, BODY));
        let lines = bordered_lines(&rows, width);
        MenuScreen {
            x: (size.width - width) / 2,
            y: (size.height - lines.len()) / 2,
            lines,
        }
    }

    fn details(&self) -> Vec<String> {
        let Some(entry) = self.entries.get(self.selected) else {
            return self
                .error
                .iter()
                .map(|error| format!("Cannot load actions: {}", plain_text(error)))
                .collect();
        };
        let mut lines = vec![format!(
            "Source: {} ({:?})",
            plain_text(&entry.definition.source.path.display().to_string()),
            entry.definition.source.scope
        )];
        lines.push(format!(
            "On success: {}",
            entry.definition.action.on_success.as_str()
        ));
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

/// Returns the menu's top row and height without covering its PR row.
/// Prefer below, then above; if neither fits, use the roomier side and let the list scroll.
fn menu_placement(
    screen_height: usize,
    desired_height: usize,
    anchor_row: Option<usize>,
) -> (usize, usize) {
    if screen_height == 0 {
        return (0, 0);
    }
    let Some(row) = anchor_row else {
        let height = desired_height.min(screen_height);
        return ((screen_height - height) / 2, height);
    };
    let above = row.min(screen_height - 1);
    let below = screen_height - above - 1;
    if desired_height <= below || (desired_height > above && below >= above) {
        (above + 1, desired_height.min(below))
    } else {
        let height = desired_height.min(above);
        (above - height, height)
    }
}

// Match Zellij's Acme right-click menu: pale green, dark green, and reversed bold selection.
const BODY: &str = "\x1b[0;38;2;31;91;42;48;2;228;246;211m";
const SELECTED: &str = "\x1b[0;1;38;2;228;246;211;48;2;31;91;42m";

fn bordered_lines(rows: &[(&str, &str)], width: usize) -> Vec<String> {
    let mut lines = vec![panel_line(
        &format!("┌{}┐", "─".repeat(width - 2)),
        width,
        BODY,
    )];
    lines.extend(rows.iter().map(|(label, style)| {
        format!(
            "{BODY}│{style} {} {BODY}│\x1b[0m",
            pad_plain(label, width - 4)
        )
    }));
    lines.push(panel_line(
        &format!("└{}┘", "─".repeat(width - 2)),
        width,
        BODY,
    ));
    lines
}

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
