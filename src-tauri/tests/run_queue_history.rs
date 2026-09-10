#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use dbox_lib::application::{
    BatchStatus, CancelDisposition, GlobalSerialPolicy, NoopRunRecoveryHandler, RunQueue,
    RunQueueError, RunRecoveryHandler, RunSchedulingPolicy,
};
use dbox_lib::domain::{ComponentId, InstallationId, Run, RunId, RunStatus, StrategyId, ToolId};
use dbox_lib::executor::{
    CommandExecutor, CommandSpec, ConfirmationContext, ConfirmedPlan, NoopEventSink, PlanContext,
    UpdatePlan,
};
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use tempfile::TempDir;

struct Fixture {
    temporary: TempDir,
    store: Arc<PersistenceStore>,
    queue_program: PathBuf,
    process_program: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let queue_program = copy_executable(
            temporary.path(),
            "fake-queue-command",
            "tests/fixtures/fake-queue-command.sh",
        );
        let process_program = copy_executable(
            temporary.path(),
            "fake-process-command",
            "tests/fixtures/fake-command.sh",
        );
        let store = Arc::new(PersistenceStore::new(AppPaths::new(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("logs"),
        )));
        Self {
            temporary,
            store,
            queue_program,
            process_program,
        }
    }

    fn executor(&self) -> CommandExecutor {
        CommandExecutor::new(Some(Arc::clone(&self.store)), Arc::new(NoopEventSink), 4096)
    }

    fn queue(&self) -> Arc<RunQueue> {
        RunQueue::new(self.executor(), Arc::clone(&self.store)).unwrap()
    }

    fn record_plan(&self, tool: &str, item_id: &str, outcome: &str) -> ConfirmedPlan {
        confirmed_plan(
            tool,
            &self.queue_program,
            vec![
                "record".into(),
                self.temporary
                    .path()
                    .join("order.log")
                    .display()
                    .to_string(),
                item_id.into(),
                outcome.into(),
                self.temporary
                    .path()
                    .join("active.lock")
                    .display()
                    .to_string(),
                self.temporary
                    .path()
                    .join("overlap.log")
                    .display()
                    .to_string(),
            ],
        )
    }
}

