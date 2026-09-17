use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};

/// Restores the terminal on ordinary interrupts; dashboards consume their own interrupt state.
pub(super) fn install_interrupt_cursor_restore() -> io::Result<()> {
    interrupt_state().map(|_| ())
}

/// Keeps the parent dashboard alive while a foreground child receives terminal signals normally.
pub(super) struct DashboardInterrupts(&'static Arc<InterruptState>);

impl DashboardInterrupts {
    pub(super) fn enter() -> io::Result<Self> {
        let state = interrupt_state()?;
        state.dashboard.store(true, Ordering::SeqCst);
        Ok(Self(state))
    }

    pub(super) fn take_pending(&self) -> bool {
        self.0.pending.swap(false, Ordering::SeqCst)
    }
}

impl Drop for DashboardInterrupts {
    fn drop(&mut self) {
        self.0.dashboard.store(false, Ordering::SeqCst);
    }
}

struct InterruptState {
    pending: Arc<AtomicBool>,
    dashboard: AtomicBool,
}

fn interrupt_state() -> io::Result<&'static Arc<InterruptState>> {
    static STATE: OnceLock<io::Result<Arc<InterruptState>>> = OnceLock::new();
    STATE
        .get_or_init(|| {
            let state = Arc::new(InterruptState {
                pending: Arc::new(AtomicBool::new(false)),
                dashboard: AtomicBool::new(false),
            });
            // Record at signal delivery, not when the worker wakes: a late wakeup must not
            // turn an already acknowledged child cancellation into a dashboard exit.
            signal_hook::flag::register(
                signal_hook::consts::signal::SIGINT,
                state.pending.clone(),
            )?;
            let mut signals =
                signal_hook::iterator::Signals::new([signal_hook::consts::signal::SIGINT])?;
            let worker = state.clone();
            std::thread::Builder::new()
                .name("jx-signal-handler".to_owned())
                .spawn(move || {
                    for _ in signals.forever() {
                        if !worker.dashboard.load(Ordering::SeqCst)
                            && worker.pending.swap(false, Ordering::SeqCst)
                        {
                            restore_terminal_cursor();
                            std::process::exit(130);
                        }
                    }
                })?;
            Ok(state)
        })
        .as_ref()
        .map_err(|error| io::Error::other(error.to_string()))
}

/// Restores terminal state when an interactive command exits through an interrupt path.
pub(super) fn restore_terminal_cursor() {
    let _ = crossterm::terminal::disable_raw_mode();
    if io::stdout().is_terminal() {
        let mut stdout = io::stdout();
        let _ = stdout.write_all(b"\x1b[?25h\x1b[?1049l");
        let _ = stdout.flush();
    }
    let mut stderr = io::stderr();
    let _ = stderr.write_all(b"\x1b[?25h");
    let _ = stderr.flush();
}
