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
    refreshes: BTreeMap<DashboardRefreshKind, RefreshStatus>,
    foreground_refresh: bool,
    foreground_requested: bool,
    error: Option<(ErrorSource, Instant, StatusMessage)>,
    transient: Option<(Instant, StatusMessage)>,
}

#[derive(Default)]
struct RefreshStatus {
    action: Option<DashboardActionInfo>,
    started: Option<Instant>,
    timed_out: bool,
}

impl DashboardStatus {
    /// Reports success immediately for no-refresh actions, otherwise waits for the list update.
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
                if let Some(kind) = DashboardRefreshKind::for_action(policy) {
                    self.refreshes.entry(kind).or_default().action = Some(action);
                    Some(policy)
                } else {
                    self.show_action_success(action, now);
                    None
                }
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
        if self
            .refreshes
            .get(&DashboardRefreshKind::Live)
            .is_some_and(|refresh| refresh.started.is_some())
        {
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
        let refresh = self.refreshes.entry(kind).or_default();
        refresh.started = Some(now);
        refresh.timed_out = false;
        if kind == DashboardRefreshKind::Live {
            self.foreground_refresh = std::mem::take(&mut self.foreground_requested) || initial;
        }
    }

    /// Reports a timeout once while retaining the worker and its action context for later recovery.
    pub(super) fn refresh_timed_out(
        &mut self,
        kind: DashboardRefreshKind,
        failure: PrActionFailure,
        environment: &RuntimeEnvironment,
        now: Instant,
    ) {
        self.refreshes.entry(kind).or_default().timed_out = true;
        self.report_refresh_error(kind, failure, environment, now);
    }

    /// Called after the replacement snapshot is rendered, not merely when fetching finishes.
    pub(super) fn refreshed(
        &mut self,
        kind: DashboardRefreshKind,
        result: Result<(), PrActionFailure>,
        environment: &RuntimeEnvironment,
        now: Instant,
    ) {
        if kind == DashboardRefreshKind::Live {
            self.foreground_refresh = false;
        }
        match result {
            Ok(()) => {
                self.clear_error(&ErrorSource::Refresh(kind));
                self.clear_error(&ErrorSource::Render);
                if let Some(action) = self
                    .refreshes
                    .remove(&kind)
                    .and_then(|refresh| refresh.action)
                {
                    self.show_action_success(action, now);
                }
            }
            Err(error) => {
                self.report_refresh_error(kind, error, environment, now);
                self.refreshes.remove(&kind);
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

    fn show_action_success(&mut self, action: DashboardActionInfo, now: Instant) {
        self.transient = Some((
            now + NOTICE_DURATION,
            StatusMessage::new(
                StatusKind::Success,
                format!("\"{}\" completed", action.title),
            ),
        ));
    }

    fn report_refresh_error(
        &mut self,
        kind: DashboardRefreshKind,
        failure: PrActionFailure,
        environment: &RuntimeEnvironment,
        now: Instant,
    ) {
        let mut message = if let Some(action) = self
            .refreshes
            .get(&kind)
            .and_then(|refresh| refresh.action.as_ref())
        {
            StatusMessage::new(
                StatusKind::Error,
                format!("\"{}\" completed; refresh failed", action.title),
            )
        } else {
            StatusMessage::new(StatusKind::Error, "Refresh failed".to_owned())
        };
        message.add_failure(failure, environment);
        self.error = Some((ErrorSource::Refresh(kind), now + ERROR_DURATION, message));
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

    /// Keeps one action timer across execution and reload; only the subprocess can be cancelled.
    fn message(
        &self,
        running: Option<&DashboardActionInfo>,
        now: Instant,
    ) -> Option<StatusMessage> {
        // Prefer the short local follow-up over a concurrent remote follow-up.
        let updating = self
            .refreshes
            .values()
            .rev()
            .filter(|refresh| !refresh.timed_out)
            .find_map(|refresh| refresh.action.as_ref());
        if let Some(action) = running.or(updating) {
            let elapsed = now.saturating_duration_since(action.started).as_secs();
            return Some(StatusMessage {
                kind: StatusKind::Working,
                body: format!("Running {}… {elapsed}s", action.title),
                hint: running.map(|_| "Esc cancel".to_owned()),
            });
        }
        if let Some(refresh) = self
            .refreshes
            .get(&DashboardRefreshKind::Live)
            .filter(|refresh| !refresh.timed_out && self.foreground_refresh)
        {
            if let Some(started) = refresh.started {
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
    Refresh(DashboardRefreshKind),
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