fn copy_executable(directory: &Path, name: &str, fixture: &str) -> PathBuf {
    let path = directory.join(name);
    fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture), &path).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn confirmed_plan(tool: &str, program: &Path, args: Vec<String>) -> ConfirmedPlan {
    let now = Utc::now();
    let plan = UpdatePlan::new(
        ToolId::new(tool).unwrap(),
        InstallationId::new(format!("fixture:{tool}")).unwrap(),
        ComponentId::new("core").unwrap(),
        StrategyId::new("fixture").unwrap(),
        CommandSpec::new(program, args),
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

async fn wait_for_file(path: &Path) {
    for _ in 0..200 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("fixture did not create {}", path.display());
}

#[tokio::test]
async fn twenty_items_run_serially_in_deterministic_order_without_overlap() {
    let fixture = Fixture::new();
    let queue = fixture.queue();
    assert_eq!(queue.max_concurrency(), 1);
    assert_eq!(queue.policy_name(), "global_serial");
    let plans = (0..20)
        .map(|index| fixture.record_plan("same-tool", &format!("item-{index:02}"), "ok"))
        .collect();
    let batch = queue.enqueue_batch(plans).await.unwrap();
    let summary = queue.wait_batch(batch.batch_id).await.unwrap();

    assert_eq!(summary.status, BatchStatus::Succeeded);
    assert_eq!(summary.counts.succeeded, 20);
    let order = fs::read_to_string(fixture.temporary.path().join("order.log")).unwrap();
    assert_eq!(
        order.lines().collect::<Vec<_>>(),
        (0..20)
            .map(|index| format!("item-{index:02}"))
            .collect::<Vec<_>>()
    );
    assert!(!fixture.temporary.path().join("overlap.log").exists());
    assert_eq!(queue.history().unwrap().len(), 20);
}

#[tokio::test]
async fn one_failure_does_not_block_the_next_item_and_batch_is_partial() {
    let fixture = Fixture::new();
    let queue = fixture.queue();
    let batch = queue
        .enqueue_batch(vec![
            fixture.record_plan("tool-a", "before", "ok"),
            fixture.record_plan("tool-b", "fails", "fail"),
            fixture.record_plan("tool-c", "after", "ok"),
        ])
        .await
        .unwrap();
    let summary = queue.wait_batch(batch.batch_id).await.unwrap();
    assert_eq!(summary.status, BatchStatus::Partial);
    assert_eq!(summary.counts.succeeded, 2);
    assert_eq!(summary.counts.failed, 1);
    assert_eq!(
        fs::read_to_string(fixture.temporary.path().join("order.log"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["before", "fails", "after"]
    );
    assert_eq!(
        queue.run(&batch.run_ids[2]).await.unwrap().run.status,
        RunStatus::Succeeded
    );
    let batch_id = batch.batch_id;
    drop(queue);
    let reconstructed = fixture.queue();
    let reconstructed_summary = reconstructed.batch_summary(batch_id).await.unwrap();
    assert_eq!(reconstructed_summary.status, BatchStatus::Partial);
    assert_eq!(reconstructed_summary.counts.failed, 1);
}

#[tokio::test]
async fn queued_cancel_never_starts_and_same_tool_items_never_overlap() {
    let fixture = Fixture::new();
    let queue = fixture.queue();
    let started = fixture.temporary.path().join("hold.started");
    let release = fixture.temporary.path().join("hold.release");
    let first = confirmed_plan(
        "same-tool",
        &fixture.queue_program,
        vec![
            "hold".into(),
            started.display().to_string(),
            release.display().to_string(),
            "first".into(),
        ],
    );
    let second = fixture.record_plan("same-tool", "must-not-run", "ok");
    let batch = queue.enqueue_batch(vec![first, second]).await.unwrap();
    wait_for_file(&started).await;
    let running_state = fixture.store.load_state().unwrap();
    assert_eq!(
        running_state
            .value
            .runs
            .iter()
            .find(|run| run.id == batch.run_ids[0])
            .unwrap()
            .status,
        RunStatus::Running
    );
    assert_eq!(
        queue.cancel(&batch.run_ids[1]).await.unwrap(),
        CancelDisposition::CancelledQueued
    );
    let persisted = fixture.store.load_state().unwrap();
    let queued_cancel = persisted
        .value
        .runs
        .iter()
        .find(|run| run.id == batch.run_ids[1])
        .unwrap();
    assert_eq!(queued_cancel.status, RunStatus::Cancelled);
    assert!(queued_cancel.started_at.is_none());
    fs::write(&release, "release").unwrap();
    let summary = queue.wait_batch(batch.batch_id).await.unwrap();
    assert_eq!(summary.status, BatchStatus::Partial);
    assert_eq!(summary.counts.succeeded, 1);
    assert_eq!(summary.counts.cancelled, 1);
    assert!(!fixture.temporary.path().join("order.log").exists());
}

async fn wait_for_pid_files(prefix: &Path) -> (i32, i32) {
    let parent_file = PathBuf::from(format!("{}.parent", prefix.display()));
    let child_file = PathBuf::from(format!("{}.child", prefix.display()));
    // A redirected printf creates the file before writing the PID. Wait for a
    // complete numeric payload rather than racing the file's creation.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let parent = fs::read_to_string(&parent_file)
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok());
            let child = fs::read_to_string(&child_file)
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok());
            if let (Some(parent), Some(child)) = (parent, child) {
                return (parent, child);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fixture writes both process IDs")
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
    for _ in 0..200 {
        if !process_exists(parent) && !process_exists(child) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!process_exists(parent), "parent process remains");
    assert!(!process_exists(child), "child process remains");
}

#[tokio::test]
async fn running_cancel_cleans_the_process_tree() {
    let fixture = Fixture::new();
    let queue = fixture.queue();
    let prefix = fixture.temporary.path().join("running-cancel");
    let plan = confirmed_plan(
        "tool",
        &fixture.process_program,
        vec!["sleep-tree".into(), prefix.display().to_string()],
    );
    let run_id = queue.enqueue(plan).await.unwrap();
    let pids = wait_for_pid_files(&prefix).await;
    assert_eq!(
        queue.cancel(&run_id).await.unwrap(),
        CancelDisposition::CancellationRequested
    );
    let outcome = queue.wait_run(&run_id).await.unwrap();
    assert_eq!(outcome.run.status, RunStatus::Cancelled);
    assert_processes_gone(pids.0, pids.1).await;
}

#[derive(Default)]
struct RecordingRecoveryHandler {
    calls: Mutex<Vec<BTreeSet<ToolId>>>,
}

impl RunRecoveryHandler for RecordingRecoveryHandler {
    fn recheck_tools(&self, tool_ids: &BTreeSet<ToolId>) {
        self.calls.lock().unwrap().push(tool_ids.clone());
    }
}

fn persisted_run(id: &str, tool: &str, status: RunStatus) -> Run {
    Run {
        id: RunId::new(id).unwrap(),
        tool_id: Some(ToolId::new(tool).unwrap()),
        subject: Default::default(),
        operation: None,
        resource_names: vec![],
        component_ids: vec![ComponentId::new("core").unwrap()],
        status,
        created_at: Utc::now(),
        started_at: (status == RunStatus::Running).then(Utc::now),
        finished_at: status.is_terminal().then(Utc::now),
        batch_id: Some("persisted-batch".into()),
        retry_of: None,
        log_path: None,
        summary: None,
    }
}

#[test]
fn restart_marks_unfinished_runs_interrupted_and_requests_tool_rechecks() {
    let fixture = Fixture::new();
    let loaded = fixture.store.load_state().unwrap();
    let mut cached = loaded.value;
    cached.runs = vec![
        persisted_run("queued", "tool-a", RunStatus::Queued),
        persisted_run("running", "tool-b", RunStatus::Running),
        persisted_run("finished", "tool-c", RunStatus::Succeeded),
    ];
    fixture.store.save_state(&loaded.revision, &cached).unwrap();
    let handler = Arc::new(RecordingRecoveryHandler::default());
    let queue = RunQueue::with_policy_and_recovery(
        fixture.executor(),
        Arc::clone(&fixture.store),
        Arc::new(GlobalSerialPolicy),
        handler.clone(),
    )
    .unwrap();
    assert_eq!(queue.recovered_run_ids().len(), 2);
    let history = queue.history().unwrap();
    assert_eq!(history[0].status, RunStatus::Interrupted);
    assert_eq!(history[1].status, RunStatus::Interrupted);
    assert_eq!(history[2].status, RunStatus::Succeeded);
    assert!(history[0].summary.as_ref().unwrap().error.is_some());
    assert_eq!(
        handler.calls.lock().unwrap()[0],
        BTreeSet::from([
            ToolId::new("tool-a").unwrap(),
            ToolId::new("tool-b").unwrap()
        ])
    );
}

#[tokio::test]
async fn retry_creates_a_new_run_and_links_the_original() {
    let fixture = Fixture::new();
    let queue = fixture.queue();
    let original = queue
        .enqueue(fixture.record_plan("tool", "original", "fail"))
        .await
        .unwrap();
    assert_eq!(
        queue.wait_run(&original).await.unwrap().run.status,
        RunStatus::Failed
    );
    let retry = queue.retry(&original).await.unwrap();
    assert_ne!(retry, original);
    assert_eq!(
        queue.wait_run(&retry).await.unwrap().run.status,
        RunStatus::Failed
    );
    let retried = queue
        .history()
        .unwrap()
        .into_iter()
        .find(|run| run.id == retry)
        .unwrap();
    assert_eq!(retried.retry_of, Some(original));
    assert!(retried
        .log_path
        .unwrap()
        .ends_with(format!("{retry}.jsonl")));
    assert!(!queue.run_log(&retry).unwrap().is_empty());
}

struct ParallelPolicy;

impl RunSchedulingPolicy for ParallelPolicy {
    fn name(&self) -> &str {
        "future_parallel"
    }

    fn max_concurrency(&self) -> usize {
        2
    }

    fn exclusion_key(&self, item: &dbox_lib::application::QueueItemDescriptor) -> String {
        item.tool_id.to_string()
    }
}

#[test]
fn parallel_policy_interface_is_reserved_but_not_enabled_for_mvp() {
    let fixture = Fixture::new();
    let result = RunQueue::with_policy_and_recovery(
        fixture.executor(),
        Arc::clone(&fixture.store),
        Arc::new(ParallelPolicy),
        Arc::new(NoopRunRecoveryHandler),
    );
    assert!(matches!(
        result,
        Err(RunQueueError::UnsupportedConcurrency { requested: 2, .. })
    ));
}
