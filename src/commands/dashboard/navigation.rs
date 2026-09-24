use super::*;
use crate::domain::{PrActionContext, PrActionKey};
use crate::repository::DashboardCommand;

/// Selection follows a PR, with checkout identity distinguishing multiple clones of the same PR.
#[derive(Default)]
pub(super) struct DashboardNavigation {
    target: Option<(PrActionKey, Option<PathBuf>)>,
    index: usize,
    repository_index: usize,
    pub(super) selected_line: Option<usize>,
    pub(super) scroll_top: usize,
}

impl DashboardNavigation {
    /// Falls back within the selected repository and checkout before moving to another group.
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
        let index = retained
            .or_else(|| self.repository_fallback_index(frame))
            .unwrap_or(self.index)
            .min(frame.rows.len() - 1);
        self.select(frame, index);
    }

    pub(super) fn selected<'a>(
        &self,
        frame: &'a PullRequestTableFrame,
    ) -> Option<&'a PrActionContext> {
        frame.rows.get(self.index).map(|row| &row.context)
    }

    /// Applies resolved navigation without knowing which key or sequence invoked it.
    pub(super) fn handle_command(
        &mut self,
        command: DashboardCommand,
        frame: &PullRequestTableFrame,
        height: usize,
    ) {
        if frame.rows.is_empty() {
            return;
        }
        let last = frame.rows.len() - 1;
        let index = match command {
            DashboardCommand::Up => self.index.saturating_sub(1),
            DashboardCommand::Down => self.index.saturating_add(1).min(last),
            DashboardCommand::PageUp => self.index.saturating_sub(height.max(1)),
            DashboardCommand::PageDown => self.index.saturating_add(height.max(1)).min(last),
            DashboardCommand::First => 0,
            DashboardCommand::Last => last,
            _ => return,
        };
        self.select(frame, index);
    }

    /// Retains both table-wide and repository-local positions for the next refresh.
    fn select(&mut self, frame: &PullRequestTableFrame, index: usize) {
        let row = &frame.rows[index];
        self.index = index;
        self.repository_index = frame.rows[..index]
            .iter()
            .filter(|candidate| {
                candidate.context.repository == row.context.repository
                    && candidate.context.repository_root == row.context.repository_root
            })
            .count();
        self.target = Some((row.context.key(), row.context.repository_root.clone()));
        self.selected_line = Some(row.line);
    }

    /// Keeps the group's ordinal, clamping to its last row when the selected tail disappears.
    fn repository_fallback_index(&self, frame: &PullRequestTableFrame) -> Option<usize> {
        let (key, root) = self.target.as_ref()?;
        frame
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                row.context.repository.slug() == key.repository
                    && row.context.repository_root == *root
            })
            .take(self.repository_index.saturating_add(1))
            .last()
            .map(|(index, _)| index)
    }

    /// Keeps the selected logical line visible without removing headers or altering row text.
    pub(super) fn viewport(&mut self, output: &str, height: usize) -> (String, Option<usize>) {
        let lines = output.lines().collect::<Vec<_>>();
        let selected = self.selected_line;
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
