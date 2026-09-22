use super::*;

/// Owns the visible PR snapshot and holds refresh results until the action menu closes.
#[derive(Default)]
pub(super) struct DashboardView {
    snapshot: Option<DashboardFrameSnapshot>,
    pub(super) frame: Option<PullRequestTableFrame>,
    pub(super) pending: Option<Result<DashboardFrameSnapshot, String>>,
    rendered_size: Option<DashboardTerminalSize>,
}

impl DashboardView {
    /// Freezes data while a menu is open and emits results once, leaving notice lifetime to status.
    pub(super) fn update(
        &mut self,
        menu_open: bool,
        size: DashboardTerminalSize,
    ) -> Option<DashboardViewUpdate> {
        if menu_open {
            return None;
        }
        if let Some(result) = self.pending.take() {
            let result = result.and_then(|snapshot| {
                let frame = snapshot.render(size.render_options())?;
                self.snapshot = Some(snapshot);
                self.frame = Some(frame);
                self.rendered_size = Some(size);
                Ok(())
            });
            return Some(DashboardViewUpdate::Loaded(result));
        }
        if self.rendered_size != Some(size) {
            self.rendered_size = Some(size);
            if let Some(snapshot) = &self.snapshot {
                let result = snapshot.render(size.render_options()).map(|frame| {
                    self.frame = Some(frame);
                });
                return Some(DashboardViewUpdate::Reflowed(result));
            }
        }
        None
    }
}

pub(super) enum DashboardViewUpdate {
    Loaded(Result<(), String>),
    Reflowed(Result<(), String>),
}

#[cfg(test)]
#[path = "tests/view.rs"]
mod tests;
