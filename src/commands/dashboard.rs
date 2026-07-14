use super::*;
use chrono::{DateTime, Local, TimeZone as _};
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

const CLEAR_SCREEN: &str = "\x1b[2J\x1b[H";
const DASHBOARD_IDLE_POLL: Duration = Duration::from_millis(500);
const DASHBOARD_REFRESH_TIMEOUT: Duration = Duration::from_secs(120);
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(super) type DashboardFrameLoader = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

/// Runs a live terminal dashboard by replacing the display with complete refresh frames.
pub(super) fn run_interactive_dashboard(
    title: &'static str,
    refresh_seconds: u64,
    loader: DashboardFrameLoader,
) -> Result<CommandResult, CommandError> {
    let mut watcher = ExecutableWatcher::from_process();
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
            last_frame.as_deref(),
            last_refreshed,
            true,
            last_error.as_deref(),
            spinner_index,
            next_refresh_at,
        )?;
        loop {
            if watcher.changed() {
                return restart_dashboard_process();
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(frame)) => {
                    let refreshed_at = Local::now();
                    last_frame = Some(frame);
                    last_refreshed = Some(refreshed_at);
                    next_refresh_at = next_dashboard_refresh_time(refreshed_at, refresh_seconds);
                    last_error = None;
                    render_dashboard_frame(
                        title,
                        last_frame.as_deref(),
                        last_refreshed,
                        false,
                        None,
                        spinner_index,
                        next_refresh_at,
                    )?;
                    break;
                }
                Ok(Err(error)) => {
                    last_error = Some(error);
                    next_refresh_at = next_dashboard_refresh_time(Local::now(), refresh_seconds);
                    render_dashboard_frame(
                        title,
                        last_frame.as_deref(),
                        last_refreshed,
                        false,
                        last_error.as_deref(),
                        spinner_index,
                        next_refresh_at,
                    )?;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if dashboard_refresh_timed_out(refresh_started.elapsed()) {
                        last_error = Some(dashboard_refresh_timeout_error());
                        next_refresh_at =
                            next_dashboard_refresh_time(Local::now(), refresh_seconds);
                        render_dashboard_frame(
                            title,
                            last_frame.as_deref(),
                            last_refreshed,
                            false,
                            last_error.as_deref(),
                            spinner_index,
                            next_refresh_at,
                        )?;
                        break;
                    }
                    spinner_index = spinner_index.wrapping_add(1);
                    render_dashboard_frame(
                        title,
                        last_frame.as_deref(),
                        last_refreshed,
                        true,
                        last_error.as_deref(),
                        spinner_index,
                        next_refresh_at,
                    )?;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    last_error = Some("dashboard refresh worker stopped unexpectedly".to_owned());
                    next_refresh_at = next_dashboard_refresh_time(Local::now(), refresh_seconds);
                    render_dashboard_frame(
                        title,
                        last_frame.as_deref(),
                        last_refreshed,
                        false,
                        last_error.as_deref(),
                        spinner_index,
                        next_refresh_at,
                    )?;
                    break;
                }
            }
        }

        while let Some(wait_duration) = dashboard_wait_duration(Local::now(), next_refresh_at) {
            if watcher.changed() {
                return restart_dashboard_process();
            }
            thread::sleep(wait_duration);
        }
    }
}

fn spawn_dashboard_load(loader: DashboardFrameLoader) -> mpsc::Receiver<Result<String, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(loader());
    });
    receiver
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

fn render_dashboard_frame(
    title: &str,
    frame: Option<&str>,
    last_refreshed: Option<DateTime<Local>>,
    refreshing: bool,
    error: Option<&str>,
    spinner_index: usize,
    next_refresh_at: Option<DateTime<Local>>,
) -> io::Result<()> {
    let spinner = SPINNER_FRAMES[spinner_index % SPINNER_FRAMES.len()];
    let refreshed = last_refreshed
        .map(format_dashboard_refresh_time)
        .unwrap_or_else(|| "never".to_owned());
    let state = if refreshing {
        format!("{spinner} refreshing")
    } else {
        next_refresh_at
            .map(|time| format!("next refresh: {}", format_dashboard_refresh_time(time)))
            .unwrap_or_else(|| "next refresh: unknown".to_owned())
    };
    let mut output = String::new();
    output.push_str(CLEAR_SCREEN);
    output.push_str(&format!("{title}  Last refreshed: {refreshed}  {state}\n",));
    if let Some(error) = error {
        output.push_str(&format!("Last refresh failed: {error}\n"));
    } else {
        output.push('\n');
    }
    if let Some(frame) = frame {
        output.push_str(frame);
    }
    let mut stdout = io::stdout();
    stdout.write_all(output.as_bytes())?;
    stdout.flush()
}

fn format_dashboard_refresh_time(time: DateTime<Local>) -> String {
    time.format("%Y-%m-%d %H:%M").to_string()
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
    let _ = io::stdout().write_all(b"\x1b[2J\x1b[Hjx updated; restarting...\n");
    let _ = io::stdout().flush();
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
