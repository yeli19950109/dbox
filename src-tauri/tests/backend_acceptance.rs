#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dbox_lib::api::{
    ApiErrorCode, ApiEvent, ApiService, BufferedExecutionEventSink, CancelDispositionDto,
    CancelRequestDto, ConfirmBatchOutcomeDto, ConfirmBatchRequestDto, ConfirmRequestDto,
    MemoryApiEventEmitter, PreviewRequestDto, RefreshPhaseDto, RefreshRequestDto, RefreshScopeDto,
    RefreshStageDto, RunLogRequestDto, RunStateKindDto, UpdateSelectionDto,
};
use dbox_lib::application::{
    ApplicationEnvironment, ApplicationService, ApplicationServiceOptions,
};
use dbox_lib::catalog::Catalog;
use dbox_lib::domain::{
    ComponentId, Installation, InstallationScope, PackageCoordinate, PackageKind, ProviderId,
    StrategyId,
};
use dbox_lib::executor::CommandExecutor;
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use dbox_lib::providers::{
    ProviderCapabilities, ProviderError, ProviderStatus, ProviderUpdatePlan, ProviderUpdateRequest,
    ToolProvider, UpdateCheck,
};
use dbox_lib::version::{ComponentStatus, VersionValue};
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
        "acceptance-environment".into()
    }
}

struct AdvancingProvider {
    id: ProviderId,
    program: PathBuf,
    args: Vec<String>,
    scans: AtomicUsize,
}

impl AdvancingProvider {
    fn installation(&self, version: &str) -> Installation {
        let mut installation = Installation::new(
            self.id.clone(),
            PackageCoordinate::new(PackageKind::Other("fixture".into()), "acceptance-tool")
                .unwrap(),
            InstallationScope::Global,
        )
        .unwrap();
        installation.installed_version = Some(VersionValue::new(version));
        installation
    }
}

#[async_trait]
impl ToolProvider for AdvancingProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            full_scan: true,
            check_updates: true,
            update: true,
            ..ProviderCapabilities::default()
        }
    }

    async fn probe(&self) -> Result<ProviderStatus, ProviderError> {
        Ok(ProviderStatus::available())
    }

    async fn scan(&self) -> Result<Vec<Installation>, ProviderError> {
        let version = if self.scans.fetch_add(1, Ordering::SeqCst) == 0 {
            "1.0.0"
        } else {
            "2.0.0"
        };
        Ok(vec![self.installation(version)])
    }

    async fn check_updates(
        &self,
        installations: &[Installation],
    ) -> Result<Vec<UpdateCheck>, ProviderError> {
        let installed = installations[0].installed_version.clone();
        let latest = VersionValue::new("2.0.0");
        Ok(vec![UpdateCheck {
            installation_id: installations[0].id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            installed_version: installed.clone(),
            latest_version: Some(latest.clone()),
            status: ComponentStatus::from_versions(installed.as_ref(), Some(&latest)),
        }])
    }

    async fn plan_update(
        &self,
        request: ProviderUpdateRequest,
    ) -> Result<ProviderUpdatePlan, ProviderError> {
        Ok(ProviderUpdatePlan {
            provider_id: self.id.clone(),
            installation_id: request.installation_id,
            component_id: request.component_id,
            strategy_id: request.strategy_id,
            program: self.program.to_string_lossy().into_owned(),
            args: self.args.clone(),
        })
    }
}

struct Fixture {
    _temporary: TempDir,
    api: Arc<ApiService>,
    emitter: Arc<MemoryApiEventEmitter>,
}

impl Fixture {
    fn new(args: Vec<String>) -> Self {
        Self::with_commands(vec![args], ApplicationServiceOptions::default())
    }

    fn with_commands(commands: Vec<Vec<String>>, options: ApplicationServiceOptions) -> Self {
        let temporary = TempDir::new().unwrap();
        let program = temporary.path().join("fake-command");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-command.sh"),
            &program,
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        let store = Arc::new(PersistenceStore::new(AppPaths::new(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("logs"),
        )));
        let emitter = Arc::new(MemoryApiEventEmitter::default());
        let sink = Arc::new(BufferedExecutionEventSink::new(
            emitter.clone(),
            8,
            Duration::from_millis(10),
        ));
        let mut registry = dbox_lib::providers::ProviderRegistry::new();
        for (index, args) in commands.into_iter().enumerate() {
            let provider = AdvancingProvider {
                id: ProviderId::new(if index == 0 {
                    "acceptance".into()
                } else {
                    format!("z-acceptance-{index}")
                })
                .unwrap(),
                program: program.clone(),
                args,
                scans: AtomicUsize::new(0),
            };
            registry.register(Arc::new(provider)).unwrap();
        }
        let application = Arc::new(
            ApplicationService::new(
                Arc::new(registry),
                Catalog::default(),
                Arc::clone(&store),
                CommandExecutor::new(Some(Arc::clone(&store)), sink, 4096),
                Arc::new(FakeEnvironment),
                options,
            )
            .unwrap(),
        );
        let api = Arc::new(ApiService::new(application, store, emitter.clone()));
        Self {
            _temporary: temporary,
            api,
            emitter,
        }
    }

