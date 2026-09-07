use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dbox_lib::api::{
    export_bindings, ApiErrorCode, ApiEvent, ApiService, BufferedExecutionEventSink,
    CancelRequestDto, ConfirmRequestDto, ManifestInputDto, ManifestReadRequestDto,
    MemoryApiEventEmitter, PreviewRequestDto, RefreshRequestDto, RefreshScopeDto, RunLogRequestDto,
    SaveManifestRequestDto, SaveSettingsRequestDto, SnapshotDto, UpdateSelectionDto,
    API_SCHEMA_REVISION,
};
use dbox_lib::application::{
    ApplicationEnvironment, ApplicationService, ApplicationServiceOptions,
};
use dbox_lib::catalog::Catalog;
use dbox_lib::domain::{ProviderId, RunId, RunStatus};
use dbox_lib::executor::{
    CommandExecutor, ExecutionEventSink, OutputStream, PlanId, RunEvent, RunEventKind,
};
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use dbox_lib::providers::ProviderRegistry;
use tempfile::TempDir;

#[derive(Default)]
struct FakeEnvironment;

impl ApplicationEnvironment for FakeEnvironment {
    fn resolve_program(
        &self,
        name: &str,
        user_override: Option<&Path>,
        _now: chrono::DateTime<Utc>,
    ) -> Result<PathBuf, String> {
        user_override
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("{name} is unavailable"))
    }

    fn revision(&self) -> String {
        "environment-revision".into()
    }
}

struct Fixture {
    _temporary: TempDir,
    store: Arc<PersistenceStore>,
    emitter: Arc<MemoryApiEventEmitter>,
    api: ApiService,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let store = Arc::new(PersistenceStore::new(AppPaths::new(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("logs"),
        )));
        let emitter = Arc::new(MemoryApiEventEmitter::default());
        let event_sink = Arc::new(BufferedExecutionEventSink::new(
            emitter.clone(),
            32,
            Duration::from_millis(50),
        ));
        let application = Arc::new(
            ApplicationService::new(
                Arc::new(ProviderRegistry::new()),
                Catalog::default(),
                Arc::clone(&store),
                CommandExecutor::new(Some(Arc::clone(&store)), event_sink, 4096),
                Arc::new(FakeEnvironment),
                ApplicationServiceOptions::default(),
            )
            .unwrap(),
        );
        let api = ApiService::new(application, Arc::clone(&store), emitter.clone());
        Self {
            _temporary: temporary,
            store,
            emitter,
            api,
        }
    }
}

#[test]
fn snapshot_wire_format_and_generated_bindings_are_stable() {
    let snapshot = SnapshotDto {
        schema_revision: API_SCHEMA_REVISION,
        tools: Vec::new(),
        installations: Vec::new(),
        providers: Vec::new(),
        catalog_diagnostics: Vec::new(),
        catalog_errors: Vec::new(),
        config_revision: "config-revision".into(),
        state_revision: "state-revision".into(),
        environment_revision: "environment-revision".into(),
        refreshed_at: None,
    };
    assert_eq!(
        serde_json::to_string_pretty(&snapshot).unwrap(),
        include_str!("fixtures/api-snapshot-v1.json").trim()
    );

    let temporary = TempDir::new().unwrap();
    let generated = temporary.path().join("bindings.ts");
    export_bindings(&generated).unwrap();
    assert_eq!(
        std::fs::read_to_string(generated).unwrap(),
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("src/bindings.ts")
        )
        .unwrap()
    );
}

#[tokio::test]
async fn handlers_map_not_found_invalid_plan_conflict_and_internal_errors() {
    let fixture = Fixture::new();

    let not_found = fixture
        .api
        .refresh(RefreshRequestDto {
            scope: RefreshScopeDto::Provider {
                provider_id: "missing".into(),
            },
            force: true,
        })
        .await
        .unwrap_err();
    assert_eq!(not_found.code, ApiErrorCode::NotFound);

    let invalid_plan = fixture
        .api
        .confirm(ConfirmRequestDto {
            plan_id: PlanId::new().to_string(),
            plan_hash: "0".repeat(64),
        })
        .await
        .unwrap_err();
    assert_eq!(invalid_plan.code, ApiErrorCode::InvalidPlan);

    let settings = fixture.api.settings().unwrap();
    let mut changed = settings.settings.clone();
    changed.default_timeout_seconds = "42".into();
    fixture
        .api
        .save_settings(SaveSettingsRequestDto {
            expected_revision: settings.revision.clone(),
            settings: changed.clone(),
        })
        .await
        .unwrap();
    let conflict = fixture
        .api
        .save_settings(SaveSettingsRequestDto {
            expected_revision: settings.revision,
            settings: changed,
        })
        .await
        .unwrap_err();
    assert_eq!(conflict.code, ApiErrorCode::Conflict);

    std::fs::write(fixture.store.paths().state_file(), b"not-json").unwrap();
    let internal = fixture.api.run_history().unwrap_err();
    assert_eq!(internal.code, ApiErrorCode::Internal);
}

