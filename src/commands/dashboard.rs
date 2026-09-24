use super::*;
use chrono::{DateTime, Local, TimeZone as _};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, queue,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    env,
    ffi::OsString,
    fs,
    path::Path,
    process::Command as ProcessCommand,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

mod actions;
mod keybindings;
mod menu;
mod navigation;
mod refresh;
mod screen;
mod status;
#[path = "dashboard/terminal.rs"]
mod terminal_session;
#[cfg(test)]
#[path = "dashboard/tests/fixtures.rs"]
pub(super) mod test_support;
mod view;

use crate::repository::DashboardCommand;
use actions::DashboardActions;
use keybindings::{DashboardInput, DashboardKeyboard};
use menu::{MenuIntent, PrActionMenu};
use navigation::DashboardNavigation;
pub(super) use refresh::DashboardRefreshKind;
use refresh::{DashboardRefresh, DashboardRefreshSchedule};
use screen::{render_dashboard_frame, DashboardControls};
use status::DashboardStatus;
use terminal_session::DashboardTerminalSession;
use view::{DashboardView, DashboardViewUpdate};

const DASHBOARD_EVENT_POLL: Duration = Duration::from_millis(100);
const DASHBOARD_IDLE_POLL: Duration = Duration::from_millis(500);
const DASHBOARD_REFRESH_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) type DashboardFrameLoader =
    Arc<dyn Fn(DashboardRefreshKind) -> Result<DashboardFrameSnapshot, String> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DashboardRenderOptions {
    pub(super) color: bool,
    pub(super) terminal_width: Option<usize>,
}

pub(super) struct DashboardFrameSnapshot {
    renderer:
        Box<dyn Fn(DashboardRenderOptions) -> Result<PullRequestTableFrame, String> + Send + Sync>,
}

