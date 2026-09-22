use super::actions::{DashboardActionCompletion, DashboardActionInfo, DashboardActionOutcome};
use super::*;
use crate::{
    commands::{handlers::display_path, pr_actions::PrActionFailure},
    domain::PrActionKey,
    repository::PrActionOnSuccess,
};
use std::time::Instant;

/// Keeps ongoing activity visible while notices expire or yield to keyboard interaction.
#[derive(Default)]
pub(super) struct DashboardStatus {
    updating_action: Option<DashboardActionInfo>,
    refresh_started: Option<(DashboardRefreshKind, Instant)>,
    foreground_refresh: bool,
    foreground_requested: bool,
    timed_out: bool,
    error: Option<(ErrorSource, Instant, StatusMessage)>,
    transient: Option<(Instant, StatusMessage)>,
}

impl DashboardStatus {
    /// Starts the list-update phase only after the command has successfully completed.
    pub(super) fn action_completed(
        &mut self,
        completion: DashboardActionCompletion,
        environment: &RuntimeEnvironment,
        now: Instant,
    ) -> Option<PrActionOnSuccess> {
        let DashboardActionCompletion { action, outcome } = completion;
        let source = ErrorSource::Action(action.id.clone(), action.target.clone());
        self.transient = None;
        match outcome {
            DashboardActionOutcome::Succeeded(policy) => {
                self.clear_error(&source);
                self.updating_action = Some(action);
                Some(policy)
            }
            DashboardActionOutcome::Failed(failure) => {
                let mut message =
                    StatusMessage::new(StatusKind::Error, format!("\"{}\" failed", action.title));
                message.add_failure(failure, environment);
                self.error = Some((source, now + ERROR_DURATION, message));
                None
            }
            DashboardActionOutcome::Cancelled(failure) => {
                let mut message = StatusMessage::new(
                    StatusKind::Warning,
                    format!("\"{}\" cancelled", action.title),
                );
                message.add_failure(failure, environment);
                self.transient = Some((now + NOTICE_DURATION, message));
                None
            }
        }
    }

    pub(super) fn request_refresh(&mut self) {
        if matches!(self.refresh_started, Some((DashboardRefreshKind::Live, _))) {
            self.foreground_refresh = true;
        } else {
            self.foreground_requested = true;
        }
    }

    /// Shows user-requested live loads, never the implementation-only cached rebuild.
    pub(super) fn refresh_started(
        &mut self,
        kind: DashboardRefreshKind,
        initial: bool,
        now: Instant,
    ) {
        self.refresh_started = Some((kind, now));
        self.foreground_refresh = kind == DashboardRefreshKind::Live
            && (std::mem::take(&mut self.foreground_requested)
                || initial
                || self.updating_action.is_some());
        self.timed_out = false;
    }

    /// Reports a timeout once while retaining the worker and its action context for later recovery.
    pub(super) fn refresh_timed_out(&mut self, now: Instant) {
        self.timed_out = true;
        self.report_refresh_error(dashboard_refresh_timeout_error(), now);
    }

    /// Called after the replacement snapshot is rendered, not merely when fetching finishes.
    pub(super) fn refreshed(&mut self, result: Result<(), String>, now: Instant) {
        self.refresh_started = None;
        self.foreground_refresh = false;
        self.timed_out = false;
        match result {
            Ok(()) => {
                self.clear_error(&ErrorSource::Refresh);
                self.clear_error(&ErrorSource::Render);
                if let Some(action) = self.updating_action.take() {
                    self.transient = Some((
                        now + NOTICE_DURATION,
                        StatusMessage::new(
                            StatusKind::Success,
                            format!("\"{}\" completed", action.title),
                        ),
                    ));
                }
            }
            Err(error) => {
                self.report_refresh_error(error, now);
                self.updating_action = None;
            }
        }
    }

    pub(super) fn reflowed(&mut self, result: Result<(), String>, now: Instant) {
        match result {
            Ok(()) => self.clear_error(&ErrorSource::Render),
            Err(error) => {
                self.error = Some((
                    ErrorSource::Render,
                    now + ERROR_DURATION,
                    StatusMessage::new(StatusKind::Error, format!("Cannot redraw list: {error}")),
                ))
            }
        }
    }

    pub(super) fn tick(&mut self, now: Instant) {
        if self
            .error
            .as_ref()
            .is_some_and(|(_, until, _)| now >= *until)
        {
            self.error = None;
        }
        if self
            .transient
            .as_ref()
            .is_some_and(|(until, _)| now >= *until)
        {
            self.transient = None;
        }
    }

