use super::*;

pub(super) trait ProgressSink {
    fn status(&self, message: &str);

    /// Shows bounded command progress with a fixed-width percentage before the action label.
    fn percentage(&self, action: &str, completed: usize, total: usize) {
        let percent = progress_percent(completed, total);
        self.status(&format!("{percent:>3}% {action}…"));
    }

    fn finish(&self);
}

#[cfg(test)]
pub(super) struct NoProgress;

#[cfg(test)]
impl ProgressSink for NoProgress {
    fn status(&self, _message: &str) {}

    fn finish(&self) {}
}

pub(super) struct SpinnerProgress {
    enabled: bool,
    bar: RefCell<Option<ProgressBar>>,
}

impl SpinnerProgress {
    pub(super) fn new() -> Self {
        Self {
            enabled: io::stderr().is_terminal(),
            bar: RefCell::new(None),
        }
    }
}

impl ProgressSink for SpinnerProgress {
    fn status(&self, message: &str) {
        if !self.enabled {
            return;
        }

        let bar = self.progress_bar();
        bar.set_style(spinner_style());
        bar.set_message(message.to_owned());
    }

    fn percentage(&self, action: &str, completed: usize, total: usize) {
        if !self.enabled {
            return;
        }

        let bar = self.progress_bar();
        bar.set_style(percentage_spinner_style());
        bar.set_length(total as u64);
        bar.set_position(completed.min(total) as u64);
        bar.set_message(format!("{action}…"));
    }

    fn finish(&self) {
        if let Some(bar) = self.bar.borrow_mut().take() {
            bar.finish_and_clear();
        }
    }
}

impl SpinnerProgress {
    fn progress_bar(&self) -> ProgressBar {
        let mut bar = self.bar.borrow_mut();
        bar.get_or_insert_with(|| {
            // Create the spinner only for commands that actually report progress,
            // so passthrough commands can hand the terminal to child renderers cleanly.
            let bar = ProgressBar::new_spinner();
            bar.enable_steady_tick(Duration::from_millis(80));
            bar
        })
        .clone()
    }
}

fn progress_percent(completed: usize, total: usize) -> usize {
    completed
        .min(total)
        .saturating_mul(100)
        .checked_div(total)
        .unwrap_or(100)
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner} {msg}").expect("spinner template is valid")
}

fn percentage_spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner} {percent:>3}% {msg}")
        .expect("percentage spinner template is valid")
}

impl Drop for SpinnerProgress {
    fn drop(&mut self) {
        if let Some(bar) = self.bar.get_mut().take() {
            bar.finish_and_clear();
        }
    }
}
