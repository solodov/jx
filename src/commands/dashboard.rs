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
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant, SystemTime},
};

mod actions;
mod menu;
mod navigation;
#[path = "dashboard/terminal.rs"]
mod terminal_session;
#[cfg(test)]
#[path = "dashboard/tests/fixtures.rs"]
pub(super) mod test_support;
mod view;

use actions::DashboardActions;
use menu::{MenuIntent, PrActionMenu};
use navigation::DashboardNavigation;
use terminal_session::DashboardTerminalSession;
use view::DashboardView;

const DASHBOARD_EVENT_POLL: Duration = Duration::from_millis(100);
const DASHBOARD_IDLE_POLL: Duration = Duration::from_millis(500);
const DASHBOARD_REFRESH_TIMEOUT: Duration = Duration::from_secs(120);
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(super) type DashboardFrameLoader =
    Arc<dyn Fn() -> Result<DashboardFrameSnapshot, String> + Send + Sync>;

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

    fn render(&self, options: DashboardRenderOptions) -> Result<PullRequestTableFrame, String> {
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

/// Runs a live dashboard with stable PR selection, quiet actions, and persistent failure notices.
pub(super) fn run_interactive_dashboard(
    refresh_seconds: u64,
    loader: DashboardFrameLoader,
    environment: &RuntimeEnvironment,
    action_set: pr_actions::PrActionSet,
) -> Result<CommandResult, CommandError> {
    let interrupts = DashboardInterrupts::enter()?;
    let mut terminal = DashboardTerminalSession::enter()?;
    let mut terminal_size = dashboard_terminal_size()?;
    let mut watcher = ExecutableWatcher::from_process();
    let mut view = DashboardView::default();
    let mut navigation = DashboardNavigation::default();
    let mut menu = None::<PrActionMenu>;
    let mut actions = DashboardActions::default();
    let mut refresh = None::<DashboardRefresh>;
    let mut next_refresh_at = None;
    let mut spinner_index = 0usize;

    loop {
        if interrupts.take_pending() {
            if !actions.cancel() {
                return Ok(CommandResult::with_exit_code(String::new(), 130));
            }
            view.pending = None;
            next_refresh_at = None;
        }
        if actions.poll() {
            view.pending = None;
            next_refresh_at = None;
        }
        if menu.is_none() && !actions.is_running() && actions.failure.is_none() && watcher.changed()
        {
            terminal.restore()?;
            return restart_dashboard_process();
        }
        if refresh.is_none()
            && !actions.is_running()
            && view.pending.is_none()
            && menu.is_none()
            && dashboard_wait_duration(Local::now(), next_refresh_at).is_none()
        {
            refresh = Some(DashboardRefresh::start(Arc::clone(&loader)));
        }
        if let Some(loading) = &mut refresh {
            if let Some(result) = loading.poll() {
                view.pending = Some(result);
                refresh = None;
                next_refresh_at = next_dashboard_refresh_time(Local::now(), refresh_seconds);
            } else if !loading.timed_out && dashboard_refresh_timed_out(loading.started.elapsed()) {
                loading.timed_out = true;
                // Retain the worker: abandoning it could race an action or a new load.
            }
        }
        view.update(
            menu.is_some(),
            refresh.as_ref().is_some_and(|refresh| refresh.timed_out),
            terminal_size,
        );
        navigation.reconcile(view.frame.as_ref());
        render_dashboard_frame(
            dashboard_frame_state(
                view.frame.as_ref().map(|frame| frame.text.as_str()),
                refresh.as_ref().is_some_and(|refresh| !refresh.timed_out),
                view.error.as_deref(),
                spinner_index,
            ),
            terminal_size,
            &mut navigation,
            menu.as_mut(),
            actions.failure.as_ref(),
        )?;
        let timeout = if refresh.is_some() || actions.is_running() {
            DASHBOARD_EVENT_POLL
        } else {
            DASHBOARD_IDLE_POLL
        };
        match read_dashboard_event(timeout, &mut terminal_size)? {
            DashboardEvent::Interrupt => {
                if actions.cancel() {
                    view.pending = None;
                    next_refresh_at = None;
                } else {
                    return Ok(CommandResult::with_exit_code(String::new(), 130));
                }
            }
            DashboardEvent::Resized => {}
            DashboardEvent::Key(key) => {
                if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
                    continue;
                }
                if actions.handle_failure_key(key) {
                    continue;
                }
                if key.code == KeyCode::Esc && key.kind == KeyEventKind::Press && actions.cancel() {
                    view.pending = None;
                    next_refresh_at = None;
                    continue;
                }
                if let Some(open_menu) = &mut menu {
                    match open_menu.handle_key(key, refresh.is_some(), terminal_size) {
                        MenuIntent::None => {}
                        MenuIntent::Close => menu = None,
                        MenuIntent::Run(action) => {
                            menu = None;
                            actions.start(action, environment, action_set);
                            view.pending = None;
                            next_refresh_at = None;
                        }
                    }
                } else if dashboard_exit_key(key) {
                    return Ok(CommandResult::success(String::new()));
                } else if key.code == KeyCode::Enter
                    && key.kind == KeyEventKind::Press
                    && !actions.is_running()
                {
                    if let Some(context) = view
                        .frame
                        .as_ref()
                        .and_then(|frame| navigation.selected(frame))
                    {
                        let entries =
                            pr_actions::load_pr_actions(context.clone(), environment, action_set)
                                .map_err(|error| error.to_string());
                        menu = Some(PrActionMenu::new(context, entries));
                    }
                } else if key.code == KeyCode::Char('r') && key.kind == KeyEventKind::Press {
                    next_refresh_at = None;
                } else if let Some(frame) = &view.frame {
                    navigation.handle_key(key.code, frame, terminal_size.height);
                }
            }
            DashboardEvent::None => spinner_index = spinner_index.wrapping_add(1),
        }
    }
}

