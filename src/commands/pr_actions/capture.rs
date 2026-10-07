use crate::commands::LogSink;
use std::{
    io,
    path::Path,
    process::{ExitStatus, Stdio},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    runtime::{Builder, Runtime},
    task::JoinHandle,
};

/// Owns a child and bounded stream capture without exposing log files to the subprocess.
pub(super) struct CapturedProcess {
    runtime: Runtime,
    child: Option<Child>,
    pid: Option<u32>,
    streams: Vec<JoinHandle<io::Result<()>>>,
    exited: Option<(ExitStatus, Instant)>,
}

impl CapturedProcess {
    /// Starts non-interactive execution, capturing each stream through the diagnostic sink.
    pub(super) fn spawn(command: &[String], cwd: &Path, sink: LogSink) -> io::Result<Self> {
        let (program, arguments) = command
            .split_first()
            .ok_or_else(|| io::Error::other("action has no executable"))?;
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let mut command = Command::new(program);
        command
            .args(arguments)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = {
            let _guard = runtime.enter();
            command.spawn()?
        };
        let pid = child.id();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let streams = vec![
            runtime.spawn(capture_stream(stdout, sink.clone())),
            runtime.spawn(capture_stream(stderr, sink)),
        ];
        Ok(Self {
            runtime,
            child: Some(child),
            pid,
            streams,
            exited: None,
        })
    }

    /// Polls exit and stream completion without waiting on child output in the dashboard thread.
    pub(super) fn poll(&mut self) -> Option<io::Result<ExitStatus>> {
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(None) => return None,
                Ok(Some(status)) => {
                    self.child = None;
                    self.exited = Some((status, Instant::now()));
                }
                Err(error) => return Some(Err(error)),
            }
        }
        let (status, exited_at) = self.exited?;
        if !self.streams.iter().all(JoinHandle::is_finished) {
            // Descendants can retain pipe handles after their parent exits. Never hang the dashboard on them.
            if exited_at.elapsed() < Duration::from_secs(1) {
                return None;
            }
            self.stop();
            return Some(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "action output did not close after the command exited",
            )));
        }
        for stream in self.streams.drain(..) {
            match self.runtime.block_on(stream) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Some(Err(error)),
                Err(error) => return Some(Err(io::Error::other(error))),
            }
        }
        Some(Ok(status))
    }

    /// Stops the process group, reaps the child, and cancels pipe readers before releasing their runtime.
    pub(super) fn stop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid.take() {
            // SAFETY: this action was spawned into its own process group.
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let _ = self.runtime.block_on(child.wait());
        }
        for stream in &self.streams {
            stream.abort();
        }
        for stream in self.streams.drain(..) {
            let _ = self.runtime.block_on(stream);
        }
    }
}

impl Drop for CapturedProcess {
    fn drop(&mut self) {
        if self.child.is_some() || !self.streams.is_empty() {
            self.stop();
        }
    }
}

/// Drains a stream with bounded memory even after a write fails, so a full pipe cannot block the child.
async fn capture_stream(mut stream: impl AsyncRead + Unpin, sink: LogSink) -> io::Result<()> {
    let mut buffer = [0_u8; 8192];
    let mut first_error = None;
    loop {
        match stream.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                if let Err(error) = sink.append(&buffer[..count]) {
                    first_error.get_or_insert(error);
                }
            }
            Err(error) => {
                first_error.get_or_insert(error);
                break;
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}