impl DashboardFrameSnapshot {
    pub(super) fn new(
        renderer: impl Fn(DashboardRenderOptions) -> Result<PullRequestTableFrame, String>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            renderer: Box::new(renderer),
        }
    }

    pub(super) fn render(
        &self,
        options: DashboardRenderOptions,
    ) -> Result<PullRequestTableFrame, String> {
        (self.renderer)(options)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DashboardTerminalSize {
    width: usize,
    height: usize,
}

impl DashboardTerminalSize {
    fn new(width: u16, height: u16) -> Self {
        Self {
            width: usize::from(width),
            height: usize::from(height),
        }
    }

    fn render_options(self) -> DashboardRenderOptions {
        DashboardRenderOptions {
            color: true,
            terminal_width: Some(self.width),
        }
    }
}

/// Runs a live dashboard with stable PR selection, quiet actions, and short-lived notices.
pub(super) fn run_interactive_dashboard(
    refresh_seconds: u64,
    loader: DashboardFrameLoader,
    environment: &RuntimeEnvironment,
    action_set: pr_actions::PrActionSet,
) -> Result<CommandResult, CommandError> {
    let bindings = WorkflowConfig::discover_global(environment)?
        .ui
        .dashboard_keys;
    let mut keyboard = DashboardKeyboard::new(bindings);
    let interrupts = DashboardInterrupts::enter()?;
    let mut terminal = DashboardTerminalSession::enter()?;
    let mut terminal_size = dashboard_terminal_size()?;
    let mut watcher = ExecutableWatcher::from_process();
    let mut view = DashboardView::default();
    let mut navigation = DashboardNavigation::default();
    let mut menu = None::<PrActionMenu>;
    let mut actions = DashboardActions::default();
    let mut refresh = None::<DashboardRefresh>;
    let mut schedule = DashboardRefreshSchedule::default();
    let mut status = DashboardStatus::default();

    loop {
        if interrupts.take_pending() {
            status.clear_notice();
            if !actions.cancel() {
                return Ok(CommandResult::with_exit_code(String::new(), 130));
            }
            view.pending = None;
        }
        if let Some(completion) = actions.poll() {
            if let Some(policy) = status.action_completed(completion, environment, Instant::now()) {
                view.pending = None;
                schedule.after_action(policy);
            }
        }
        status.tick(Instant::now());
        if menu.is_none()
            && !keyboard.help_open()
            && !actions.is_busy()
            && !status.has_error()
            && watcher.changed()
        {
            terminal.restore()?;
            return restart_dashboard_process();
        }
        if refresh.is_none()
            && !actions.is_running()
            && view.pending.is_none()
            && menu.is_none()
            && !keyboard.help_open()
        {
            if let Some(kind) = schedule.next(Local::now()) {
                actions.refresh_started(kind);
                status.refresh_started(kind, view.frame.is_none(), Instant::now());
                refresh = Some(DashboardRefresh::start(Arc::clone(&loader), kind));
            }
        }
        if let Some(loading) = &mut refresh {
            if let Some(result) = loading.poll() {
                view.pending = Some(result);
                schedule.loaded(loading.kind, Local::now(), refresh_seconds);
                refresh = None;
            } else if !loading.timed_out && dashboard_refresh_timed_out(loading.started.elapsed()) {
                loading.timed_out = true;
                let failure = actions.refresh_timed_out(dashboard_refresh_timeout_error());
                status.refresh_timed_out(failure, environment, Instant::now());
                // Retain the worker: abandoning it could race an action or a new load.
            }
        }
        if let Some(update) = view.update(menu.is_some() || keyboard.help_open(), terminal_size) {
            match update {
                DashboardViewUpdate::Loaded(result) => {
                    status.refreshed(actions.refreshed(result), environment, Instant::now())
                }
                DashboardViewUpdate::Reflowed(result) => status.reflowed(result, Instant::now()),
            }
        }
        navigation.reconcile(view.frame.as_ref());
        let content_size = render_dashboard_frame(
            view.frame.as_ref(),
            terminal_size,
            &mut navigation,
            DashboardControls {
                menu: menu.as_mut(),
                keyboard: &mut keyboard,
            },
            &status,
            actions.running_info(),
        )?;
        let timeout = if refresh.is_some() || actions.is_running() {
            DASHBOARD_EVENT_POLL
        } else {
            DASHBOARD_IDLE_POLL
        };
        match read_dashboard_event(timeout, &mut terminal_size)? {
            DashboardEvent::Interrupt => {
                status.clear_notice();
                if actions.cancel() {
                    view.pending = None;
                } else {
                    return Ok(CommandResult::with_exit_code(String::new(), 130));
                }
            }
            DashboardEvent::Resized => {}
            DashboardEvent::Key(key) => {
                status.clear_notice();
                if let Some(open_menu) = &mut menu {
                    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
                        continue;
                    }
                    match open_menu.handle_key(key, refresh.is_some(), content_size) {
                        MenuIntent::None => {}
                        MenuIntent::Close => menu = None,
                        MenuIntent::Run(action) => {
                            menu = None;
                            // Finish any queued list update before attributing work to the next action.
                            if let Some(update) = view.update(false, terminal_size) {
                                match update {
                                    DashboardViewUpdate::Loaded(result) => status.refreshed(
                                        actions.refreshed(result),
                                        environment,
                                        Instant::now(),
                                    ),
                                    DashboardViewUpdate::Reflowed(result) => {
                                        status.reflowed(result, Instant::now())
                                    }
                                }
                            }
                            actions.start(action, environment, action_set);
                        }
                    }
                } else {
                    match keyboard.handle_key(key) {
                        DashboardInput::Cancel => {
                            if actions.cancel() {
                                view.pending = None;
                            }
                        }
                        DashboardInput::Command(DashboardCommand::Quit) => {
                            return Ok(CommandResult::success(String::new()));
                        }
                        DashboardInput::Command(DashboardCommand::Menu) if !actions.is_busy() => {
                            if let Some(context) = view
                                .frame
                                .as_ref()
                                .and_then(|frame| navigation.selected(frame))
                            {
                                let entries = pr_actions::load_pr_actions(
                                    context.clone(),
                                    environment,
                                    action_set,
                                )
                                .map_err(|error| error.to_string());
                                menu = Some(PrActionMenu::new(context, entries));
                            }
                        }
                        DashboardInput::Command(DashboardCommand::Refresh) => {
                            status.request_refresh();
                            if refresh.as_ref().map(|loading| loading.kind)
                                != Some(DashboardRefreshKind::Live)
                            {
                                schedule.request_live();
                            }
                        }
                        DashboardInput::Command(command) if command.is_navigation() => {
                            if let Some(frame) = &view.frame {
                                navigation.handle_command(command, frame, content_size.height);
                            }
                        }
                        _ => {}
                    }
                }
            }
            DashboardEvent::None => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardEvent {
    None,
    Resized,
    Key(KeyEvent),
    Interrupt,
}

fn read_dashboard_event(
    timeout: Duration,
    terminal_size: &mut DashboardTerminalSize,
) -> io::Result<DashboardEvent> {
    if !event::poll(timeout)? {
        return Ok(DashboardEvent::None);
    }

    match event::read()? {
        Event::Resize(width, height) => {
            *terminal_size = DashboardTerminalSize::new(width, height);
            Ok(DashboardEvent::Resized)
        }
        Event::Key(key) if dashboard_interrupt_key(key) => Ok(DashboardEvent::Interrupt),
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            Ok(DashboardEvent::Key(key))
        }
        _ => Ok(DashboardEvent::None),
    }
}

fn dashboard_interrupt_key(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn dashboard_terminal_size() -> io::Result<DashboardTerminalSize> {
    let (width, height) = terminal::size()?;
    Ok(DashboardTerminalSize::new(width, height))
}

fn next_dashboard_refresh_time(
    refreshed_at: DateTime<Local>,
    refresh_seconds: u64,
) -> Option<DateTime<Local>> {
    let interval = i64::try_from(refresh_seconds).ok()?;
    if interval <= 0 {
        return None;
    }
    let current = refreshed_at.timestamp();
    let next = current.checked_add(interval - current.rem_euclid(interval))?;
    Local.timestamp_opt(next, 0).single()
}

fn dashboard_refresh_timed_out(elapsed: Duration) -> bool {
    elapsed >= DASHBOARD_REFRESH_TIMEOUT
}

fn dashboard_refresh_timeout_error() -> String {
    format!(
        "dashboard refresh timed out after {}s; keeping the previous frame",
        DASHBOARD_REFRESH_TIMEOUT.as_secs()
    )
}

/// Returns the next short sleep before a dashboard refresh is due.
fn dashboard_wait_duration(
    now: DateTime<Local>,
    next_refresh_at: Option<DateTime<Local>>,
) -> Option<Duration> {
    let next_refresh_at = next_refresh_at?;
    let remaining = next_refresh_at.signed_duration_since(now).to_std().ok()?;
    if remaining.is_zero() {
        return None;
    }
    Some(remaining.min(DASHBOARD_IDLE_POLL))
}

struct ExecutableWatcher {
    path: Option<PathBuf>,
    initial: Option<FileStamp>,
}

impl ExecutableWatcher {
    fn from_process() -> Self {
        let path = restart_executable_path();
        let initial = path.as_deref().and_then(FileStamp::read);
        Self { path, initial }
    }

    fn changed(&mut self) -> bool {
        let Some(path) = self.path.as_deref() else {
            return false;
        };
        match (&self.initial, FileStamp::read(path)) {
            (Some(initial), Some(current)) => &current != initial,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

impl FileStamp {
    fn read(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            dev: std::os::unix::fs::MetadataExt::dev(&metadata),
            #[cfg(unix)]
            ino: std::os::unix::fs::MetadataExt::ino(&metadata),
        })
    }
}

fn restart_executable_path() -> Option<PathBuf> {
    env::args_os()
        .next()
        .and_then(resolve_invoked_executable)
        .or_else(|| env::current_exe().ok())
}

fn resolve_invoked_executable(value: OsString) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    if path.components().count() > 1 {
        return Some(path);
    }
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|directory| directory.join(&path))
        .find(|candidate| candidate.is_file())
}

fn restart_dashboard_process() -> Result<CommandResult, CommandError> {
    let argv = env::args_os().collect::<Vec<_>>();
    let Some(executable) = restart_executable_path() else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "jx executable was not found").into());
    };
    restart_process(&executable, &argv)
}

