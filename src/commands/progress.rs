use super::*;

pub(super) trait ProgressSink {
    fn status(&self, message: &str);
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

        let mut bar = self.bar.borrow_mut();
        let bar = bar.get_or_insert_with(|| {
            // Create the spinner only for commands that actually report progress,
            // so passthrough commands can hand the terminal to child renderers cleanly.
            let bar = ProgressBar::new_spinner();
            bar.set_style(
                ProgressStyle::with_template("{spinner} {msg}").expect("spinner template is valid"),
            );
            bar.enable_steady_tick(Duration::from_millis(80));
            bar
        });
        bar.set_message(message.to_owned());
    }

    fn finish(&self) {
        if let Some(bar) = self.bar.borrow_mut().take() {
            bar.finish_and_clear();
        }
    }
}

impl Drop for SpinnerProgress {
    fn drop(&mut self) {
        if let Some(bar) = self.bar.get_mut().take() {
            bar.finish_and_clear();
        }
    }
}