#[tokio::test]
async fn malformed_ids_and_hashes_are_rejected_before_execution() {
    let fixture = Fixture::new();
    let invalid_tool = fixture
        .api
        .preview(PreviewRequestDto {
            selections: vec![UpdateSelectionDto {
                tool_id: "bad\nidentifier".into(),
                component_id: "core".into(),
                strategy_id: None,
                target_version: None,
            }],
        })
        .await
        .unwrap_err();
    assert_eq!(invalid_tool.code, ApiErrorCode::InvalidRequest);

    let invalid_hash = fixture
        .api
        .confirm(ConfirmRequestDto {
            plan_id: PlanId::new().to_string(),
            plan_hash: "not-a-hash".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(invalid_hash.code, ApiErrorCode::InvalidRequest);

    let invalid_run = fixture
        .api
        .cancel(CancelRequestDto {
            run_id: "bad\nrun".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(invalid_run.code, ApiErrorCode::InvalidRequest);
    assert!(fixture.emitter.events().is_empty());
}

#[tokio::test]
async fn settings_and_manifest_commands_round_trip_with_revisions() {
    let fixture = Fixture::new();
    let before = fixture.api.snapshot().await.unwrap();
    let settings = fixture.api.settings().unwrap();
    let mut changed_settings = settings.settings;
    changed_settings.default_timeout_seconds = "42".into();
    let saved_settings = fixture
        .api
        .save_settings(SaveSettingsRequestDto {
            expected_revision: settings.revision,
            settings: changed_settings,
        })
        .await
        .unwrap();
    let after = fixture.api.snapshot().await.unwrap();
    assert_ne!(before.config_revision, saved_settings.revision);
    assert_eq!(after.config_revision, saved_settings.revision);

    let manifest = "schema_version = 1\nid = \"custom-tool\"\ndisplay_name = \"Custom Tool\"\n";
    let validated = fixture
        .api
        .validate_manifest(ManifestInputDto {
            file_name: "custom-tool.toml".into(),
            contents: manifest.into(),
        })
        .unwrap();
    assert_eq!(validated.manifest_id, "custom-tool");

    let saved = fixture
        .api
        .save_manifest(SaveManifestRequestDto {
            file_name: "custom-tool.toml".into(),
            contents: manifest.into(),
            expected_revision: None,
        })
        .await
        .unwrap();
    assert_eq!(saved.revision.len(), 64);
    let read = fixture
        .api
        .read_manifest(ManifestReadRequestDto {
            file_name: "custom-tool.toml".into(),
        })
        .unwrap();
    assert_eq!(read.revision.as_deref(), Some(saved.revision.as_str()));
    assert_eq!(read.contents.as_deref(), Some(manifest));

    let conflict = fixture
        .api
        .save_manifest(SaveManifestRequestDto {
            file_name: "custom-tool.toml".into(),
            contents: manifest.into(),
            expected_revision: None,
        })
        .await
        .unwrap_err();
    assert_eq!(conflict.code, ApiErrorCode::Conflict);

    let traversal = fixture
        .api
        .read_manifest(ManifestReadRequestDto {
            file_name: "../outside.toml".into(),
        })
        .unwrap_err();
    assert_eq!(traversal.code, ApiErrorCode::InvalidRequest);
}

#[test]
fn output_events_are_batched_ordered_and_isolated_by_run() {
    let emitter = Arc::new(MemoryApiEventEmitter::default());
    let sink = BufferedExecutionEventSink::new(emitter.clone(), 32, Duration::from_secs(60));
    let first = RunId::new("run-first").unwrap();
    let second = RunId::new("run-second").unwrap();
    let at = Utc::now();
    for event in [
        RunEvent {
            run_id: first.clone(),
            sequence: 1,
            timestamp: at,
            event: RunEventKind::Output {
                stream: OutputStream::Stdout,
                message: "first-1".into(),
            },
        },
        RunEvent {
            run_id: second.clone(),
            sequence: 1,
            timestamp: at,
            event: RunEventKind::Output {
                stream: OutputStream::Stderr,
                message: "second-1".into(),
            },
        },
        RunEvent {
            run_id: first.clone(),
            sequence: 2,
            timestamp: at,
            event: RunEventKind::Output {
                stream: OutputStream::Stdout,
                message: "first-2".into(),
            },
        },
        RunEvent {
            run_id: first.clone(),
            sequence: 3,
            timestamp: at,
            event: RunEventKind::Exit { code: Some(0) },
        },
        RunEvent {
            run_id: second.clone(),
            sequence: 2,
            timestamp: at,
            event: RunEventKind::StateChanged {
                status: RunStatus::Failed,
            },
        },
    ] {
        ExecutionEventSink::emit(&sink, &event).unwrap();
    }

    let events = emitter.events();
    assert_eq!(events.len(), 4);
    let ApiEvent::RunOutput(first_output) = &events[0] else {
        panic!("first event should flush the first run output")
    };
    assert_eq!(first_output.run_id, "run-first");
    assert_eq!(first_output.first_sequence, "1");
    assert_eq!(first_output.last_sequence, "2");
    assert_eq!(
        first_output
            .chunks
            .iter()
            .map(|chunk| chunk.message.as_str())
            .collect::<Vec<_>>(),
        ["first-1", "first-2"]
    );
    let ApiEvent::RunOutput(second_output) = &events[2] else {
        panic!("third event should flush the second run output")
    };
    assert_eq!(second_output.run_id, "run-second");
    assert_eq!(second_output.first_sequence, "1");
    assert_eq!(second_output.last_sequence, "1");
}

#[test]
fn capability_does_not_grant_shell_or_process_access() {
    let capability: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let permissions = capability["permissions"].as_array().unwrap();
    assert_eq!(permissions, &[serde_json::json!("core:default")]);
    let serialized = serde_json::to_string(permissions).unwrap();
    assert!(!serialized.contains("shell"));
    assert!(!serialized.contains("process"));
}

#[tokio::test]
async fn unknown_run_log_is_not_found() {
    let fixture = Fixture::new();
    let error = fixture
        .api
        .run_log(RunLogRequestDto {
            run_id: "missing-run".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::NotFound);
}

#[test]
fn refresh_scope_wire_names_are_camel_case() {
    assert_eq!(
        serde_json::to_value(RefreshScopeDto::Provider {
            provider_id: ProviderId::new("npm-global").unwrap().to_string(),
        })
        .unwrap(),
        serde_json::json!({"scope": "provider", "providerId": "npm-global"})
    );
}

#[tokio::test]
async fn quiet_commands_flush_output_before_any_later_output_or_terminal_event() {
    let emitter = Arc::new(MemoryApiEventEmitter::default());
    let sink = BufferedExecutionEventSink::new(emitter.clone(), 32, Duration::from_millis(20));
    let run_id = RunId::new("quiet-run").unwrap();
    ExecutionEventSink::emit(
        &sink,
        &RunEvent {
            run_id: run_id.clone(),
            sequence: 1,
            timestamp: Utc::now(),
            event: RunEventKind::Output {
                stream: OutputStream::Stdout,
                message: "download started".into(),
            },
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while emitter.events().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a quiet command must publish output without waiting for exit");
    let events = emitter.events();
    assert!(
        matches!(&events[0], ApiEvent::RunOutput(output) if output.chunks[0].message == "download started")
    );
    ExecutionEventSink::emit(
        &sink,
        &RunEvent {
            run_id,
            sequence: 2,
            timestamp: Utc::now(),
            event: RunEventKind::StateChanged {
                status: RunStatus::Succeeded,
            },
        },
    )
    .unwrap();
    assert_eq!(
        emitter.events().len(),
        2,
        "terminal flush must not repeat the timed batch"
    );
}