    async fn preview(&self) -> dbox_lib::api::UpdatePlanDto {
        self.preview_all().await.remove(0)
    }

    async fn preview_all(&self) -> Vec<dbox_lib::api::UpdatePlanDto> {
        let snapshot = self
            .api
            .refresh(RefreshRequestDto {
                scope: RefreshScopeDto::All,
                force: true,
            })
            .await
            .unwrap();
        self.api
            .preview(PreviewRequestDto {
                selections: snapshot
                    .tools
                    .iter()
                    .map(|tool| UpdateSelectionDto {
                        tool_id: tool.id.clone(),
                        component_id: "core".into(),
                        strategy_id: Some(StrategyId::new("provider-default").unwrap().to_string()),
                        target_version: None,
                    })
                    .collect(),
            })
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn batch_continues_after_an_update_replaces_its_command_and_the_preview_expires() {
    let gates = TempDir::new().unwrap();
    let gate = gates.path().join("finish");
    let fixture = Fixture::with_commands(
        vec![
            vec!["replace-self-gated".into(), gate.display().to_string()],
            vec!["streams".into()],
        ],
        ApplicationServiceOptions {
            plan_ttl: chrono::Duration::seconds(2),
            ..Default::default()
        },
    );
    let plans = fixture.preview_all().await;
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0].command.args[0], "replace-self-gated");
    let expires_at = chrono::DateTime::parse_from_rfc3339(&plans[1].expires_at).unwrap();
    let request = ConfirmBatchRequestDto {
        plans: plans
            .iter()
            .map(|plan| ConfirmRequestDto {
                plan_id: plan.plan_id.clone(),
                plan_hash: plan.plan_hash.clone(),
            })
            .collect(),
    };
    let api = fixture.api.clone();
    let batch = tokio::spawn(async move { api.confirm_batch(request).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if fixture.emitter.events().iter().any(|event| matches!(event,
                ApiEvent::RunOutput(output) if output.chunks.iter().any(|chunk| chunk.message.contains("fixture-started"))
            )) { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    let remaining = (expires_at.with_timezone(&Utc) - Utc::now())
        .to_std()
        .unwrap_or_default();
    tokio::time::sleep(remaining + Duration::from_millis(20)).await;
    fs::write(gate, "release").unwrap();
    let response = tokio::time::timeout(Duration::from_secs(3), batch)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.outcomes.len(), 2);
    for outcome in &response.outcomes {
        assert!(
            matches!(outcome, ConfirmBatchOutcomeDto::Ok { execution, .. } if execution.status == "succeeded")
        );
    }
    assert!(
        matches!(&response.outcomes[1], ConfirmBatchOutcomeDto::Ok { execution, .. }
        if execution.output_tail.contains("replacement-command"))
    );
    assert_eq!(fixture.api.run_history().unwrap().runs.len(), 2);
    // An accepted plan is consumed, even if it was waiting behind another command.
    let replay = fixture
        .api
        .confirm(ConfirmRequestDto {
            plan_id: plans[1].plan_id.clone(),
            plan_hash: plans[1].plan_hash.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(replay.code, ApiErrorCode::InvalidPlan);
    assert!(replay.message.contains("already consumed"));
}

#[tokio::test]
async fn invalid_later_plan_rejects_the_whole_batch_before_any_command_starts() {
    let fixture = Fixture::with_commands(
        vec![vec!["streams".into()], vec!["streams".into()]],
        Default::default(),
    );
    let plans = fixture.preview_all().await;
    let error = fixture
        .api
        .confirm_batch(ConfirmBatchRequestDto {
            plans: vec![
                ConfirmRequestDto {
                    plan_id: plans[0].plan_id.clone(),
                    plan_hash: plans[0].plan_hash.clone(),
                },
                ConfirmRequestDto {
                    plan_id: plans[1].plan_id.clone(),
                    plan_hash: "0".repeat(64),
                },
            ],
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::InvalidPlan);
    assert!(fixture.api.run_history().unwrap().runs.is_empty());
    assert!(!fixture
        .emitter
        .events()
        .iter()
        .any(|event| matches!(event, ApiEvent::RunState(_))));
    // Failed validation does not consume otherwise valid plans.
    fixture
        .api
        .confirm(ConfirmRequestDto {
            plan_id: plans[0].plan_id.clone(),
            plan_hash: plans[0].plan_hash.clone(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn command_changes_before_batch_confirmation_are_still_rejected() {
    let fixture = Fixture::with_commands(
        vec![vec!["streams".into()], vec!["streams".into()]],
        Default::default(),
    );
    let plans = fixture.preview_all().await;
    fs::write(
        &plans[0].command.program,
        "#!/bin/sh\necho unexpected-change\n",
    )
    .unwrap();
    let error = fixture
        .api
        .confirm_batch(ConfirmBatchRequestDto {
            plans: plans
                .iter()
                .map(|plan| ConfirmRequestDto {
                    plan_id: plan.plan_id.clone(),
                    plan_hash: plan.plan_hash.clone(),
                })
                .collect(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::InvalidPlan);
    assert_eq!(error.details["reason"], "environment_changed");
    assert!(fixture.api.run_history().unwrap().runs.is_empty());
}

#[tokio::test]
async fn failed_command_does_not_stop_later_approved_updates() {
    let fixture = Fixture::with_commands(
        vec![vec!["fail".into()], vec!["streams".into()]],
        Default::default(),
    );
    let plans = fixture.preview_all().await;
    let response = fixture
        .api
        .confirm_batch(ConfirmBatchRequestDto {
            plans: plans
                .iter()
                .map(|plan| ConfirmRequestDto {
                    plan_id: plan.plan_id.clone(),
                    plan_hash: plan.plan_hash.clone(),
                })
                .collect(),
        })
        .await
        .unwrap();
    assert!(
        matches!(&response.outcomes[0], ConfirmBatchOutcomeDto::Ok { execution, .. } if execution.status == "failed")
    );
    assert!(
        matches!(&response.outcomes[1], ConfirmBatchOutcomeDto::Ok { execution, .. } if execution.status == "succeeded")
    );
}

#[tokio::test]
async fn typed_api_completes_refresh_preview_confirm_post_check_history_chain() {
    let fixture = Fixture::new(vec!["streams".into()]);
    let plan = fixture.preview().await;
    assert!(plan.command.program.contains("fake-command"));
    assert_eq!(plan.command.args, ["streams"]);

    let confirmed = fixture
        .api
        .confirm(ConfirmRequestDto {
            plan_id: plan.plan_id,
            plan_hash: plan.plan_hash,
        })
        .await
        .unwrap();
    assert_eq!(confirmed.execution.status, "succeeded");
    assert_eq!(
        confirmed.execution.verification.as_ref().unwrap().status,
        "verified"
    );
    assert_eq!(
        confirmed.snapshot.tools[0].components[0]
            .installed_version
            .as_deref(),
        Some("2.0.0")
    );

    let history = fixture.api.run_history().unwrap();
    assert_eq!(history.runs.len(), 1);
    assert_eq!(history.runs[0].status, "succeeded");
    let log = fixture
        .api
        .run_log(RunLogRequestDto {
            run_id: history.runs[0].id.clone(),
        })
        .await
        .unwrap();
    assert!(log
        .entries
        .iter()
        .any(|entry| entry.message.contains("stdout-one")));
    assert!(log
        .entries
        .iter()
        .any(|entry| entry.message.contains("stderr-one")));

    let events = fixture.emitter.events();
    assert!(events
        .iter()
        .any(|event| matches!(event, ApiEvent::RefreshProgress(_))));
    assert!(events.iter().any(|event| matches!(
        event,
        ApiEvent::RefreshProgress(progress)
            if progress.phase == RefreshPhaseDto::Progress
                && progress.provider_id.as_deref() == Some("acceptance")
                && progress.completed == Some(0)
                && progress.total == Some(1)
    )));
    let refresh_progress: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            ApiEvent::RefreshProgress(progress) => Some(progress),
            _ => None,
        })
        .collect();
    assert!(refresh_progress.iter().any(|progress| {
        progress.stage == Some(RefreshStageDto::Probe)
            && progress.item_completed.is_none()
            && progress.item_total.is_none()
            && progress.package_name.is_none()
    }));
    assert!(refresh_progress.iter().any(|progress| {
        progress.stage == Some(RefreshStageDto::Scan)
            && progress.item_completed == Some(1)
            && progress.item_total == Some(1)
    }));
    assert!(refresh_progress.iter().any(|progress| {
        progress.stage == Some(RefreshStageDto::CheckUpdates)
            && progress.item_completed == Some(1)
            && progress.item_total == Some(1)
    }));
    assert!(refresh_progress
        .iter()
        .filter(|progress| progress.phase == RefreshPhaseDto::Completed)
        .all(|progress| {
            progress.stage.is_none()
                && progress.package_name.is_none()
                && progress.item_completed.is_none()
                && progress.item_total.is_none()
        }));
    assert!(events
        .iter()
        .any(|event| matches!(event, ApiEvent::RunOutput(_))));
    assert!(events
        .iter()
        .any(|event| matches!(event, ApiEvent::ToolState(_))));
    let run_sequences: Vec<u64> = events
        .iter()
        .filter_map(|event| match event {
            ApiEvent::RunState(event) if event.run_id == confirmed.execution.run_id => {
                event.sequence.parse().ok()
            }
            ApiEvent::RunOutput(event) if event.run_id == confirmed.execution.run_id => {
                event.last_sequence.parse().ok()
            }
            _ => None,
        })
        .collect();
    assert!(run_sequences.windows(2).all(|pair| pair[0] < pair[1]));
}

#[tokio::test]
async fn cancel_command_stops_an_active_confirmed_run() {
    let temporary = TempDir::new().unwrap();
    let process_prefix = temporary.path().join("cancelled-process");
    let fixture = Fixture::new(vec![
        "sleep-tree".into(),
        process_prefix.to_string_lossy().into_owned(),
    ]);
    let plan = fixture.preview().await;
    let api = Arc::clone(&fixture.api);
    let confirm = tokio::spawn(async move {
        api.confirm(ConfirmRequestDto {
            plan_id: plan.plan_id,
            plan_hash: plan.plan_hash,
        })
        .await
    });

    let run_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(run_id) = fixture
                .emitter
                .events()
                .iter()
                .find_map(|event| match event {
                    ApiEvent::RunState(event)
                        if matches!(
                            &event.state,
                            RunStateKindDto::StateChanged { status } if status == "running"
                        ) =>
                    {
                        Some(event.run_id.clone())
                    }
                    _ => None,
                })
            {
                break run_id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let cancelled = fixture
        .api
        .cancel(CancelRequestDto {
            run_id: run_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        cancelled.disposition,
        CancelDispositionDto::CancellationRequested
    );
    let result = confirm.await.unwrap().unwrap();
    assert_eq!(result.execution.run_id, run_id);
    assert_eq!(result.execution.status, "cancelled");
    assert_eq!(
        fixture.api.run_history().unwrap().runs[0].status,
        "cancelled"
    );
}

#[tokio::test]
async fn confirmed_command_emits_live_output_while_the_confirm_response_is_pending() {
    let gate_dir = TempDir::new().unwrap();
    let gate = gate_dir.path().join("finish");
    let fixture = Fixture::new(vec!["live-gated".into(), gate.display().to_string()]);
    let plan = fixture.preview().await;
    let api = Arc::clone(&fixture.api);
    let execution = tokio::spawn(async move {
        api.confirm(ConfirmRequestDto {
            plan_id: plan.plan_id,
            plan_hash: plan.plan_hash,
        })
        .await
    });
    let first_output = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if fixture.emitter.events().iter().any(|event| matches!(event,
                ApiEvent::RunOutput(output) if output.chunks.iter().any(|chunk| chunk.message.contains("fixture-started"))
            )) { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await;
    if first_output.is_err() {
        execution.abort();
        let _ = execution.await;
        panic!("live output was not delivered before the command finished");
    }
    assert!(
        !execution.is_finished(),
        "confirm must still be running when the first output reaches the UI"
    );
    fs::write(gate, "finish").unwrap();
    let confirmed = execution.await.unwrap().unwrap();
    assert_eq!(confirmed.execution.status, "succeeded");
    assert!(fixture.emitter.events().iter().any(|event| matches!(event,
        ApiEvent::RunOutput(output) if output.chunks.iter().any(|chunk| chunk.message.contains("fixture-finished"))
    )));
}
