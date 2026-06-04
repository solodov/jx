//! Lightweight JSONL performance tracing for command paths where latency matters.
//!
//! The tracer intentionally mirrors the small in-process span model used by
//! Shiny: commands create one span, record named step timings, and emit one
//! structured log record on completion. Logging is best-effort so perf tracing
//! never changes command behavior.

use super::*;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Map, Value};
use std::{
    fs::{self, File, OpenOptions},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime},
};

const DEFAULT_PERF_LOG_FILE: &str = "jx-perf.log";
static NEXT_SPAN_ID: AtomicU64 = AtomicU64::new(1);

/// Append-only destination for command performance spans.
#[derive(Debug, Clone)]
pub(super) struct PerfLog {
    writer: Option<Arc<Mutex<File>>>,
}

impl PerfLog {
    /// Opens the configured perf log, falling back to a no-op tracer on failure.
    pub(super) fn from_environment(environment: &RuntimeEnvironment) -> Self {
        let Some(path) = perf_log_path(environment) else {
            return Self::disabled();
        };
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            if fs::create_dir_all(parent).is_err() {
                return Self::disabled();
            }
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(|file| Self {
                writer: Some(Arc::new(Mutex::new(file))),
            })
            .unwrap_or_else(|_| Self::disabled())
    }

    pub(super) fn disabled() -> Self {
        Self { writer: None }
    }

    /// Starts a span that will log one perf event when ended or dropped.
    pub(super) fn start(
        &self,
        op: impl Into<String>,
        attrs: impl IntoIterator<Item = PerfAttr>,
    ) -> PerfSpan {
        let span_id = next_span_id();
        PerfSpan {
            writer: self.writer.clone(),
            trace_id: span_id.clone(),
            span_id,
            parent_span_id: None,
            op: normalized_name(op.into(), "unknown"),
            started_at: SystemTime::now(),
            started_instant: Instant::now(),
            attrs: attrs.into_iter().collect(),
            steps: Vec::new(),
            error: None,
            ended: false,
        }
    }
}

/// One active traced operation.
pub(super) struct PerfSpan {
    writer: Option<Arc<Mutex<File>>>,
    trace_id: String,
    span_id: String,
    parent_span_id: Option<String>,
    op: String,
    started_at: SystemTime,
    started_instant: Instant,
    attrs: Vec<PerfAttr>,
    steps: Vec<PerfStep>,
    error: Option<String>,
    ended: bool,
}

impl PerfSpan {
    /// Starts a step timer that can wrap work which records nested steps.
    pub(super) fn start_step(
        &mut self,
        name: impl Into<String>,
        attrs: impl IntoIterator<Item = PerfAttr>,
    ) -> PerfStepTimer {
        PerfStepTimer {
            name: normalized_name(name.into(), "step"),
            started: Instant::now(),
            attrs: attrs.into_iter().collect(),
        }
    }

    /// Records a completed step timer.
    pub(super) fn finish_step<E>(
        &mut self,
        timer: PerfStepTimer,
        attrs: impl IntoIterator<Item = PerfAttr>,
        error: Option<&E>,
    ) where
        E: fmt::Display,
    {
        let mut step_attrs = timer.attrs;
        step_attrs.extend(attrs);
        self.record_step_us(
            timer.name,
            micros(timer.started.elapsed()),
            step_attrs,
            error,
        );
    }

    /// Records a step whose duration was measured below the command layer.
    pub(super) fn record_step_us<E>(
        &mut self,
        name: impl Into<String>,
        duration_us: u64,
        attrs: impl IntoIterator<Item = PerfAttr>,
        error: Option<&E>,
    ) where
        E: fmt::Display,
    {
        self.steps.push(PerfStep {
            name: normalized_name(name.into(), "step"),
            duration_us,
            attrs: attrs.into_iter().collect(),
            error: error.map(ToString::to_string),
        });
    }