#[cfg(unix)]
fn restart_process(executable: &Path, argv: &[OsString]) -> Result<CommandResult, CommandError> {
    use std::os::unix::process::CommandExt;

    let mut command = ProcessCommand::new(executable);
    command.args(argv.iter().skip(1));
    Err(command.exec().into())
}

#[cfg(not(unix))]
fn restart_process(executable: &Path, argv: &[OsString]) -> Result<CommandResult, CommandError> {
    ProcessCommand::new(executable)
        .args(argv.iter().skip(1))
        .spawn()?;
    std::process::exit(0);
}

pub(super) struct SilentProgress;

impl ProgressSink for SilentProgress {
    fn status(&self, _message: &str) {}

    fn finish(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_wait_duration_treats_due_or_overdue_refreshes_as_ready() {
        let now = local_test_time();

        assert_eq!(dashboard_wait_duration(now, Some(now)), None);
        assert_eq!(
            dashboard_wait_duration(now, Some(now - chrono::Duration::seconds(30))),
            None
        );
    }

    #[test]
    fn next_dashboard_refresh_time_aligns_to_wall_clock_interval_marks() {
        let refreshed_at = local_test_time_at(12, 2, 15);

        assert_eq!(
            next_dashboard_refresh_time(refreshed_at, 300),
            Some(local_test_time_at(12, 5, 0))
        );
        assert_eq!(
            next_dashboard_refresh_time(local_test_time_at(12, 5, 0), 300),
            Some(local_test_time_at(12, 10, 0))
        );
    }

    #[test]
    fn dashboard_refresh_timeout_keeps_slow_workers_from_spinning_forever() {
        assert!(!dashboard_refresh_timed_out(
            DASHBOARD_REFRESH_TIMEOUT - Duration::from_millis(1)
        ));
        assert!(dashboard_refresh_timed_out(DASHBOARD_REFRESH_TIMEOUT));
        assert!(dashboard_refresh_timeout_error().contains("keeping the previous frame"));
    }

    #[test]
    fn dashboard_wait_duration_sleeps_in_short_wall_clock_chunks() {
        let now = local_test_time();

        assert_eq!(
            dashboard_wait_duration(now, Some(now + chrono::Duration::seconds(30))),
            Some(DASHBOARD_IDLE_POLL)
        );
        assert_eq!(
            dashboard_wait_duration(now, Some(now + chrono::Duration::milliseconds(100))),
            Some(Duration::from_millis(100))
        );
    }

    fn local_test_time() -> DateTime<Local> {
        local_test_time_at(12, 0, 0)
    }

    fn local_test_time_at(hour: u32, minute: u32, second: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 1, 15, hour, minute, second)
            .single()
            .expect("test time is unambiguous in the local timezone")
    }
}
