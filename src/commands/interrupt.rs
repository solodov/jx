use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, OnceLock,
};

/// Installs cooperative cancellation; dashboards consume their own interrupt state.
pub(super) fn install_interrupt_handler() -> io::Result<()> {
    interrupt_state()?;
    crate::jj::install_mutation_interrupt_check(check_jj_interrupted);
    Ok(())
}

/// Lets the dashboard handle interrupts and clean up its terminal and running action.
pub(super) struct DashboardInterrupts(&'static Arc<InterruptState>);

impl DashboardInterrupts {
    pub(super) fn enter() -> io::Result<Self> {
        let state = interrupt_state()?;
        state.dashboard.store(true, Ordering::SeqCst);
        Ok(Self(state))
    }

    pub(super) fn take_pending(&self) -> usize {
        self.0.pending.swap(0, Ordering::SeqCst)
    }
}

impl Drop for DashboardInterrupts {
    fn drop(&mut self) {
        self.0.dashboard.store(false, Ordering::SeqCst);
    }
}

/// Rejects new operations without interrupting an active repository mutation.
pub(super) fn check_interrupted() -> io::Result<()> {
    if interrupt_requested() {
        Err(io::Error::new(io::ErrorKind::Interrupted, "Interrupted"))
    } else {
        Ok(())
    }
}

/// Checks the same boundary for command services returning jj errors.
pub(super) fn check_jj_interrupted() -> Result<(), JjError> {
    check_interrupted().map_err(|_| JjError::Interrupted)
}

/// Reports cancellation outside dashboards, which own their interrupt handling.
fn interrupt_requested() -> bool {
    STATE
        .get()
        .and_then(|state| state.as_ref().ok())
        .is_some_and(|state| {
            !state.dashboard.load(Ordering::SeqCst) && state.pending.load(Ordering::SeqCst) > 0
        })
}

/// Returns through the command entrypoint only after operation-local resources have dropped.
pub(super) fn finish_interrupted_command(
    result: Result<CommandResult, CommandError>,
) -> Result<CommandResult, CommandError> {
    if !interrupt_requested() {
        return result;
    }
    restore_terminal_cursor();
    let stdout = match result {
        Ok(result) => result.stdout,
        Err(CommandError::Environment(error)) if error.kind() == io::ErrorKind::Interrupted => {
            String::new()
        }
        Err(CommandError::Jj(JjError::Interrupted)) => String::new(),
        Err(error) => {
            eprintln!("error: {error}");
            String::new()
        }
    };
    Ok(CommandResult::with_exit_code(stdout, 130))
}

struct InterruptState {
    pending: AtomicUsize,
    dashboard: AtomicBool,
}

static STATE: OnceLock<io::Result<Arc<InterruptState>>> = OnceLock::new();

fn interrupt_state() -> io::Result<&'static Arc<InterruptState>> {
    STATE
        .get_or_init(|| {
            let state = Arc::new(InterruptState {
                pending: AtomicUsize::new(0),
                dashboard: AtomicBool::new(false),
            });
            let handler = state.clone();
            // SAFETY: the signal handler only updates an atomic. Record at delivery so a
            // delayed worker cannot replay an interrupt already consumed by the dashboard.
            unsafe {
                signal_hook::low_level::register(signal_hook::consts::signal::SIGINT, move || {
                    handler.pending.fetch_add(1, Ordering::SeqCst);
                })?;
            }
            let mut signals =
                signal_hook::iterator::Signals::new([signal_hook::consts::signal::SIGINT])?;
            let worker = state.clone();
            std::thread::Builder::new()
                .name("jx-signal-handler".to_owned())
                .spawn(move || {
                    let mut announced = false;
                    for _ in signals.forever() {
                        if worker.dashboard.load(Ordering::SeqCst) {
                            continue;
                        }
                        match worker.pending.load(Ordering::SeqCst) {
                            0 => {}
                            1 if !announced => {
                                eprintln!("Interrupt requested. Waiting for the current operation to finish. Press Ctrl-C again to force exit (may leave Git locks).");
                                announced = true;
                            }
                            1 => {}
                            _ => {
                                restore_terminal_cursor();
                                std::process::exit(130);
                            }
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
