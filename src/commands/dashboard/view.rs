use super::*;

/// Owns the visible PR snapshot and holds refresh results until the action menu closes.
#[derive(Default)]
pub(super) struct DashboardView {
    snapshot: Option<DashboardFrameSnapshot>,
    pub(super) frame: Option<PullRequestTableFrame>,
    pub(super) error: Option<String>,
    pub(super) pending: Option<Result<DashboardFrameSnapshot, String>>,
    rendered_size: Option<DashboardTerminalSize>,
}

impl DashboardView {
    /// Freezes data, errors, and reflow while the menu is open; applies queued changes on close.
    pub(super) fn update(
        &mut self,
        menu_open: bool,
        refresh_timed_out: bool,
        size: DashboardTerminalSize,
    ) {
        if menu_open {
            return;
        }
        if refresh_timed_out {
            self.error = Some(dashboard_refresh_timeout_error());
        }
        if let Some(result) = self.pending.take() {
            match result.and_then(|snapshot| {
                snapshot
                    .render(size.render_options())
                    .map(|frame| (snapshot, frame))
            }) {
                Ok((snapshot, frame)) => {
                    self.snapshot = Some(snapshot);
                    self.frame = Some(frame);
                    self.error = None;
                    self.rendered_size = Some(size);
                }
                Err(error) => self.error = Some(error),
            }
        }
        if self.rendered_size != Some(size) {
            self.rendered_size = Some(size);
            if let Some(snapshot) = &self.snapshot {
                match snapshot.render(size.render_options()) {
                    Ok(frame) => self.frame = Some(frame),
                    Err(error) => self.error = Some(error),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/view.rs"]
mod tests;
