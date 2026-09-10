use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::domain::{Run, RunId, RunStatus, RunSummary, ToolId};
use crate::executor::{CommandExecutor, ConfirmedPlan, ExecutionResult};
use crate::persistence::RunLogEntry;
use crate::persistence::{PersistenceStore, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BatchId(Uuid);

impl BatchId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BatchId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for BatchId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for BatchId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItemDescriptor {
    pub tool_id: ToolId,
    pub installation_id: crate::domain::InstallationId,
}

pub trait RunSchedulingPolicy: Send + Sync {
    fn name(&self) -> &str;

    fn max_concurrency(&self) -> usize;

    fn exclusion_key(&self, item: &QueueItemDescriptor) -> String;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalSerialPolicy;

impl RunSchedulingPolicy for GlobalSerialPolicy {
    fn name(&self) -> &str {
        "global_serial"
    }

    fn max_concurrency(&self) -> usize {
        1
    }

    fn exclusion_key(&self, _item: &QueueItemDescriptor) -> String {
        "global".into()
    }
}

pub trait RunRecoveryHandler: Send + Sync {
    fn recheck_tools(&self, tool_ids: &BTreeSet<ToolId>);
}

#[derive(Debug, Default)]
pub struct NoopRunRecoveryHandler;

impl RunRecoveryHandler for NoopRunRecoveryHandler {
    fn recheck_tools(&self, _tool_ids: &BTreeSet<ToolId>) {}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnqueuedBatch {
    pub batch_id: BatchId,
    pub run_ids: Vec<RunId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Partial,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchCounts {
    pub queued: usize,
    pub running: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub timed_out: usize,
    pub interrupted: usize,
    pub partial: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchSummary {
    pub batch_id: BatchId,
    pub run_ids: Vec<RunId>,
    pub status: BatchStatus,
    pub counts: BatchCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub run: Run,
    pub execution: Option<ExecutionResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelDisposition {
    CancelledQueued,
    CancellationRequested,
    AlreadyFinished,
}

struct QueueEntry {
    run: Run,
    plan: ConfirmedPlan,
    cancellation: CancellationToken,
    execution: Option<ExecutionResult>,
    _exclusion_key: String,
}

#[derive(Default)]
struct QueueState {
    entries: BTreeMap<RunId, QueueEntry>,
    order: VecDeque<RunId>,
    batches: BTreeMap<BatchId, Vec<RunId>>,
    worker_running: bool,
    worker_error: Option<String>,
}

pub struct RunQueue {
    executor: CommandExecutor,
    store: Arc<PersistenceStore>,
    policy: Arc<dyn RunSchedulingPolicy>,
    state: Mutex<QueueState>,
    persistence: Mutex<()>,
    changed: Notify,
    recovered_run_ids: Vec<RunId>,
}

impl RunQueue {
    pub fn new(
        executor: CommandExecutor,
        store: Arc<PersistenceStore>,
    ) -> Result<Arc<Self>, RunQueueError> {
        Self::with_policy_and_recovery(
            executor,
            store,
            Arc::new(GlobalSerialPolicy),
            Arc::new(NoopRunRecoveryHandler),
        )
    }

    pub fn with_policy_and_recovery(
        executor: CommandExecutor,
        store: Arc<PersistenceStore>,
        policy: Arc<dyn RunSchedulingPolicy>,
        recovery_handler: Arc<dyn RunRecoveryHandler>,
    ) -> Result<Arc<Self>, RunQueueError> {
        if policy.max_concurrency() != 1 {
            return Err(RunQueueError::UnsupportedConcurrency {
                policy: policy.name().into(),
                requested: policy.max_concurrency(),
            });
        }
        let recovered_run_ids = recover_unfinished(&store, recovery_handler.as_ref())?;
        Ok(Arc::new(Self {
            executor,
            store,
            policy,
            state: Mutex::new(QueueState::default()),
            persistence: Mutex::new(()),
            changed: Notify::new(),
            recovered_run_ids,
        }))
    }

    pub fn policy_name(&self) -> &str {
        self.policy.name()
    }

    pub fn max_concurrency(&self) -> usize {
        self.policy.max_concurrency()
    }

    pub fn recovered_run_ids(&self) -> &[RunId] {
        &self.recovered_run_ids
    }

    pub async fn enqueue(self: &Arc<Self>, plan: ConfirmedPlan) -> Result<RunId, RunQueueError> {
        Ok(self.enqueue_batch(vec![plan]).await?.run_ids.remove(0))
    }

    pub async fn enqueue_batch(
        self: &Arc<Self>,
        plans: Vec<ConfirmedPlan>,
    ) -> Result<EnqueuedBatch, RunQueueError> {
        self.enqueue_plans(plans, None).await
    }

    async fn enqueue_plans(
        self: &Arc<Self>,
        plans: Vec<ConfirmedPlan>,
        retry_of: Option<RunId>,
    ) -> Result<EnqueuedBatch, RunQueueError> {
        if plans.is_empty() {
            return Err(RunQueueError::EmptyBatch);
        }
        let batch_id = BatchId::new();
        let now = Utc::now();
        let mut entries = Vec::with_capacity(plans.len());
        for plan in plans {
            let run_id = RunId::new(Uuid::new_v4().to_string()).expect("a UUID is a valid run ID");
            let descriptor = QueueItemDescriptor {
                tool_id: plan.plan().tool_id.clone(),
                installation_id: plan.plan().installation_id.clone(),
            };
            let run = Run {
                id: run_id.clone(),
                tool_id: Some(descriptor.tool_id.clone()),
                subject: Default::default(),
                operation: None,
                resource_names: vec![],
                component_ids: vec![plan.plan().component_id.clone()],
                status: RunStatus::Queued,
                created_at: now,
                started_at: None,
                finished_at: None,
                batch_id: Some(batch_id.to_string()),
                retry_of: retry_of.clone(),
                log_path: Some(
                    self.store
                        .paths()
                        .runs_dir()
                        .join(format!("{run_id}.jsonl")),
                ),
                summary: None,
            };
            entries.push(QueueEntry {
                run,
                plan,
                cancellation: CancellationToken::new(),
                execution: None,
                _exclusion_key: self.policy.exclusion_key(&descriptor),
            });
        }
        let records: Vec<_> = entries.iter().map(|entry| entry.run.clone()).collect();
        self.persist_runs(&records).await?;
        let run_ids: Vec<_> = records.iter().map(|run| run.id.clone()).collect();
        let should_start = {
            let mut state = self.state.lock().await;
            for entry in entries {
                state.order.push_back(entry.run.id.clone());
                state.entries.insert(entry.run.id.clone(), entry);
            }
            state.batches.insert(batch_id, run_ids.clone());
            if state.worker_running {
                false
            } else {
                state.worker_running = true;
                true
            }
        };
        self.changed.notify_waiters();
        if should_start {
            let queue = Arc::clone(self);
            tokio::spawn(async move { queue.worker().await });
        }
        Ok(EnqueuedBatch { batch_id, run_ids })
    }

    pub async fn cancel(&self, run_id: &RunId) -> Result<CancelDisposition, RunQueueError> {
        let mut persist = None;
        let disposition = {
            let mut state = self.state.lock().await;
            let entry = state
                .entries
                .get_mut(run_id)
                .ok_or_else(|| RunQueueError::UnknownRun(run_id.clone()))?;
            match entry.run.status {
                RunStatus::Queued => {
                    entry.run.status = RunStatus::Cancelled;
                    entry.run.finished_at = Some(Utc::now());
                    entry.run.summary = Some(RunSummary {
                        exit_code: None,
                        output_tail: String::new(),
                        verification: None,
                        error: Some("cancelled before execution".into()),
                    });
                    persist = Some(entry.run.clone());
                    CancelDisposition::CancelledQueued
                }
                RunStatus::Running => {
                    entry.cancellation.cancel();
                    CancelDisposition::CancellationRequested
                }
                _ => CancelDisposition::AlreadyFinished,
            }
        };
        if let Some(run) = persist {
            self.persist_runs(&[run]).await?;
        }
        self.changed.notify_waiters();
        Ok(disposition)
    }

    pub async fn cancel_batch(&self, batch_id: BatchId) -> Result<(), RunQueueError> {
        let run_ids = {
            let state = self.state.lock().await;
            state
                .batches
                .get(&batch_id)
                .cloned()
                .ok_or(RunQueueError::UnknownBatch(batch_id))?
        };
        for run_id in run_ids {
            self.cancel(&run_id).await?;
        }
        Ok(())
    }

    pub async fn retry(self: &Arc<Self>, run_id: &RunId) -> Result<RunId, RunQueueError> {
        let plan = {
            let state = self.state.lock().await;
            let entry = state
                .entries
                .get(run_id)
                .ok_or_else(|| RunQueueError::UnknownRun(run_id.clone()))?;
            ensure_retryable(&entry.run)?;
            entry.plan.clone()
        };
        Ok(self
            .enqueue_plans(vec![plan], Some(run_id.clone()))
            .await?
            .run_ids
            .remove(0))
    }

    pub async fn retry_with_plan(
        self: &Arc<Self>,
        run_id: &RunId,
        plan: ConfirmedPlan,
    ) -> Result<RunId, RunQueueError> {
        let original = self
            .history()?
            .into_iter()
            .find(|run| &run.id == run_id)
            .ok_or_else(|| RunQueueError::UnknownRun(run_id.clone()))?;
        ensure_retryable(&original)?;
        Ok(self
            .enqueue_plans(vec![plan], Some(run_id.clone()))
            .await?
            .run_ids
            .remove(0))
    }

    pub async fn run(&self, run_id: &RunId) -> Result<RunOutcome, RunQueueError> {
        if let Some(outcome) = {
            let state = self.state.lock().await;
            state.entries.get(run_id).map(|entry| RunOutcome {
                run: entry.run.clone(),
                execution: entry.execution.clone(),
            })
        } {
            return Ok(outcome);
        }
        self.history()?
            .into_iter()
            .find(|run| &run.id == run_id)
            .map(|run| RunOutcome {
                run,
                execution: None,
            })
            .ok_or_else(|| RunQueueError::UnknownRun(run_id.clone()))
    }

    pub async fn wait_run(&self, run_id: &RunId) -> Result<RunOutcome, RunQueueError> {
        loop {
            let notified = self.changed.notified();
            let outcome = self.run(run_id).await?;
            if outcome.run.status.is_terminal() {
                return Ok(outcome);
            }
            if let Some(error) = self.state.lock().await.worker_error.clone() {
                return Err(RunQueueError::Worker(error));
            }
            notified.await;
        }
    }

    pub async fn batch_summary(&self, batch_id: BatchId) -> Result<BatchSummary, RunQueueError> {
        let current_run_ids = {
            let state = self.state.lock().await;
            state.batches.get(&batch_id).cloned()
        };
        let run_ids = match current_run_ids {
            Some(run_ids) => run_ids,
            None => {
                let batch_id_string = batch_id.to_string();
                let run_ids: Vec<_> = self
                    .history()?
                    .into_iter()
                    .filter(|run| run.batch_id.as_deref() == Some(batch_id_string.as_str()))
                    .map(|run| run.id)
                    .collect();
                if run_ids.is_empty() {
                    return Err(RunQueueError::UnknownBatch(batch_id));
                }
                run_ids
            }
        };
        let mut counts = BatchCounts::default();
        for run_id in &run_ids {
            let outcome = self.run(run_id).await?;
            match outcome.run.status {
                RunStatus::Queued => counts.queued += 1,
                RunStatus::Running => counts.running += 1,
                RunStatus::Succeeded => counts.succeeded += 1,
                RunStatus::Failed => counts.failed += 1,
                RunStatus::Cancelled => counts.cancelled += 1,
                RunStatus::TimedOut => counts.timed_out += 1,
                RunStatus::Interrupted => counts.interrupted += 1,
                RunStatus::Partial => counts.partial += 1,
            }
        }
        let status = aggregate_batch_status(&counts, run_ids.len());
        Ok(BatchSummary {
            batch_id,
            run_ids,
            status,
            counts,
        })
    }

    pub async fn wait_batch(&self, batch_id: BatchId) -> Result<BatchSummary, RunQueueError> {
        loop {
            let notified = self.changed.notified();
            let summary = self.batch_summary(batch_id).await?;
            if summary.counts.queued == 0 && summary.counts.running == 0 {
                return Ok(summary);
            }
            if let Some(error) = self.state.lock().await.worker_error.clone() {
                return Err(RunQueueError::Worker(error));
            }
            notified.await;
        }
    }

    pub fn history(&self) -> Result<Vec<Run>, RunQueueError> {
        let mut runs = self.store.load_state()?.value.runs;
        runs.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(runs)
    }

    pub fn run_log(&self, run_id: &RunId) -> Result<Vec<RunLogEntry>, RunQueueError> {
        self.store.read_run_log(run_id).map_err(Into::into)
    }

    async fn worker(self: Arc<Self>) {
        loop {
            let next = {
                let mut state = self.state.lock().await;
                let mut next = None;
                while let Some(run_id) = state.order.pop_front() {
                    let entry = state
                        .entries
                        .get_mut(&run_id)
                        .expect("queued run has an entry");
                    if entry.run.status != RunStatus::Queued {
                        continue;
                    }
                    entry.run.status = RunStatus::Running;
                    entry.run.started_at = Some(Utc::now());
                    next = Some((
                        run_id,
                        entry.plan.clone(),
                        entry.cancellation.clone(),
                        entry.run.clone(),
                    ));
                    break;
                }
                if next.is_none() {
                    state.worker_running = false;
                }
                next
            };
            let Some((run_id, plan, cancellation, running_record)) = next else {
                self.changed.notify_waiters();
                return;
            };
            if let Err(error) = self.persist_runs(&[running_record]).await {
                self.fail_worker_run(&run_id, error.to_string()).await;
                continue;
            }
            self.changed.notify_waiters();

            let execution = self
                .executor
                .execute(run_id.clone(), &plan, cancellation)
                .await;
            let final_record = {
                let mut state = self.state.lock().await;
                let entry = state
                    .entries
                    .get_mut(&run_id)
                    .expect("running run has an entry");
                entry.run.finished_at = Some(Utc::now());
                match execution {
                    Ok(result) => {
                        entry.run.status = result.status;
                        entry.run.summary = Some(RunSummary {
                            exit_code: result.exit_code,
                            output_tail: result.output_tail.clone(),
                            verification: result.verification.clone(),
                            error: None,
                        });
                        entry.execution = Some(result);
                    }
                    Err(error) => {
                        entry.run.status = RunStatus::Failed;
                        entry.run.summary = Some(RunSummary {
                            exit_code: None,
                            output_tail: String::new(),
                            verification: None,
                            error: Some(error.summary.to_string()),
                        });
                    }
                }
                entry.run.clone()
            };
            if let Err(error) = self.persist_runs(&[final_record]).await {
                self.state.lock().await.worker_error = Some(error.to_string());
            }
            self.changed.notify_waiters();
        }
    }

    async fn fail_worker_run(&self, run_id: &RunId, summary: String) {
        let record = {
            let mut state = self.state.lock().await;
            state.worker_error = Some(summary.clone());
            let entry = state
                .entries
                .get_mut(run_id)
                .expect("running run has an entry");
            entry.run.status = RunStatus::Failed;
            entry.run.finished_at = Some(Utc::now());
            entry.run.summary = Some(RunSummary {
                exit_code: None,
                output_tail: String::new(),
                verification: None,
                error: Some(summary),
            });
            entry.run.clone()
        };
        let _ = self.persist_runs(&[record]).await;
        self.changed.notify_waiters();
    }

    async fn persist_runs(&self, records: &[Run]) -> Result<(), RunQueueError> {
        let _persistence = self.persistence.lock().await;
        self.store.update_cached_state(|cached| {
            for record in records {
                if let Some(existing) = cached.runs.iter_mut().find(|run| run.id == record.id) {
                    existing.clone_from(record);
                } else {
                    cached.runs.push(record.clone());
                }
            }
        })?;
        Ok(())
    }
}

fn ensure_retryable(run: &Run) -> Result<(), RunQueueError> {
    if matches!(
        run.status,
        RunStatus::Failed | RunStatus::TimedOut | RunStatus::Interrupted | RunStatus::Partial
    ) {
        Ok(())
    } else {
        Err(RunQueueError::RunNotRetryable {
            run_id: run.id.clone(),
            status: run.status,
        })
    }
}

fn aggregate_batch_status(counts: &BatchCounts, total: usize) -> BatchStatus {
    if counts.running > 0 {
        return BatchStatus::Running;
    }
    if counts.queued > 0 {
        return BatchStatus::Queued;
    }
    if counts.succeeded == total {
        return BatchStatus::Succeeded;
    }
    if counts.cancelled == total {
        return BatchStatus::Cancelled;
    }
    if counts.failed + counts.timed_out + counts.interrupted == total {
        return BatchStatus::Failed;
    }
    BatchStatus::Partial
}

fn recover_unfinished(
    store: &PersistenceStore,
    recovery_handler: &dyn RunRecoveryHandler,
) -> Result<Vec<RunId>, RunQueueError> {
    let loaded = store.load_state()?;
    let mut cached = loaded.value;
    let mut recovered = Vec::new();
    let mut tools = BTreeSet::new();
    let now = Utc::now();
    for run in &mut cached.runs {
        if matches!(run.status, RunStatus::Queued | RunStatus::Running) {
            run.status = RunStatus::Interrupted;
            run.finished_at = Some(now);
            run.summary = Some(RunSummary {
                exit_code: None,
                output_tail: String::new(),
                verification: None,
                error: Some("application restarted before the run completed".into()),
            });
            recovered.push(run.id.clone());
            if let Some(tool) = &run.tool_id {
                tools.insert(tool.clone());
            }
        }
    }
    if !recovered.is_empty() {
        store.save_state(&loaded.revision, &cached)?;
        recovery_handler.recheck_tools(&tools);
    }
    Ok(recovered)
}

#[derive(Debug, Error)]
pub enum RunQueueError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("batch cannot be empty")]
    EmptyBatch,
    #[error("run {0} is unknown")]
    UnknownRun(RunId),
    #[error("batch {0} is unknown")]
    UnknownBatch(BatchId),
    #[error("run {run_id} with status {status:?} cannot be retried")]
    RunNotRetryable { run_id: RunId, status: RunStatus },
    #[error("scheduling policy {policy} requested unsupported concurrency {requested}")]
    UnsupportedConcurrency { policy: String, requested: usize },
    #[error("queue worker failed: {0}")]
    Worker(String),
}
