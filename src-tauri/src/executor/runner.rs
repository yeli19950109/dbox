use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bstr::ByteSlice;
use bytes::Bytes;
use chrono::{DateTime, Utc};
#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{CommandSpec, ConfirmedPlan};
use crate::domain::{RunId, RunStatus};
use crate::persistence::{PersistenceStore, RunLogEntry, RunLogStream};
use crate::version::{StatusReason, VerificationStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEventKind {
    StateChanged {
        status: RunStatus,
    },
    Output {
        stream: OutputStream,
        message: String,
    },
    Exit {
        code: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunEvent {
    pub run_id: RunId,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub event: RunEventKind,
}

pub trait ExecutionEventSink: Send + Sync {
    fn emit(&self, event: &RunEvent) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub struct NoopEventSink;

impl ExecutionEventSink for NoopEventSink {
    fn emit(&self, _event: &RunEvent) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct MemoryEventSink {
    events: Mutex<Vec<RunEvent>>,
}

impl MemoryEventSink {
    pub fn events(&self) -> Vec<RunEvent> {
        self.events
            .lock()
            .expect("memory event sink mutex is not poisoned")
            .clone()
    }
}

impl ExecutionEventSink for MemoryEventSink {
    fn emit(&self, event: &RunEvent) -> Result<(), String> {
        self.events
            .lock()
            .map_err(|_| "memory event sink mutex is poisoned".to_owned())?
            .push(event.clone());
        Ok(())
    }
}

#[derive(Clone)]
pub struct CommandExecutor {
    log_store: Option<Arc<PersistenceStore>>,
    event_sink: Arc<dyn ExecutionEventSink>,
    max_tail_bytes: usize,
}

impl Default for CommandExecutor {
    fn default() -> Self {
        Self::new(None, Arc::new(NoopEventSink), 64 * 1024)
    }
}

impl CommandExecutor {
    pub fn new(
        log_store: Option<Arc<PersistenceStore>>,
        event_sink: Arc<dyn ExecutionEventSink>,
        max_tail_bytes: usize,
    ) -> Self {
        Self {
            log_store,
            event_sink,
            max_tail_bytes: max_tail_bytes.max(1),
        }
    }

    pub async fn execute(
        &self,
        run_id: RunId,
        confirmed: &ConfirmedPlan,
        cancellation: CancellationToken,
    ) -> Result<ExecutionResult, ExecutionError> {
        let started_at = Utc::now();
        let mut session = ExecutionSession::new(
            run_id.clone(),
            Arc::clone(&self.event_sink),
            self.log_store.clone(),
        );
        session.emit(RunEventKind::StateChanged {
            status: RunStatus::Running,
        })?;
        tracing::debug!(run_id = %run_id, plan_id = %confirmed.plan().plan_id, "executing confirmed plan");

        let redactor = Redactor::new(confirmed.command());
        let main = run_command(
            confirmed.command(),
            &cancellation,
            &redactor,
            &mut session,
            self.max_tail_bytes,
        )
        .await?;

        let (status, exit_code, output_tail, verification) = match main {
            RawOutcome::Cancelled { tail } => (RunStatus::Cancelled, None, tail, None),
            RawOutcome::TimedOut { tail } => (RunStatus::TimedOut, None, tail, None),
            RawOutcome::Exited { code, tail } => {
                if !confirmed.command().success_exit_codes.contains(&code) {
                    (RunStatus::Failed, Some(code), tail, None)
                } else if let Some(post_check) = &confirmed.command().post_check {
                    session.emit(RunEventKind::Output {
                        stream: OutputStream::System,
                        message: "running post-check".into(),
                    })?;
                    match run_command(
                        post_check,
                        &cancellation,
                        &redactor,
                        &mut session,
                        self.max_tail_bytes,
                    )
                    .await
                    {
                        Ok(RawOutcome::Exited {
                            code: check_code,
                            tail: check_tail,
                        }) if post_check.success_exit_codes.contains(&check_code) => (
                            RunStatus::Succeeded,
                            Some(code),
                            combine_tails(&tail, &check_tail, self.max_tail_bytes),
                            Some(VerificationStatus::Verified),
                        ),
                        Ok(RawOutcome::Cancelled { tail: check_tail }) => (
                            RunStatus::Cancelled,
                            Some(code),
                            combine_tails(&tail, &check_tail, self.max_tail_bytes),
                            None,
                        ),
                        Ok(RawOutcome::TimedOut { tail: check_tail }) => (
                            RunStatus::Partial,
                            Some(code),
                            combine_tails(&tail, &check_tail, self.max_tail_bytes),
                            Some(VerificationStatus::VerificationFailed {
                                reason: StatusReason::new(
                                    "post_check_timed_out",
                                    "The update command succeeded but its post-check timed out",
                                    true,
                                ),
                            }),
                        ),
                        Ok(RawOutcome::Exited {
                            code: check_code,
                            tail: check_tail,
                        }) => (
                            RunStatus::Partial,
                            Some(code),
                            combine_tails(&tail, &check_tail, self.max_tail_bytes),
                            Some(VerificationStatus::VerificationFailed {
                                reason: StatusReason::new(
                                    "post_check_failed",
                                    format!("Post-check exited with code {check_code}"),
                                    true,
                                ),
                            }),
                        ),
                        Err(error) => (
                            RunStatus::Partial,
                            Some(code),
                            tail,
                            Some(VerificationStatus::VerificationFailed {
                                reason: StatusReason::new(
                                    "post_check_error",
                                    error.summary.to_string(),
                                    error.retryable,
                                ),
                            }),
                        ),
                    }
                } else {
                    (RunStatus::Succeeded, Some(code), tail, None)
                }
            }
        };

        session.emit(RunEventKind::Exit { code: exit_code })?;
        session.emit(RunEventKind::StateChanged { status })?;
        Ok(ExecutionResult {
            run_id,
            status,
            exit_code,
            started_at,
            finished_at: Utc::now(),
            output_tail,
            verification,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionResult {
    pub run_id: RunId,
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub output_tail: String,
    pub verification: Option<VerificationStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionErrorKind {
    InvalidPlan,
    Spawn,
    Wait,
    Kill,
    Output,
    Log,
    EventSink,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("run {run_id} failed: {summary}")]
pub struct ExecutionError {
    pub run_id: RunId,
    pub kind: ExecutionErrorKind,
    pub summary: Box<str>,
    pub retryable: bool,
}

impl ExecutionError {
    fn new(
        run_id: &RunId,
        kind: ExecutionErrorKind,
        summary: impl Into<Box<str>>,
        retryable: bool,
    ) -> Self {
        Self {
            run_id: run_id.clone(),
            kind,
            summary: summary.into(),
            retryable,
        }
    }
}

struct ExecutionSession {
    run_id: RunId,
    sequence: u64,
    sink: Arc<dyn ExecutionEventSink>,
    store: Option<Arc<PersistenceStore>>,
}

impl ExecutionSession {
    fn new(
        run_id: RunId,
        sink: Arc<dyn ExecutionEventSink>,
        store: Option<Arc<PersistenceStore>>,
    ) -> Self {
        Self {
            run_id,
            sequence: 0,
            sink,
            store,
        }
    }

    fn emit(&mut self, kind: RunEventKind) -> Result<(), ExecutionError> {
        self.sequence += 1;
        let timestamp = Utc::now();
        let event = RunEvent {
            run_id: self.run_id.clone(),
            sequence: self.sequence,
            timestamp,
            event: kind,
        };
        if let Some(store) = &self.store {
            let (stream, message) = match &event.event {
                RunEventKind::Output { stream, message } => {
                    (map_log_stream(*stream), message.clone())
                }
                RunEventKind::StateChanged { status } => {
                    (RunLogStream::System, format!("state:{status:?}"))
                }
                RunEventKind::Exit { code } => (RunLogStream::System, format!("exit:{code:?}")),
            };
            store
                .append_run_log(
                    &self.run_id,
                    &RunLogEntry {
                        sequence: self.sequence,
                        timestamp,
                        stream,
                        message,
                    },
                )
                .map_err(|error| {
                    ExecutionError::new(
                        &self.run_id,
                        ExecutionErrorKind::Log,
                        error.to_string(),
                        true,
                    )
                })?;
        }
        self.sink.emit(&event).map_err(|error| {
            ExecutionError::new(&self.run_id, ExecutionErrorKind::EventSink, error, false)
        })
    }
}

fn map_log_stream(stream: OutputStream) -> RunLogStream {
    match stream {
        OutputStream::Stdout => RunLogStream::Stdout,
        OutputStream::Stderr => RunLogStream::Stderr,
        OutputStream::System => RunLogStream::System,
    }
}

enum RawOutcome {
    Exited { code: i32, tail: String },
    Cancelled { tail: String },
    TimedOut { tail: String },
}

enum Completion {
    Exited(std::process::ExitStatus),
    Cancelled,
    TimedOut,
    WaitFailed(std::io::Error),
}

async fn run_command(
    spec: &CommandSpec,
    cancellation: &CancellationToken,
    redactor: &Redactor,
    session: &mut ExecutionSession,
    max_tail_bytes: usize,
) -> Result<RawOutcome, ExecutionError> {
    spec.validate().map_err(|error| {
        ExecutionError::new(
            &session.run_id,
            ExecutionErrorKind::InvalidPlan,
            error.to_string(),
            false,
        )
    })?;

    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    for (name, value) in &spec.env {
        command.env(name, value.expose());
    }

    let mut wrapped = CommandWrap::from(command);
    wrapped.wrap(KillOnDrop);
    #[cfg(unix)]
    wrapped.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    wrapped.wrap(JobObject);
    let child = wrapped.spawn().map_err(|error| {
        ExecutionError::new(
            &session.run_id,
            ExecutionErrorKind::Spawn,
            error.to_string(),
            error.kind() != std::io::ErrorKind::PermissionDenied,
        )
    })?;
    let mut child = ChildGuard::new(child);
    let stdout = child.child_mut().stdout().take().ok_or_else(|| {
        ExecutionError::new(
            &session.run_id,
            ExecutionErrorKind::Output,
            "stdout pipe was not created",
            false,
        )
    })?;
    let stderr = child.child_mut().stderr().take().ok_or_else(|| {
        ExecutionError::new(
            &session.run_id,
            ExecutionErrorKind::Output,
            "stderr pipe was not created",
            false,
        )
    })?;

    let (sender, mut receiver) = mpsc::channel(32);
    tokio::spawn(pump_output(stdout, OutputStream::Stdout, sender.clone()));
    tokio::spawn(pump_output(stderr, OutputStream::Stderr, sender.clone()));
    drop(sender);
    let mut output_open = true;
    let mut tail = TailBuffer::new(max_tail_bytes);
    let deadline = tokio::time::sleep(Duration::from_secs(spec.timeout_seconds));
    tokio::pin!(deadline);

    let completion = {
        let wait = child.child_mut().wait();
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => break Completion::Cancelled,
                _ = &mut deadline => break Completion::TimedOut,
                status = &mut wait => {
                    break match status {
                        Ok(status) => Completion::Exited(status),
                        Err(error) => Completion::WaitFailed(error),
                    };
                }
                output = receiver.recv(), if output_open => {
                    match output {
                        Some(output) => handle_output(output, redactor, session, &mut tail)?,
                        None => output_open = false,
                    }
                }
            }
        }
    };

    match &completion {
        Completion::Cancelled | Completion::TimedOut => {
            child.kill().await.map_err(|error| {
                ExecutionError::new(
                    &session.run_id,
                    ExecutionErrorKind::Kill,
                    error.to_string(),
                    true,
                )
            })?;
        }
        Completion::Exited(_) => child.disarm(),
        Completion::WaitFailed(_) => {}
    }
    while let Some(output) = receiver.recv().await {
        handle_output(output, redactor, session, &mut tail)?;
    }
    let tail = tail.finish();
    match completion {
        Completion::Exited(status) => Ok(RawOutcome::Exited {
            code: status.code().unwrap_or(-1),
            tail,
        }),
        Completion::Cancelled => Ok(RawOutcome::Cancelled { tail }),
        Completion::TimedOut => Ok(RawOutcome::TimedOut { tail }),
        Completion::WaitFailed(error) => Err(ExecutionError::new(
            &session.run_id,
            ExecutionErrorKind::Wait,
            error.to_string(),
            true,
        )),
    }
}

async fn pump_output<R>(mut reader: R, stream: OutputStream, sender: mpsc::Sender<OutputChunk>)
where
    R: AsyncRead + Unpin,
{
    let mut buffer = vec![0_u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                if sender
                    .send(OutputChunk {
                        stream,
                        bytes: Bytes::copy_from_slice(&buffer[..count]),
                        error: None,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                let _ = sender
                    .send(OutputChunk {
                        stream,
                        bytes: Bytes::new(),
                        error: Some(error.to_string()),
                    })
                    .await;
                break;
            }
        }
    }
}

struct OutputChunk {
    stream: OutputStream,
    bytes: Bytes,
    error: Option<String>,
}

fn handle_output(
    output: OutputChunk,
    redactor: &Redactor,
    session: &mut ExecutionSession,
    tail: &mut TailBuffer,
) -> Result<(), ExecutionError> {
    if let Some(error) = output.error {
        return Err(ExecutionError::new(
            &session.run_id,
            ExecutionErrorKind::Output,
            error,
            true,
        ));
    }
    let stripped = strip_ansi_escapes::strip(&output.bytes);
    let text = stripped.as_slice().to_str_lossy();
    let redacted = redactor.redact(&text);
    tail.push(redacted.as_bytes());
    session.emit(RunEventKind::Output {
        stream: output.stream,
        message: redacted,
    })
}

struct ChildGuard {
    child: Box<dyn ChildWrapper>,
    armed: bool,
}

impl ChildGuard {
    fn new(child: Box<dyn ChildWrapper>) -> Self {
        Self { child, armed: true }
    }

    fn child_mut(&mut self) -> &mut dyn ChildWrapper {
        self.child.as_mut()
    }

    async fn kill(&mut self) -> std::io::Result<()> {
        let result = Box::into_pin(self.child.kill()).await;
        self.armed = false;
        result
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.child.start_kill();
        }
    }
}

struct TailBuffer {
    chunks: VecDeque<u8>,
    maximum: usize,
}

impl TailBuffer {
    fn new(maximum: usize) -> Self {
        Self {
            chunks: VecDeque::with_capacity(maximum.min(8192)),
            maximum,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.maximum {
            self.chunks.clear();
            self.chunks
                .extend(bytes[bytes.len() - self.maximum..].iter().copied());
            return;
        }
        let overflow = self
            .chunks
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(self.maximum);
        self.chunks.drain(..overflow);
        self.chunks.extend(bytes.iter().copied());
    }

    fn finish(self) -> String {
        let bytes: Vec<_> = self.chunks.into_iter().collect();
        bytes.as_slice().to_str_lossy().into_owned()
    }
}

fn combine_tails(first: &str, second: &str, maximum: usize) -> String {
    let mut tail = TailBuffer::new(maximum);
    tail.push(first.as_bytes());
    tail.push(second.as_bytes());
    tail.finish()
}

struct Redactor {
    secret_values: Vec<String>,
}

impl Redactor {
    fn new(command: &CommandSpec) -> Self {
        let mut secret_values: Vec<_> = command
            .env
            .values()
            .filter(|value| value.is_sensitive() && !value.expose().is_empty())
            .map(|value| value.expose().to_owned())
            .collect();
        if let Some(post_check) = &command.post_check {
            secret_values.extend(
                post_check
                    .env
                    .values()
                    .filter(|value| value.is_sensitive() && !value.expose().is_empty())
                    .map(|value| value.expose().to_owned()),
            );
        }
        secret_values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        secret_values.dedup();
        Self { secret_values }
    }

    fn redact(&self, text: &str) -> String {
        static NAMED_SECRET: OnceLock<Regex> = OnceLock::new();
        let regex = NAMED_SECRET.get_or_init(|| {
            Regex::new(
                r"(?i)\b([A-Z0-9_]*(?:TOKEN|KEY|SECRET|PASSWORD)[A-Z0-9_]*)\s*[:=]\s*[^\s,;]+",
            )
            .expect("embedded secret redaction regex is valid")
        });
        let mut redacted = regex.replace_all(text, "$1=[REDACTED]").into_owned();
        for secret in &self.secret_values {
            redacted = redacted.replace(secret, "[REDACTED]");
        }
        redacted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ComponentId, InstallationId, StrategyId, ToolId};
    use crate::executor::{ConfirmationContext, EnvironmentValue, PlanContext, UpdatePlan};
    use crate::persistence::{AppPaths, PersistenceStore};
    use chrono::Duration as ChronoDuration;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn fixture(temporary: &TempDir) -> PathBuf {
        let directory = temporary.path().join("path with spaces");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("fake command");
        std::fs::write(
            &path,
            include_bytes!("../../tests/fixtures/fake-command.sh"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn confirmed(command: CommandSpec) -> ConfirmedPlan {
        let now = Utc::now();
        let plan = UpdatePlan::new(
            ToolId::new("fixture").unwrap(),
            InstallationId::new("fixture:command").unwrap(),
            ComponentId::new("core").unwrap(),
            StrategyId::new("fixture").unwrap(),
            command,
            PlanContext {
                config_revision: "config".into(),
                environment_revision: "environment".into(),
                version_snapshot: "version".into(),
                now,
                ttl: ChronoDuration::minutes(5),
            },
        )
        .unwrap();
        plan.confirm(
            plan.plan_id,
            &plan.plan_hash,
            &ConfirmationContext {
                config_revision: "config".into(),
                environment_revision: "environment".into(),
                version_snapshot: "version".into(),
                now,
            },
        )
        .unwrap()
    }

    fn executor(
        temporary: &TempDir,
        sink: Arc<dyn ExecutionEventSink>,
        tail: usize,
    ) -> CommandExecutor {
        let store = Arc::new(PersistenceStore::new(AppPaths::new(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("logs"),
        )));
        CommandExecutor::new(Some(store), sink, tail)
    }

    fn run_id() -> RunId {
        RunId::new(uuid::Uuid::new_v4().to_string()).unwrap()
    }

    #[tokio::test]
    async fn streams_stdout_stderr_strips_ansi_and_handles_nonzero_exit() {
        let temporary = TempDir::new().unwrap();
        let program = fixture(&temporary);
        let sink = Arc::new(MemoryEventSink::default());
        let success = executor(&temporary, sink.clone(), 4096)
            .execute(
                run_id(),
                &confirmed(CommandSpec::new(&program, vec!["streams".into()])),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(success.status, RunStatus::Succeeded);
        assert!(success.output_tail.contains("stdout-one"));
        assert!(!success.output_tail.contains("\u{1b}"));
        assert!(sink.events().iter().any(|event| matches!(
            event.event,
            RunEventKind::Output {
                stream: OutputStream::Stderr,
                ..
            }
        )));

        let failed = executor(&temporary, Arc::new(NoopEventSink), 4096)
            .execute(
                run_id(),
                &confirmed(CommandSpec::new(program, vec!["fail".into()])),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(failed.status, RunStatus::Failed);
        assert_eq!(failed.exit_code, Some(7));
    }

    #[tokio::test]
    async fn paths_and_shell_metacharacter_arguments_execute_as_literal_argv() {
        let temporary = TempDir::new().unwrap();
        let program = fixture(&temporary);
        let values = ["space value", "quote'value", ";", "$(never)", "`never`"];
        let mut args = vec!["args".into()];
        args.extend(values.iter().map(ToString::to_string));
        let result = executor(&temporary, Arc::new(NoopEventSink), 4096)
            .execute(
                run_id(),
                &confirmed(CommandSpec::new(program, args)),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        for value in values {
            assert!(result.output_tail.contains(&format!("<{value}>")));
        }
    }

    #[tokio::test]
    async fn invalid_utf8_and_long_output_are_bounded_in_memory_but_complete_on_disk() {
        let temporary = TempDir::new().unwrap();
        let program = fixture(&temporary);
        let invalid = executor(&temporary, Arc::new(NoopEventSink), 1024)
            .execute(
                run_id(),
                &confirmed(CommandSpec::new(&program, vec!["invalid-utf8".into()])),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(invalid.output_tail.contains('�'));

        let long_run = run_id();
        let command_executor = executor(&temporary, Arc::new(NoopEventSink), 512);
        let long = command_executor
            .execute(
                long_run.clone(),
                &confirmed(CommandSpec::new(program, vec!["long".into()])),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(long.output_tail.len() <= 515);
        let log_path = temporary
            .path()
            .join("logs/runs")
            .join(format!("{long_run}.jsonl"));
        assert!(std::fs::metadata(log_path).unwrap().len() > 100_000);
    }

    #[tokio::test]
    async fn environment_and_output_secrets_are_redacted_from_events_and_logs() {
        let temporary = TempDir::new().unwrap();
        let program = fixture(&temporary);
        let sink = Arc::new(MemoryEventSink::default());
        let run_id = run_id();
        let mut command = CommandSpec::new(program, vec!["secret".into()]);
        command
            .env
            .insert("TOKEN".into(), EnvironmentValue::secret("super-secret"));
        executor(&temporary, sink.clone(), 4096)
            .execute(
                run_id.clone(),
                &confirmed(command),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let events = format!("{:?}", sink.events());
        assert!(!events.contains("super-secret"));
        assert!(events.contains("[REDACTED]"));
        let log = std::fs::read_to_string(
            temporary
                .path()
                .join("logs/runs")
                .join(format!("{run_id}.jsonl")),
        )
        .unwrap();
        assert!(!log.contains("super-secret"));
    }

    #[tokio::test]
    async fn post_check_failure_is_partial_not_success() {
        let temporary = TempDir::new().unwrap();
        let program = fixture(&temporary);
        let mut command = CommandSpec::new(&program, vec!["streams".into()]);
        command.post_check = Some(Box::new(CommandSpec::new(program, vec!["fail".into()])));
        let result = executor(&temporary, Arc::new(NoopEventSink), 4096)
            .execute(run_id(), &confirmed(command), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.status, RunStatus::Partial);
        assert!(matches!(
            result.verification,
            Some(VerificationStatus::VerificationFailed { .. })
        ));
    }

    async fn wait_for_pid_files(prefix: &Path) -> (i32, i32) {
        let parent_file = PathBuf::from(format!("{}.parent", prefix.display()));
        let child_file = PathBuf::from(format!("{}.child", prefix.display()));
        for _ in 0..100 {
            if parent_file.exists() && child_file.exists() {
                let parent = std::fs::read_to_string(&parent_file)
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                let child = std::fs::read_to_string(&child_file)
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                return (parent, child);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("fixture did not write PID files")
    }

    fn process_exists(pid: i32) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    async fn assert_processes_gone(parent: i32, child: i32) {
        for _ in 0..100 {
            if !process_exists(parent) && !process_exists(child) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!process_exists(parent), "parent process remains");
        assert!(!process_exists(child), "child process remains");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_and_cancellation_clean_the_process_group() {
        let temporary = TempDir::new().unwrap();
        let program = fixture(&temporary);

        let timeout_prefix = temporary.path().join("timeout");
        let mut timeout_command = CommandSpec::new(
            &program,
            vec!["sleep-tree".into(), timeout_prefix.display().to_string()],
        );
        timeout_command.timeout_seconds = 1;
        let timeout_task = tokio::spawn({
            let executor = executor(&temporary, Arc::new(NoopEventSink), 1024);
            let confirmed = confirmed(timeout_command);
            async move {
                executor
                    .execute(run_id(), &confirmed, CancellationToken::new())
                    .await
                    .unwrap()
            }
        });
        let timeout_pids = wait_for_pid_files(&timeout_prefix).await;
        assert_eq!(timeout_task.await.unwrap().status, RunStatus::TimedOut);
        assert_processes_gone(timeout_pids.0, timeout_pids.1).await;

        let cancel_prefix = temporary.path().join("cancel");
        let cancel_command = CommandSpec::new(
            program,
            vec!["sleep-tree".into(), cancel_prefix.display().to_string()],
        );
        let cancellation = CancellationToken::new();
        let cancel_task = tokio::spawn({
            let executor = executor(&temporary, Arc::new(NoopEventSink), 1024);
            let confirmed = confirmed(cancel_command);
            let cancellation = cancellation.clone();
            async move {
                executor
                    .execute(run_id(), &confirmed, cancellation)
                    .await
                    .unwrap()
            }
        });
        let cancel_pids = wait_for_pid_files(&cancel_prefix).await;
        cancellation.cancel();
        assert_eq!(cancel_task.await.unwrap().status, RunStatus::Cancelled);
        assert_processes_gone(cancel_pids.0, cancel_pids.1).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_execution_future_uses_process_wrap_group_cleanup() {
        let temporary = TempDir::new().unwrap();
        let program = fixture(&temporary);
        let prefix = temporary.path().join("drop");
        let command = CommandSpec::new(
            program,
            vec!["sleep-tree".into(), prefix.display().to_string()],
        );
        let task = tokio::spawn({
            let executor = executor(&temporary, Arc::new(NoopEventSink), 1024);
            let confirmed = confirmed(command);
            async move {
                let _ = executor
                    .execute(run_id(), &confirmed, CancellationToken::new())
                    .await;
            }
        });
        let pids = wait_for_pid_files(&prefix).await;
        task.abort();
        let _ = task.await;
        assert_processes_gone(pids.0, pids.1).await;
    }
}
