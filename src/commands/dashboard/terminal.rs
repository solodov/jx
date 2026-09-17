use super::*;
use crate::commands::pr_actions::execute_pr_action;
use crate::domain::PreparedPrAction;

pub(super) struct DashboardTerminalSession {
    restored: bool,
}

impl DashboardTerminalSession {
    pub(super) fn enter() -> io::Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(io::Error::other(
                "Cannot run an interactive dashboard without an interactive terminal",
            ));
        }
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = execute!(stdout, Show, LeaveAlternateScreen);
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        Ok(Self { restored: false })
    }

    pub(super) fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        let display_result = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let raw_result = terminal::disable_raw_mode();
        self.restored = display_result.is_ok() && raw_result.is_ok();
        display_result?;
        raw_result
    }

    /// Leaves the dashboard for a single foreground command and restores it after acknowledgement.
    pub(super) fn run_action(
        &mut self,
        action: &PreparedPrAction,
        interrupts: &DashboardInterrupts,
    ) -> io::Result<()> {
        foreground_session(self, || {
            let modes = ForegroundTerminalModes::capture()?;
            let result = execute_pr_action(action);
            modes.restore()?;
            // The child's Ctrl-C belongs to this action, not the newly resumed dashboard.
            interrupts.take_pending();
            acknowledge_action(action, result, interrupts)
        })
    }
}

impl Drop for DashboardTerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

trait ForegroundTerminal {
    fn suspend(&mut self) -> io::Result<()>;
    fn resume(&mut self) -> io::Result<()>;
}

impl ForegroundTerminal for DashboardTerminalSession {
    fn suspend(&mut self) -> io::Result<()> {
        self.restore()
    }
    fn resume(&mut self) -> io::Result<()> {
        *self = Self::enter()?;
        Ok(())
    }
}

/// Resumption is attempted even when spawning, waiting, or acknowledging fails.
fn foreground_session(
    terminal: &mut impl ForegroundTerminal,
    run: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    terminal.suspend()?;
    let result = run();
    terminal.resume()?;
    result
}

fn acknowledge_action(
    action: &PreparedPrAction,
    result: io::Result<std::process::ExitStatus>,
    interrupts: &DashboardInterrupts,
) -> io::Result<()> {
    let summary = match result {
        Ok(status) if status.success() => "completed".to_owned(),
        Ok(status) => format!("stopped: {status}"),
        Err(error) => format!("failed: {error}"),
    };
    let mut stdout = io::stdout();
    execute!(
        stdout,
        Show,
        LeaveAlternateScreen,
        event::DisableMouseCapture
    )?;
    stdout.write_all(b"\x1b]8;;\x1b\\\x1b[0m")?;
    terminal::enable_raw_mode()?;
    let result = (|| {
        // Input queued for the command must not acknowledge a failure or invoke another action.
        while event::poll(Duration::ZERO)? {
            let _ = event::read()?;
        }
        writeln!(
            stdout,
            "\r\n{} — {}",
            menu::plain_text(&action.title),
            menu::plain_text(&summary)
        )?;
        write!(stdout, "Enter/Esc to return to the dashboard… ")?;
        stdout.flush()?;
        loop {
            if interrupts.take_pending() {
                break;
            }
            if event::poll(DASHBOARD_EVENT_POLL)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press
                        && (matches!(key.code, KeyCode::Enter | KeyCode::Esc)
                            || dashboard_interrupt_key(key))
                    {
                        break;
                    }
                }
            }
        }
        Ok(())
    })();
    let restored = terminal::disable_raw_mode();
    result.and(restored)
}

/// A crashed/cancelled TUI can leave termios altered outside crossterm's raw-mode cache.
struct ForegroundTerminalModes {
    #[cfg(unix)]
    saved: libc::termios,
}

impl ForegroundTerminalModes {
    fn capture() -> io::Result<Self> {
        #[cfg(unix)]
        {
            let mut saved = std::mem::MaybeUninit::uninit();
            // SAFETY: tcgetattr writes one termios into this valid allocation; read only on success.
            if unsafe { libc::tcgetattr(libc::STDIN_FILENO, saved.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                saved: unsafe { saved.assume_init() },
            })
        }
        #[cfg(not(unix))]
        Ok(Self {})
    }

    fn restore(&self) -> io::Result<()> {
        // SAFETY: saved came from tcgetattr and remains live for this call.
        #[cfg(unix)]
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for ForegroundTerminalModes {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(test)]
#[path = "tests/terminal.rs"]
mod tests;