    /// Records one named step around a fallible operation.
    pub(super) fn measure<T, E>(
        &mut self,
        name: impl Into<String>,
        attrs: impl IntoIterator<Item = PerfAttr>,
        operation: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: fmt::Display,
    {
        self.measure_with_result_attrs(name, attrs, operation, |_| Vec::new())
    }

    /// Records one named step and lets callers add attributes derived from its result.
    pub(super) fn measure_with_result_attrs<T, E>(
        &mut self,
        name: impl Into<String>,
        attrs: impl IntoIterator<Item = PerfAttr>,
        operation: impl FnOnce() -> Result<T, E>,
        result_attrs: impl FnOnce(&Result<T, E>) -> Vec<PerfAttr>,
    ) -> Result<T, E>
    where
        E: fmt::Display,
    {
        let timer = self.start_step(name, attrs);
        let result = operation();
        let attrs = result_attrs(&result);
        self.finish_step(timer, attrs, result.as_ref().err());
        result
    }

    /// Adds attributes to the span event.
    pub(super) fn set(&mut self, attrs: impl IntoIterator<Item = PerfAttr>) {
        self.attrs.extend(attrs);
    }

    /// Marks the span as failed when the command returns an error.
    pub(super) fn record_error(&mut self, error: impl fmt::Display) {
        if self.error.is_none() {
            self.error = Some(error.to_string());
        }
    }

    /// Emits the span immediately. Dropping an un-ended span emits it too.
    pub(super) fn end(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        let Some(writer) = &self.writer else {
            return;
        };

        let event = self.event();
        let Ok(mut writer) = writer.lock() else {
            return;
        };
        if serde_json::to_writer(&mut *writer, &event).is_ok() {
            let _ = writer.write_all(b"\n");
        }
    }

    fn event(&self) -> Value {
        let mut event = Map::new();
        event.insert("event".to_owned(), Value::String("perf".to_owned()));
        event.insert("trace_id".to_owned(), Value::String(self.trace_id.clone()));
        event.insert("span_id".to_owned(), Value::String(self.span_id.clone()));
        event.insert(
            "parent_span_id".to_owned(),
            self.parent_span_id
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        event.insert("op".to_owned(), Value::String(self.op.clone()));
        event.insert(
            "status".to_owned(),
            Value::String(if self.error.is_some() { "error" } else { "ok" }.to_owned()),
        );
        event.insert(
            "started_at".to_owned(),
            Value::String(format_system_time(self.started_at)),
        );
        event.insert(
            "duration_us".to_owned(),
            Value::Number(micros(self.started_instant.elapsed()).into()),
        );
        insert_attrs(&mut event, &self.attrs);
        if !self.steps.is_empty() {
            event.insert(
                "steps".to_owned(),
                Value::Array(self.steps.iter().map(PerfStep::event).collect()),
            );
        }
        if let Some(error) = &self.error {
            event.insert("err".to_owned(), Value::String(error.clone()));
        }
        Value::Object(event)
    }
}

impl Drop for PerfSpan {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PerfAttr {
    key: String,
    value: PerfValue,
}

/// Creates a structured perf attribute.
pub(super) fn perf_attr(key: impl Into<String>, value: impl Into<PerfValue>) -> PerfAttr {
    PerfAttr {
        key: key.into(),
        value: value.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PerfValue {
    String(String),
    U64(u64),
    I64(i64),
    Bool(bool),
}

impl From<&str> for PerfValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for PerfValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&String> for PerfValue {
    fn from(value: &String) -> Self {
        Self::String(value.clone())
    }
}

impl From<usize> for PerfValue {
    fn from(value: usize) -> Self {
        Self::U64(value as u64)
    }
}

impl From<u64> for PerfValue {
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}

impl From<i64> for PerfValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

impl From<bool> for PerfValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PerfStepTimer {
    name: String,
    started: Instant,
    attrs: Vec<PerfAttr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PerfStep {
    name: String,
    duration_us: u64,
    attrs: Vec<PerfAttr>,
    error: Option<String>,
}

impl PerfStep {
    fn event(&self) -> Value {
        let mut event = Map::new();
        event.insert("name".to_owned(), Value::String(self.name.clone()));
        event.insert(
            "duration_us".to_owned(),
            Value::Number(self.duration_us.into()),
        );
        insert_attrs(&mut event, &self.attrs);
        if let Some(error) = &self.error {
            event.insert("err".to_owned(), Value::String(error.clone()));
        }
        Value::Object(event)
    }
}

fn perf_log_path(environment: &RuntimeEnvironment) -> Option<PathBuf> {
    if let Some(path) = environment
        .variable("JX_PERF_LOG")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        if matches!(path, "off" | "false" | "0") {
            return None;
        }
        return Some(PathBuf::from(path));
    }

    environment
        .variable("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .home_dir()
                .map(|home| home.join(".local").join("state"))
        })
        .map(|root| root.join("jx").join(DEFAULT_PERF_LOG_FILE))
}

fn insert_attrs(event: &mut Map<String, Value>, attrs: &[PerfAttr]) {
    for attr in attrs {
        event.insert(attr.key.clone(), attr.value.clone().into_json());
    }
}

impl PerfValue {
    fn into_json(self) -> Value {
        match self {
            Self::String(value) => Value::String(value),
            Self::U64(value) => Value::Number(value.into()),
            Self::I64(value) => Value::Number(value.into()),
            Self::Bool(value) => Value::Bool(value),
        }
    }
}

fn next_span_id() -> String {
    format!("{:016x}", NEXT_SPAN_ID.fetch_add(1, Ordering::Relaxed))
}

fn normalized_name(name: String, fallback: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        fallback.to_owned()
    } else {
        name.to_owned()
    }
}

fn micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn format_system_time(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Micros, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_span_writes_json_event_with_steps() {
        // Verifies: command tracing emits one grep-friendly JSONL record per span.
        let (root, log_path) = temp_perf_log_path("steps");
        let environment = RuntimeEnvironment::new(
            &root,
            [("JX_PERF_LOG".to_owned(), log_path.display().to_string())],
        );
        let log = PerfLog::from_environment(&environment);

        let mut span = log.start(
            "stack.publish",
            [perf_attr("repo", "example-owner/example-repo")],
        );
        span.measure(
            "update_stack",
            [perf_attr("component_nodes", 2_usize)],
            || Ok::<_, CommandError>(()),
        )
        .expect("step succeeds");
        span.end();

        let line = fs::read_to_string(&log_path).expect("perf log writes");
        let event: Value = serde_json::from_str(line.trim()).expect("perf line is json");
        assert_eq!(event["event"], "perf");
        assert_eq!(event["op"], "stack.publish");
        assert_eq!(event["status"], "ok");
        assert_eq!(event["repo"], "example-owner/example-repo");
        assert_eq!(event["steps"][0]["name"], "update_stack");
        assert_eq!(event["steps"][0]["component_nodes"], 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn perf_span_records_errors() {
        // Verifies: failed spans carry status and error text without changing control flow.
        let (root, log_path) = temp_perf_log_path("errors");
        let environment = RuntimeEnvironment::new(
            &root,
            [("JX_PERF_LOG".to_owned(), log_path.display().to_string())],
        );
        let log = PerfLog::from_environment(&environment);

        let mut span = log.start("stack.publish", Vec::new());
        span.record_error("network timeout");
        span.end();

        let line = fs::read_to_string(&log_path).expect("perf log writes");
        let event: Value = serde_json::from_str(line.trim()).expect("perf line is json");
        assert_eq!(event["status"], "error");
        assert_eq!(event["err"], "network timeout");
        let _ = fs::remove_dir_all(root);
    }

    fn temp_perf_log_path(label: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "jx-perf-test-{label}-{}-{}",
            std::process::id(),
            NEXT_SPAN_ID.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).expect("create perf test dir");
        let log_path = root.join("jx-perf.log");
        (root, log_path)
    }
}