    pub(super) fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Clears notices on interaction without consuming the key or hiding ongoing work.
    pub(super) fn clear_notice(&mut self) {
        self.error = None;
        self.transient = None;
    }

    pub(super) fn line(
        &self,
        running: Option<&DashboardActionInfo>,
        now: Instant,
        width: usize,
    ) -> Option<String> {
        self.message(running, now)
            .map(|message| message.render(width))
    }

    fn report_refresh_error(&mut self, error: String, now: Instant) {
        let message = if let Some(action) = &self.updating_action {
            StatusMessage::new(
                StatusKind::Error,
                format!("\"{}\" completed; refresh failed: {error}", action.title),
            )
        } else {
            StatusMessage::new(StatusKind::Error, format!("Refresh failed: {error}"))
        };
        self.error = Some((ErrorSource::Refresh, now + ERROR_DURATION, message));
    }

    fn clear_error(&mut self, source: &ErrorSource) {
        if self
            .error
            .as_ref()
            .is_some_and(|(current, _, _)| current == source)
        {
            self.error = None;
        }
    }

    fn message(
        &self,
        running: Option<&DashboardActionInfo>,
        now: Instant,
    ) -> Option<StatusMessage> {
        if let Some(action) = running {
            let elapsed = now.saturating_duration_since(action.started).as_secs();
            return Some(StatusMessage {
                kind: StatusKind::Working,
                body: format!("Running {}… {elapsed}s", action.title),
                hint: Some("Esc cancel".to_owned()),
            });
        }
        if !self.timed_out {
            if let Some((_, started)) = self.refresh_started.filter(|_| self.foreground_refresh) {
                let elapsed = now.saturating_duration_since(started).as_secs();
                return Some(StatusMessage::new(
                    StatusKind::Working,
                    format!("Refreshing pull requests… {elapsed}s"),
                ));
            }
        }
        self.error
            .as_ref()
            .map(|(_, _, message)| message.clone())
            .or_else(|| self.transient.as_ref().map(|(_, message)| message.clone()))
    }
}

const NOTICE_DURATION: Duration = Duration::from_secs(3);
const ERROR_DURATION: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq, Eq)]
enum ErrorSource {
    Refresh,
    Render,
    Action(String, PrActionKey),
}

#[derive(Clone, Copy)]
enum StatusKind {
    Working,
    Success,
    Warning,
    Error,
}

#[derive(Clone)]
struct StatusMessage {
    kind: StatusKind,
    body: String,
    hint: Option<String>,
}

impl StatusMessage {
    fn new(kind: StatusKind, body: String) -> Self {
        Self {
            kind,
            body,
            hint: None,
        }
    }

    /// Directs logged failures to their actual path and reports logging failures inline.
    fn add_failure(&mut self, failure: PrActionFailure, environment: &RuntimeEnvironment) {
        if let Some(path) = failure.log_path {
            self.hint = Some(format!("see {}", display_path(&path, environment)));
        } else {
            self.body.push_str(&format!(": {}", failure.message));
        }
    }

    /// Paints every cell explicitly, including right padding; the screen writer disables autowrap.
    fn render(&self, width: usize) -> String {
        let style = match self.kind {
            StatusKind::Working | StatusKind::Success | StatusKind::Warning => {
                "\x1b[0;48;2;236;233;219m\x1b[38;2;0;0;0m"
            }
            StatusKind::Error => "\x1b[0;48;2;236;233;219m\x1b[38;2;192;48;40m",
        };
        let hint = self
            .hint
            .as_deref()
            .map(menu::plain_text)
            .unwrap_or_default();
        let row_width = width;
        let width = width.saturating_sub(1);
        let hint_width = rendered_visible_width(&hint) + 2;
        let show_hint = !hint.is_empty() && width > hint_width + 20;
        let body_width = if show_hint { width - hint_width } else { width };
        let text = format!(" {}", menu::plain_text(&self.body));
        let mut text = ellipsize_rendered_line(&text, Some(body_width));
        if show_hint {
            if matches!(self.kind, StatusKind::Working) {
                text.push_str(&" ".repeat(width.saturating_sub(
                    rendered_visible_width(&text) + rendered_visible_width(&hint),
                )));
            } else {
                text.push_str(", ");
            }
            text.push_str(&hint);
        }
        text.push_str(&" ".repeat(row_width.saturating_sub(rendered_visible_width(&text))));
        format!("{style}{text}\x1b[0m")
    }
}

#[cfg(test)]
#[path = "tests/status.rs"]
mod tests;