struct DashboardRefresh {
    receiver: mpsc::Receiver<Result<DashboardFrameSnapshot, String>>,
    started: Instant,
    timed_out: bool,
}

impl DashboardRefresh {
    fn start(loader: DashboardFrameLoader) -> Self {
        Self {
            receiver: spawn_dashboard_load(loader),
            started: Instant::now(),
            timed_out: false,
        }
    }

    fn poll(&self) -> Option<Result<DashboardFrameSnapshot, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "dashboard refresh worker stopped unexpectedly".to_owned(),
            )),
        }
    }
}

fn spawn_dashboard_load(
    loader: DashboardFrameLoader,
) -> mpsc::Receiver<Result<DashboardFrameSnapshot, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(loader());
    });
    receiver
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

fn dashboard_exit_key(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q')
        )
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

struct DashboardFrameState<'a> {
    frame: Option<&'a str>,
    refreshing: bool,
    error: Option<&'a str>,
    spinner_index: usize,
}

fn dashboard_frame_state<'a>(
    frame: Option<&'a str>,
    refreshing: bool,
    error: Option<&'a str>,
    spinner_index: usize,
) -> DashboardFrameState<'a> {
    DashboardFrameState {
        frame,
        refreshing,
        error,
        spinner_index,
    }
}

fn render_dashboard_frame(
    state: DashboardFrameState<'_>,
    terminal_size: DashboardTerminalSize,
    navigation: &mut DashboardNavigation,
    menu: Option<&mut PrActionMenu>,
    failure: Option<&pr_actions::PrActionFailure>,
) -> io::Result<()> {
    let prefix_lines = dashboard_frame_text(dashboard_frame_state(None, false, state.error, 0))
        .lines()
        .count();
    let output = dashboard_frame_text(state);
    let (output, marker) = navigation.viewport(&output, prefix_lines, terminal_size.height);
    let menu = failure
        .map(|failure| menu::action_failure_screen(failure, terminal_size, marker))
        .or_else(|| menu.map(|menu| menu.screen(terminal_size, marker)));
    write_dashboard_screen(&output, terminal_size, marker, menu)
}

fn dashboard_frame_text(state: DashboardFrameState<'_>) -> String {
    let mut output = String::new();
    if let Some(error) = state.error {
        output.push_str(&format!("Last refresh failed: {error}\n\n"));
    } else if state.frame.is_none() && state.refreshing {
        let spinner = SPINNER_FRAMES[state.spinner_index % SPINNER_FRAMES.len()];
        output.push_str(&format!("{spinner} refreshing\n"));
    }
    if let Some(frame) = state.frame {
        output.push_str(frame);
    }
    output
}

fn write_dashboard_screen(
    output: &str,
    terminal_size: DashboardTerminalSize,
    marker: Option<usize>,
    menu: Option<menu::MenuScreen>,
) -> io::Result<()> {
    let mut stdout = io::stdout();
    queue!(
        stdout,
        terminal::BeginSynchronizedUpdate,
        MoveTo(0, 0),
        Clear(ClearType::All)
    )?;
    for (row, line) in clipped_dashboard_lines(output, terminal_size)
        .into_iter()
        .enumerate()
    {
        queue!(stdout, MoveTo(0, row as u16))?;
        stdout.write_all(line.as_bytes())?;
    }
    if let Some(row) = marker.filter(|_| terminal_size.width > 0) {
        queue!(stdout, MoveTo(0, row as u16))?;
        // Paint only the existing left gutter; never rewrite a row's OSC8 link bytes.
        stdout.write_all(b"\x1b[0;38;2;0;135;135m\xe2\x9d\xaf\x1b[0m")?;
    }
    if let Some(menu) = menu {
        for (row, line) in menu.lines.iter().enumerate() {
            queue!(stdout, MoveTo(menu.x as u16, (menu.y + row) as u16))?;
            stdout.write_all(line.as_bytes())?;
        }
    }
    queue!(stdout, terminal::EndSynchronizedUpdate)?;
    stdout.flush()
}

fn clipped_dashboard_lines(output: &str, terminal_size: DashboardTerminalSize) -> Vec<String> {
    output
        .split('\n')
        .take(terminal_size.height)
        .map(|line| ellipsize_rendered_line(line.trim_end_matches('\r'), Some(terminal_size.width)))
        .collect()
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
    fn dashboard_frame_text_omits_persistent_refresh_header() {
        let text = dashboard_frame_text(dashboard_frame_state(Some("repo rows\n"), false, None, 0));

        assert_eq!(text, "repo rows\n");
    }

    #[test]
    fn dashboard_frame_text_shows_initial_refresh_until_frame_exists() {
        let text = dashboard_frame_text(dashboard_frame_state(None, true, None, 0));

        assert_eq!(text, "⠋ refreshing\n");
    }

    #[test]
    fn dashboard_frame_text_keeps_refresh_errors_visible() {
        let text = dashboard_frame_text(dashboard_frame_state(
            Some("cached rows\n"),
            false,
            Some("network down"),
            0,
        ));

        assert_eq!(text, "Last refresh failed: network down\n\ncached rows\n");
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

    #[test]
    fn dashboard_output_is_clipped_to_terminal_size() {
        let lines = clipped_dashboard_lines(
            "abcdef\nok\nthird",
            DashboardTerminalSize {
                width: 4,
                height: 2,
            },
        );

        assert_eq!(lines, vec!["abc…".to_owned(), "ok".to_owned()]);
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
