use super::*;

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
        let display_result = execute!(
            io::stdout(),
            terminal::EnableLineWrap,
            Show,
            LeaveAlternateScreen
        );
        let raw_result = terminal::disable_raw_mode();
        self.restored = display_result.is_ok() && raw_result.is_ok();
        display_result?;
        raw_result
    }
}

impl Drop for DashboardTerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
