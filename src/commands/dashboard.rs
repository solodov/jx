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
    renderer: Box<dyn Fn(DashboardRenderOptions) -> Result<String, String> + Send + Sync>,
}

impl DashboardFrameSnapshot {
    pub(super) fn new(
        renderer: impl Fn(DashboardRenderOptions) -> Result<String, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            renderer: Box::new(renderer),
        }
    }

    fn render(&self, options: DashboardRenderOptions) -> Result<String, String> {
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

/// Runs a live terminal dashboard in the alternate screen until the operator exits.
pub(super) fn run_interactive_dashboard(
    title: &'static str,
    refresh_seconds: u64,
    loader: DashboardFrameLoader,
) -> Result<CommandResult, CommandError> {
    let mut terminal = DashboardTerminalSession::enter()?;
    let mut terminal_size = dashboard_terminal_size()?;
    let mut watcher = ExecutableWatcher::from_process();
    let mut last_snapshot = None::<DashboardFrameSnapshot>;
    let mut last_frame = None::<String>;
    let mut last_refreshed = None::<DateTime<Local>>;
    let mut next_refresh_at = None::<DateTime<Local>>;
    let mut last_error = None::<String>;

    loop {
        let receiver = spawn_dashboard_load(Arc::clone(&loader));
        let refresh_started = Instant::now();
        let mut spinner_index = 0usize;
        render_dashboard_frame(
            title,
            dashboard_frame_state(
                last_frame.as_deref(),
                last_refreshed,
                true,
                last_error.as_deref(),
                spinner_index,
                next_refresh_at,
            ),
            terminal_size,
        )?;
        loop {
            if watcher.changed() {
                terminal.restore()?;
                return restart_dashboard_process();
            }
            match receiver.try_recv() {
                Ok(Ok(snapshot)) => {
                    let refreshed_at = Local::now();
                    match snapshot.render(terminal_size.render_options()) {
                        Ok(frame) => {
                            last_frame = Some(frame);
                            last_snapshot = Some(snapshot);
                            last_refreshed = Some(refreshed_at);
                            next_refresh_at =
                                next_dashboard_refresh_time(refreshed_at, refresh_seconds);
                            last_error = None;
                        }
                        Err(error) => {
                            last_error = Some(error);
                            next_refresh_at =
                                next_dashboard_refresh_time(Local::now(), refresh_seconds);
                        }
                    }
                    render_dashboard_frame(
                        title,
                        dashboard_frame_state(
                            last_frame.as_deref(),
                            last_refreshed,
                            false,
                            last_error.as_deref(),
                            spinner_index,
                            next_refresh_at,
                        ),
                        terminal_size,
                    )?;
                    break;
                }
                Ok(Err(error)) => {
                    last_error = Some(error);
                    next_refresh_at = next_dashboard_refresh_time(Local::now(), refresh_seconds);
                    render_dashboard_frame(
                        title,
                        dashboard_frame_state(
                            last_frame.as_deref(),
                            last_refreshed,
                            false,
                            last_error.as_deref(),
                            spinner_index,
                            next_refresh_at,
                        ),
                        terminal_size,
                    )?;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    last_error = Some("dashboard refresh worker stopped unexpectedly".to_owned());
                    next_refresh_at = next_dashboard_refresh_time(Local::now(), refresh_seconds);
                    render_dashboard_frame(
                        title,
                        dashboard_frame_state(
                            last_frame.as_deref(),
                            last_refreshed,
                            false,
                            last_error.as_deref(),
                            spinner_index,
                            next_refresh_at,
                        ),
                        terminal_size,
                    )?;
                    break;
                }
            }

            if dashboard_refresh_timed_out(refresh_started.elapsed()) {
                last_error = Some(dashboard_refresh_timeout_error());
                next_refresh_at = next_dashboard_refresh_time(Local::now(), refresh_seconds);
                render_dashboard_frame(
                    title,
                    dashboard_frame_state(
                        last_frame.as_deref(),
                        last_refreshed,
                        false,
                        last_error.as_deref(),
                        spinner_index,
                        next_refresh_at,
                    ),
                    terminal_size,
                )?;
                break;
            }

            match read_dashboard_event(DASHBOARD_EVENT_POLL, &mut terminal_size)? {
                DashboardEvent::Exit => {
                    terminal.restore()?;
                    return Ok(CommandResult::success(String::new()));
                }
                DashboardEvent::Interrupt => {
                    terminal.restore()?;
                    return Ok(CommandResult::with_exit_code(String::new(), 130));
                }
                DashboardEvent::Resized => rerender_dashboard_snapshot(
                    last_snapshot.as_ref(),
                    terminal_size,
                    &mut last_frame,
                    &mut last_error,
                ),
                DashboardEvent::None => {
                    spinner_index = spinner_index.wrapping_add(1);
                }
            }
            render_dashboard_frame(
                title,
                dashboard_frame_state(
                    last_frame.as_deref(),
                    last_refreshed,
                    true,
                    last_error.as_deref(),
                    spinner_index,
                    next_refresh_at,
                ),
                terminal_size,
            )?;
        }

        while let Some(wait_duration) = dashboard_wait_duration(Local::now(), next_refresh_at) {
            if watcher.changed() {
                terminal.restore()?;
                return restart_dashboard_process();
            }
            match read_dashboard_event(wait_duration, &mut terminal_size)? {
                DashboardEvent::Exit => {
                    terminal.restore()?;
                    return Ok(CommandResult::success(String::new()));
                }
                DashboardEvent::Interrupt => {
                    terminal.restore()?;
                    return Ok(CommandResult::with_exit_code(String::new(), 130));
                }
                DashboardEvent::Resized => {
                    rerender_dashboard_snapshot(
                        last_snapshot.as_ref(),
                        terminal_size,
                        &mut last_frame,
                        &mut last_error,
                    );
                    render_dashboard_frame(
                        title,
                        dashboard_frame_state(
                            last_frame.as_deref(),
                            last_refreshed,
                            false,
                            last_error.as_deref(),
                            spinner_index,
                            next_refresh_at,
                        ),
                        terminal_size,
                    )?;
                }
                DashboardEvent::None => {}
            }
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

fn rerender_dashboard_snapshot(
    snapshot: Option<&DashboardFrameSnapshot>,
    terminal_size: DashboardTerminalSize,
    frame: &mut Option<String>,
    error: &mut Option<String>,
) {
    let Some(snapshot) = snapshot else {
        return;
    };
    match snapshot.render(terminal_size.render_options()) {
        Ok(rendered) => *frame = Some(rendered),
        Err(render_error) => *error = Some(render_error),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardEvent {
    None,
    Resized,
    Exit,
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
        Event::Key(key) if dashboard_exit_key(key) => Ok(DashboardEvent::Exit),
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
    last_refreshed: Option<DateTime<Local>>,
    refreshing: bool,
    error: Option<&'a str>,
    spinner_index: usize,
    next_refresh_at: Option<DateTime<Local>>,
}

fn dashboard_frame_state<'a>(
    frame: Option<&'a str>,
    last_refreshed: Option<DateTime<Local>>,
    refreshing: bool,
    error: Option<&'a str>,
    spinner_index: usize,
    next_refresh_at: Option<DateTime<Local>>,
) -> DashboardFrameState<'a> {
    DashboardFrameState {
        frame,
        last_refreshed,
        refreshing,
        error,
        spinner_index,
        next_refresh_at,
    }
}

fn render_dashboard_frame(
    title: &str,
    state: DashboardFrameState<'_>,
    terminal_size: DashboardTerminalSize,
) -> io::Result<()> {
    let output = dashboard_frame_text(title, state);
    write_dashboard_screen(&output, terminal_size)
}

fn dashboard_frame_text(title: &str, state: DashboardFrameState<'_>) -> String {
    let spinner = SPINNER_FRAMES[state.spinner_index % SPINNER_FRAMES.len()];
    let refreshed = state
        .last_refreshed
        .map(format_dashboard_refresh_time)
        .unwrap_or_else(|| "never".to_owned());
    let refresh_state = if state.refreshing {
        format!("{spinner} refreshing")
    } else {
        state
            .next_refresh_at
            .map(|time| format!("next refresh: {}", format_dashboard_refresh_time(time)))
            .unwrap_or_else(|| "next refresh: unknown".to_owned())
    };
    let mut output = String::new();
    output.push_str(&format!(
        "{title}  Last refreshed: {refreshed}  {refresh_state}\n"
    ));
    if let Some(error) = state.error {
        output.push_str(&format!("Last refresh failed: {error}\n"));
    } else {
        output.push('\n');
    }
    if let Some(frame) = state.frame {
        output.push_str(frame);
    }
    output
}

fn write_dashboard_screen(output: &str, terminal_size: DashboardTerminalSize) -> io::Result<()> {
    let mut stdout = io::stdout();
    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    for (row, line) in clipped_dashboard_lines(output, terminal_size)
        .into_iter()
        .enumerate()
    {
        queue!(stdout, MoveTo(0, row as u16))?;
        stdout.write_all(line.as_bytes())?;
    }
    stdout.flush()
}

fn clipped_dashboard_lines(output: &str, terminal_size: DashboardTerminalSize) -> Vec<String> {
    output
        .split('\n')
        .take(terminal_size.height)
        .map(|line| ellipsize_rendered_line(line.trim_end_matches('\r'), Some(terminal_size.width)))
        .collect()
}

fn format_dashboard_refresh_time(time: DateTime<Local>) -> String {
    time.format("%Y-%m-%d %H:%M").to_string()
}

struct DashboardTerminalSession {
    restored: bool,
}

impl DashboardTerminalSession {
    fn enter() -> io::Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(io::Error::other(
                "Cannot run an interactive dashboard without an interactive terminal",
            ));
        }

        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        Ok(Self { restored: false })
    }

    fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        let mut stdout = io::stdout();
        let display_result = execute!(stdout, Show, LeaveAlternateScreen);
        let raw_result = terminal::disable_raw_mode();
        self.restored = true;
        display_result?;
        raw_result
    }
}

impl Drop for DashboardTerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
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
    let mut stdout = io::stdout();
    let _ = execute!(stdout, Clear(ClearType::All), MoveTo(0, 0));
    let _ = writeln!(stdout, "jx updated; restarting...");
    let _ = stdout.flush();
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
    fn dashboard_refresh_time_format_omits_seconds() {
        assert_eq!(
            format_dashboard_refresh_time(local_test_time_at(12, 2, 15)),
            "2026-01-15 12:02"
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

    #[test]
    fn dashboard_snapshot_rerenders_with_current_terminal_width() {
        let snapshot = DashboardFrameSnapshot::new(|options| {
            Ok(format!(
                "width={}",
                options.terminal_width.unwrap_or_default()
            ))
        });
        let mut frame = None;
        let mut error = None;

        rerender_dashboard_snapshot(
            Some(&snapshot),
            DashboardTerminalSize {
                width: 42,
                height: 10,
            },
            &mut frame,
            &mut error,
        );

        assert_eq!(frame.as_deref(), Some("width=42"));
        assert_eq!(error, None);
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
