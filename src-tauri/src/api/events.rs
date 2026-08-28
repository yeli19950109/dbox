use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

use crate::domain::RunId;
use crate::executor::{ExecutionEventSink, OutputStream, RunEvent, RunEventKind};

use super::dto::run_status_name;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RefreshPhaseDto {
    Started,
    Progress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "refresh-progress")]
pub struct RefreshProgressEventDto {
    pub request_id: String,
    pub sequence: String,
    pub phase: RefreshPhaseDto,
    pub message: String,
    pub provider_id: Option<String>,
    pub completed: Option<u32>,
    pub total: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunStateKindDto {
    StateChanged { status: String },
    Exit { code: Option<i32> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "run-state")]
pub struct RunStateEventDto {
    pub run_id: String,
    pub sequence: String,
    pub timestamp: String,
    pub state: RunStateKindDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RunOutputStreamDto {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunOutputChunkDto {
    pub sequence: String,
    pub stream: RunOutputStreamDto,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "run-output")]
pub struct RunOutputEventDto {
    pub run_id: String,
    pub first_sequence: String,
    pub last_sequence: String,
    pub timestamp: String,
    pub chunks: Vec<RunOutputChunkDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "tool-state")]
pub struct ToolStateEventDto {
    pub sequence: String,
    pub state_revision: String,
    pub tool_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiEvent {
    RefreshProgress(RefreshProgressEventDto),
    RunState(RunStateEventDto),
    RunOutput(RunOutputEventDto),
    ToolState(ToolStateEventDto),
}

pub trait ApiEventEmitter: Send + Sync {
    fn emit(&self, event: ApiEvent) -> Result<(), String>;
}

pub struct TauriApiEventEmitter {
    handle: tauri::AppHandle<tauri::Wry>,
}

impl TauriApiEventEmitter {
    pub fn new(handle: tauri::AppHandle<tauri::Wry>) -> Self {
        Self { handle }
    }
}

impl ApiEventEmitter for TauriApiEventEmitter {
    fn emit(&self, event: ApiEvent) -> Result<(), String> {
        use tauri_specta::Event as _;

        let result = match event {
            ApiEvent::RefreshProgress(event) => event.emit(&self.handle),
            ApiEvent::RunState(event) => event.emit(&self.handle),
            ApiEvent::RunOutput(event) => event.emit(&self.handle),
            ApiEvent::ToolState(event) => event.emit(&self.handle),
        };
        result.map_err(|error| error.to_string())
    }
}

#[derive(Debug, Default)]
pub struct MemoryApiEventEmitter {
    events: Mutex<Vec<ApiEvent>>,
}

impl MemoryApiEventEmitter {
    pub fn events(&self) -> Vec<ApiEvent> {
        self.events
            .lock()
            .expect("memory API event mutex is not poisoned")
            .clone()
    }
}

impl ApiEventEmitter for MemoryApiEventEmitter {
    fn emit(&self, event: ApiEvent) -> Result<(), String> {
        self.events
            .lock()
            .map_err(|_| "memory API event mutex is poisoned".to_owned())?
            .push(event);
        Ok(())
    }
}

struct PendingOutput {
    first_sequence: u64,
    last_sequence: u64,
    timestamp: String,
    chunks: Vec<RunOutputChunkDto>,
    last_flush: Instant,
}

pub struct BufferedExecutionEventSink {
    emitter: Arc<dyn ApiEventEmitter>,
    pending: Mutex<BTreeMap<RunId, PendingOutput>>,
    max_chunks: usize,
    max_interval: Duration,
}

impl BufferedExecutionEventSink {
    pub fn new(
        emitter: Arc<dyn ApiEventEmitter>,
        max_chunks: usize,
        max_interval: Duration,
    ) -> Self {
        Self {
            emitter,
            pending: Mutex::new(BTreeMap::new()),
            max_chunks: max_chunks.max(1),
            max_interval,
        }
    }

    fn buffer_output(
        &self,
        event: &RunEvent,
        stream: OutputStream,
        message: &str,
    ) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "run output buffer mutex is poisoned".to_owned())?;
        let entry = pending
            .entry(event.run_id.clone())
            .or_insert_with(|| PendingOutput {
                first_sequence: event.sequence,
                last_sequence: event.sequence,
                timestamp: event.timestamp.to_rfc3339(),
                chunks: Vec::new(),
                last_flush: Instant::now(),
            });
        entry.last_sequence = event.sequence;
        entry.timestamp = event.timestamp.to_rfc3339();
        entry.chunks.push(RunOutputChunkDto {
            sequence: event.sequence.to_string(),
            stream: output_stream(stream),
            message: message.to_owned(),
        });
        let should_flush = entry.chunks.len() >= self.max_chunks
            || entry.last_flush.elapsed() >= self.max_interval;
        if should_flush {
            let batch = pending
                .remove(&event.run_id)
                .expect("the pending run output batch exists");
            drop(pending);
            self.emit_output(&event.run_id, batch)?;
        }
        Ok(())
    }

    fn flush_run(&self, run_id: &RunId) -> Result<(), String> {
        let batch = self
            .pending
            .lock()
            .map_err(|_| "run output buffer mutex is poisoned".to_owned())?
            .remove(run_id);
        if let Some(batch) = batch {
            self.emit_output(run_id, batch)?;
        }
        Ok(())
    }

    fn emit_output(&self, run_id: &RunId, batch: PendingOutput) -> Result<(), String> {
        self.emitter.emit(ApiEvent::RunOutput(RunOutputEventDto {
            run_id: run_id.to_string(),
            first_sequence: batch.first_sequence.to_string(),
            last_sequence: batch.last_sequence.to_string(),
            timestamp: batch.timestamp,
            chunks: batch.chunks,
        }))
    }
}

impl ExecutionEventSink for BufferedExecutionEventSink {
    fn emit(&self, event: &RunEvent) -> Result<(), String> {
        match &event.event {
            RunEventKind::Output { stream, message } => self.buffer_output(event, *stream, message),
            RunEventKind::StateChanged { status } => {
                self.flush_run(&event.run_id)?;
                self.emitter.emit(ApiEvent::RunState(RunStateEventDto {
                    run_id: event.run_id.to_string(),
                    sequence: event.sequence.to_string(),
                    timestamp: event.timestamp.to_rfc3339(),
                    state: RunStateKindDto::StateChanged {
                        status: run_status_name(*status).into(),
                    },
                }))
            }
            RunEventKind::Exit { code } => {
                self.flush_run(&event.run_id)?;
                self.emitter.emit(ApiEvent::RunState(RunStateEventDto {
                    run_id: event.run_id.to_string(),
                    sequence: event.sequence.to_string(),
                    timestamp: event.timestamp.to_rfc3339(),
                    state: RunStateKindDto::Exit { code: *code },
                }))
            }
        }
    }
}

fn output_stream(stream: OutputStream) -> RunOutputStreamDto {
    match stream {
        OutputStream::Stdout => RunOutputStreamDto::Stdout,
        OutputStream::Stderr => RunOutputStreamDto::Stderr,
        OutputStream::System => RunOutputStreamDto::System,
    }
}
