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
    ApiEvent, ApiService, BufferedExecutionEventSink, CancelDispositionDto, CancelRequestDto,
    ConfirmRequestDto, MemoryApiEventEmitter, PreviewRequestDto, RefreshRequestDto,
    RefreshScopeDto, RunLogRequestDto, RunStateKindDto, UpdateSelectionDto,
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
        let provider = AdvancingProvider {
            id: ProviderId::new("acceptance").unwrap(),
            program,
            args,
            scans: AtomicUsize::new(0),
        };
        let mut registry = dbox_lib::providers::ProviderRegistry::new();
        registry.register(Arc::new(provider)).unwrap();
        let application = Arc::new(
            ApplicationService::new(
                Arc::new(registry),
                Catalog::default(),
                Arc::clone(&store),
                CommandExecutor::new(Some(Arc::clone(&store)), sink, 4096),
                Arc::new(FakeEnvironment),
                ApplicationServiceOptions::default(),
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
        let snapshot = self
            .api
            .refresh(RefreshRequestDto {
                scope: RefreshScopeDto::All,
                force: true,
            })
            .await
            .unwrap();
        assert_eq!(snapshot.tools.len(), 1);
        self.api
            .preview(PreviewRequestDto {
                selections: vec![UpdateSelectionDto {
                    tool_id: snapshot.tools[0].id.clone(),
                    component_id: "core".into(),
                    strategy_id: Some(StrategyId::new("provider-default").unwrap().to_string()),
                    target_version: None,
                }],
            })
            .await
            .unwrap()
            .remove(0)
    }
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
