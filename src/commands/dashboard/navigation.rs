use super::*;
use crate::domain::{PrActionContext, PrActionKey};

/// Selection follows a PR, with checkout identity distinguishing multiple clones of the same PR.
#[derive(Default)]
pub(super) struct DashboardNavigation {
    target: Option<(PrActionKey, Option<PathBuf>)>,
    index: usize,
    pub(super) selected_line: Option<usize>,
    pub(super) scroll_top: usize,
}

impl DashboardNavigation {
    pub(super) fn reconcile(&mut self, frame: Option<&PullRequestTableFrame>) {
        let Some(frame) = frame.filter(|frame| !frame.rows.is_empty()) else {
            *self = Self::default();
            return;
        };
        let retained = self.target.as_ref().and_then(|(key, root)| {
            frame
                .rows
                .iter()
                .position(|row| row.context.key() == *key && row.context.repository_root == *root)
        });
        self.select(
            frame,
            retained.unwrap_or(self.index).min(frame.rows.len() - 1),
        );
    }

    pub(super) fn selected<'a>(
        &self,
        frame: &'a PullRequestTableFrame,
    ) -> Option<&'a PrActionContext> {
        frame.rows.get(self.index).map(|row| &row.context)
    }

    pub(super) fn handle_key(
        &mut self,
        key: KeyCode,
        frame: &PullRequestTableFrame,
        height: usize,
    ) {
        if frame.rows.is_empty() {
            return;
        }
        let last = frame.rows.len() - 1;
        let index = match key {
            KeyCode::Up | KeyCode::Char('k') => self.index.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.index.saturating_add(1).min(last),
            KeyCode::PageUp => self.index.saturating_sub(height.max(1)),
            KeyCode::PageDown => self.index.saturating_add(height.max(1)).min(last),
            KeyCode::Home | KeyCode::Char('g') => 0,
            KeyCode::End | KeyCode::Char('G') => last,
            _ => return,
        };
        self.select(frame, index);
    }

    fn select(&mut self, frame: &PullRequestTableFrame, index: usize) {
        let row = &frame.rows[index];
        self.index = index;
        self.target = Some((row.context.key(), row.context.repository_root.clone()));
        self.selected_line = Some(row.line);
    }

    /// Keeps the selected logical line visible without removing headers or altering row text.
    pub(super) fn viewport(
        &mut self,
        output: &str,
        prefix_lines: usize,
        height: usize,
    ) -> (String, Option<usize>) {
        let lines = output.lines().collect::<Vec<_>>();
        let selected = self.selected_line.map(|line| line + prefix_lines);
        if let Some(line) = selected {
            if line < self.scroll_top {
                self.scroll_top = line;
            }
            if line >= self.scroll_top.saturating_add(height) {
                self.scroll_top = line.saturating_add(1).saturating_sub(height);
            }
        }
        self.scroll_top = self.scroll_top.min(lines.len().saturating_sub(height));
        let marker = selected
            .and_then(|line| line.checked_sub(self.scroll_top))
            .filter(|line| *line < height);
        let output = lines
            .into_iter()
            .skip(self.scroll_top)
            .take(height)
            .collect::<Vec<_>>()
            .join("\n");
        (output, marker)
    }
}

#[cfg(test)]
#[path = "tests/navigation.rs"]
mod tests;
