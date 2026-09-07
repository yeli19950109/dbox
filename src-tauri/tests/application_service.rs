#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dbox_lib::application::{
    ApplicationEnvironment, ApplicationError, ApplicationService, ApplicationServiceOptions,
    ConfirmUpdateRequest, RefreshRequest, RefreshScope, UpdateSelection,
};
use dbox_lib::catalog::{built_in_manifests, Catalog};
use dbox_lib::domain::{
    ComponentId, Installation, InstallationScope, PackageCoordinate, PackageKind, ProviderId,
    RunStatus, StrategyId,
};
use dbox_lib::executor::{CommandExecutor, NoopEventSink, PlanError};
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use dbox_lib::providers::{
    FakeProvider, ProviderCapabilities, ProviderError, ProviderOperation, ProviderRegistry,
    ProviderStatus, ProviderUpdatePlan, ProviderUpdateRequest, Recoverability, ToolProvider,
    UpdateCheck,
};
use dbox_lib::version::{ComponentStatus, VerificationStatus, VersionValue};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct FakeEnvironment {
    programs: BTreeMap<String, PathBuf>,
    revision: Mutex<String>,
}

impl FakeEnvironment {
    fn with_programs(programs: BTreeMap<String, PathBuf>) -> Self {
        Self {
            programs,
            revision: Mutex::new("environment-1".into()),
        }
    }

    fn set_revision(&self, revision: &str) {
        *self.revision.lock().unwrap() = revision.into();
    }
}

impl ApplicationEnvironment for FakeEnvironment {
    fn resolve_program(
        &self,
        name: &str,
        user_override: Option<&Path>,
        _now: chrono::DateTime<Utc>,
    ) -> Result<PathBuf, String> {
        user_override
            .map(Path::to_path_buf)
            .or_else(|| self.programs.get(name).cloned())
            .ok_or_else(|| format!("{name} is unavailable"))
    }

    fn revision(&self) -> String {
        self.revision.lock().unwrap().clone()
    }
}

struct Fixture {
    temporary: TempDir,
    store: Arc<PersistenceStore>,
    program: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let program = temporary.path().join("fake-command");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-command.sh"),
            &program,
        )
        .unwrap();
        let mut permissions = fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&program, permissions).unwrap();
        let store = Arc::new(PersistenceStore::new(AppPaths::new(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("logs"),
        )));
        Self {
            temporary,
            store,
            program,
        }
    }

    fn executor(&self) -> CommandExecutor {
        CommandExecutor::new(Some(Arc::clone(&self.store)), Arc::new(NoopEventSink), 4096)
    }

    fn service(
        &self,
        registry: ProviderRegistry,
        catalog: Catalog,
        environment: Arc<dyn ApplicationEnvironment>,
    ) -> ApplicationService {
        ApplicationService::new(
            Arc::new(registry),
            catalog,
            Arc::clone(&self.store),
            self.executor(),
            environment,
            ApplicationServiceOptions {
                refresh_cache_ttl: Duration::from_secs(60),
                ..ApplicationServiceOptions::default()
            },
        )
        .unwrap()
    }
}

fn installation(provider: &str, kind: PackageKind, package: &str, version: &str) -> Installation {
    let mut installation = Installation::new(
        ProviderId::new(provider).unwrap(),
        PackageCoordinate::new(kind, package).unwrap(),
        InstallationScope::Global,
    )
    .unwrap();
    installation.installed_version = Some(VersionValue::new(version));
    installation
}

fn checked(installation: &Installation, latest: &str) -> UpdateCheck {
    let latest = VersionValue::new(latest);
    UpdateCheck {
        installation_id: installation.id.clone(),
        component_id: ComponentId::new("core").unwrap(),
        installed_version: installation.installed_version.clone(),
        latest_version: Some(latest.clone()),
        status: ComponentStatus::from_versions(
            installation.installed_version.as_ref(),
            Some(&latest),
        ),
    }
}

#[tokio::test]
async fn refresh_is_deduplicated_and_one_provider_failure_does_not_block_others() {
    let fixture = Fixture::new();
    let healthy_installation = installation(
        "healthy",
        PackageKind::Other("fixture".into()),
        "unknown-tool",
        "1.0.0",
    );
    let healthy = FakeProvider::new(ProviderId::new("healthy").unwrap())
        .with_delay(Duration::from_millis(30))
        .with_scan_result(Ok(vec![healthy_installation.clone()]))
        .with_check_result(Ok(vec![checked(&healthy_installation, "1.1.0")]));
    let healthy_observer = healthy.clone();
    let broken_id = ProviderId::new("broken").unwrap();
    let broken = FakeProvider::new(broken_id.clone()).with_scan_result(Err(ProviderError::new(
        broken_id.clone(),
        ProviderOperation::Scan,
        Recoverability::Retryable,
        "fixture scan failed",
    )));
    let broken_observer = broken.clone();
    let empty =
        FakeProvider::new(ProviderId::new("empty").unwrap()).with_scan_result(Ok(Vec::new()));
    let empty_observer = empty.clone();
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(healthy)).unwrap();
    registry.register(Arc::new(broken)).unwrap();
    registry.register(Arc::new(empty)).unwrap();
    let environment = Arc::new(FakeEnvironment::default());
    let service = fixture.service(registry, Catalog::default(), environment);

    let (first, duplicate) = tokio::join!(
        service.refresh_tools(RefreshRequest::all()),
        service.refresh_tools(RefreshRequest::all())
    );
    let first = first.unwrap();
    assert_eq!(duplicate.unwrap(), first);
    assert_eq!(first.tools.len(), 1);
    assert!(matches!(
        first.tools[0].components[0].status,
        ComponentStatus::UpdateAvailable
    ));
    assert_eq!(first.providers[&broken_id].errors.len(), 1);
    assert_eq!(healthy_observer.calls().scan, 1);
    assert_eq!(broken_observer.calls().scan, 1);
    assert_eq!(empty_observer.calls().scan, 1);

    service
        .refresh_tools(RefreshRequest::force_all())
        .await
        .unwrap();
    assert_eq!(healthy_observer.calls().scan, 2);
    assert_eq!(broken_observer.calls().scan, 2);
    assert_eq!(empty_observer.calls().scan, 2);

    service
        .refresh_tools(RefreshRequest {
            scope: RefreshScope::Provider {
                provider_id: ProviderId::new("healthy").unwrap(),
            },
            force: true,
        })
        .await
        .unwrap();
    assert_eq!(healthy_observer.calls().scan, 3);
    assert_eq!(broken_observer.calls().scan, 2);
    assert_eq!(fixture.store.load_state().unwrap().value.tools.len(), 1);
}

#[tokio::test]
async fn pi_preferences_and_component_plans_are_deterministic_and_never_use_all() {
    let fixture = Fixture::new();
    let pi = installation(
        "npm-global",
        PackageKind::NpmGlobal,
        "@earendil-works/pi-coding-agent",
        "1.0.0",
    );
    let random = installation(
        "npm-global",
        PackageKind::NpmGlobal,
        "random-uncatalogued",
        "1.0.0",
    );
    let provider = FakeProvider::new(ProviderId::new("npm-global").unwrap())
        .with_scan_result(Ok(vec![pi.clone(), random.clone()]))
        .with_check_result(Ok(vec![checked(&pi, "1.1.0"), checked(&random, "1.0.0")]));
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(provider)).unwrap();
    let environment = Arc::new(FakeEnvironment::with_programs(BTreeMap::from([
        ("pi".into(), fixture.program.clone()),
        ("npm".into(), fixture.program.clone()),
    ])));
    let service = fixture.service(
        registry,
        Catalog::build(built_in_manifests()).unwrap(),
        environment,
    );
    let snapshot = service.refresh_tools(RefreshRequest::all()).await.unwrap();
    assert_eq!(snapshot.tools.len(), 2);
    let pi_tool = snapshot
        .tools
        .iter()
        .find(|tool| tool.display_name == "Pi")
        .unwrap();
    let pi_tool_id = pi_tool.id.clone();

    service
        .select_strategy(
            &pi_tool_id,
            &ComponentId::new("core").unwrap(),
            &StrategyId::new("npm-global").unwrap(),
        )
        .await
        .unwrap();
    let settings = fixture.store.load_settings().unwrap();
    assert_eq!(
        settings.value.component_strategies[&pi_tool_id][&ComponentId::new("core").unwrap()]
            .as_str(),
        "npm-global"
    );

    let default_core = service
        .preview_updates(&[UpdateSelection {
            tool_id: pi_tool_id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: None,
            target_version: None,
        }])
        .await
        .unwrap();
    assert_eq!(
        default_core[0].command.args,
        [
            "install",
            "--global",
            "--",
            "@earendil-works/pi-coding-agent@latest"
        ]
    );

    let batch = service
        .preview_updates(&[
            UpdateSelection {
                tool_id: pi_tool_id.clone(),
                component_id: ComponentId::new("core").unwrap(),
                strategy_id: Some(StrategyId::new("self").unwrap()),
                target_version: None,
            },
            UpdateSelection {
                tool_id: pi_tool_id,
                component_id: ComponentId::new("extensions").unwrap(),
                strategy_id: None,
                target_version: None,
            },
        ])
        .await
        .unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].command.args, ["update", "--self"]);
    assert_eq!(batch[1].command.args, ["update", "--extensions"]);
    assert!(batch
        .iter()
        .all(|plan| !plan.command.args.iter().any(|argument| argument == "--all")));
}

#[tokio::test]
async fn config_and_environment_changes_invalidate_previewed_plans() {
    let fixture = Fixture::new();
    let item = installation(
        "provider",
        PackageKind::Other("fixture".into()),
        "tool",
        "1.0.0",
    );
    let provider = FakeProvider::new(ProviderId::new("provider").unwrap())
        .with_scan_result(Ok(vec![item.clone()]))
        .with_check_result(Ok(vec![checked(&item, "2.0.0")]))
        .with_plan_result(Ok(ProviderUpdatePlan {
            provider_id: ProviderId::new("provider").unwrap(),
            installation_id: item.id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: StrategyId::new("provider-default").unwrap(),
            program: fixture.program.display().to_string(),
            args: vec!["streams".into()],
        }));
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(provider)).unwrap();
    let environment = Arc::new(FakeEnvironment::default());
    let service = fixture.service(registry, Catalog::default(), environment.clone());
    let snapshot = service.refresh_tools(RefreshRequest::all()).await.unwrap();
    let selection = UpdateSelection {
        tool_id: snapshot.tools[0].id.clone(),
        component_id: ComponentId::new("core").unwrap(),
        strategy_id: None,
        target_version: None,
    };

    let plan = service
        .preview_updates(std::slice::from_ref(&selection))
        .await
        .unwrap()
        .remove(0);
    let loaded = fixture.store.load_settings().unwrap();
    let mut changed = loaded.value;
    changed.default_timeout_seconds += 1;
    fixture
        .store
        .save_settings(&loaded.revision, &changed)
        .unwrap();
    let error = service
        .confirm_update(
            ConfirmUpdateRequest {
                plan_id: plan.plan_id,
                plan_hash: plan.plan_hash,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ApplicationError::Plan(PlanError::ConfigChanged)
    ));

    let refreshed = service
        .refresh_tools(RefreshRequest::force_all())
        .await
        .unwrap();
    let selection = UpdateSelection {
        tool_id: refreshed.tools[0].id.clone(),
        ..selection
    };
    let plan = service
        .preview_updates(&[selection])
        .await
        .unwrap()
        .remove(0);
    environment.set_revision("environment-2");
    let error = service
        .confirm_update(
            ConfirmUpdateRequest {
                plan_id: plan.plan_id,
                plan_hash: plan.plan_hash,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ApplicationError::Plan(PlanError::EnvironmentChanged)
    ));
}

struct PostCheckFailureProvider {
    id: ProviderId,
    installation: Installation,
    program: PathBuf,
    scans: AtomicUsize,
}

#[async_trait]
impl ToolProvider for PostCheckFailureProvider {
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
        if self.scans.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(vec![self.installation.clone()])
        } else {
            Err(ProviderError::new(
                self.id.clone(),
                ProviderOperation::Scan,
                Recoverability::Retryable,
                "post-check scan failed",
            ))
        }
    }

    async fn check_updates(
        &self,
        _installations: &[Installation],
    ) -> Result<Vec<UpdateCheck>, ProviderError> {
        Ok(vec![checked(&self.installation, "2.0.0")])
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
            program: self.program.display().to_string(),
            args: vec!["streams".into()],
        })
    }
}

#[tokio::test]
async fn successful_command_with_failed_post_check_is_partial_and_not_verified() {
    let fixture = Fixture::new();
    let item = installation(
        "post-check",
        PackageKind::Other("fixture".into()),
        "tool",
        "1.0.0",
    );
    let provider = PostCheckFailureProvider {
        id: ProviderId::new("post-check").unwrap(),
        installation: item,
        program: fixture.program.clone(),
        scans: AtomicUsize::new(0),
    };
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(provider)).unwrap();
    let service = fixture.service(
        registry,
        Catalog::default(),
        Arc::new(FakeEnvironment::default()),
    );
    let snapshot = service.refresh_tools(RefreshRequest::all()).await.unwrap();
    let plan = service
        .preview_updates(&[UpdateSelection {
            tool_id: snapshot.tools[0].id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: None,
            target_version: None,
        }])
        .await
        .unwrap()
        .remove(0);
    let confirmed = service
        .confirm_update(
            ConfirmUpdateRequest {
                plan_id: plan.plan_id,
                plan_hash: plan.plan_hash,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(confirmed.execution.status, RunStatus::Partial);
    assert!(matches!(
        confirmed.execution.verification,
        Some(VerificationStatus::VerificationFailed { .. })
    ));
    assert!(matches!(
        confirmed.snapshot.tools[0].components[0].status,
        ComponentStatus::UpdateSucceeded {
            verification: VerificationStatus::VerificationFailed { .. }
        }
    ));
    let cached = fixture.store.load_state().unwrap();
    assert_eq!(cached.value.runs.last().unwrap().status, RunStatus::Partial);
    assert!(fixture.temporary.path().join("logs/runs").exists());
}

#[tokio::test]
async fn restart_retains_installation_sources_and_lists_unscanned_providers() {
    let fixture = Fixture::new();
    let installed = installation("npm-global", PackageKind::NpmGlobal, "@scope/cli", "1.0.0");
    let npm = FakeProvider::new(ProviderId::new("npm-global").unwrap())
        .with_scan_result(Ok(vec![installed.clone()]));
    let mise = FakeProvider::new(ProviderId::new("mise").unwrap());
    let registry = || {
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(npm.clone())).unwrap();
        registry.register(Arc::new(mise.clone())).unwrap();
        registry
    };
    let environment = Arc::new(FakeEnvironment::default());
    let service = fixture.service(registry(), Catalog::default(), environment.clone());
    let initial = service.snapshot().await;
    assert_eq!(initial.providers.len(), 2);
    assert!(initial
        .providers
        .values()
        .all(|report| report.status.is_none()));
    assert_eq!(npm.calls().scan, 0);
    assert_eq!(mise.calls().probe, 0);
    service
        .refresh_tools(RefreshRequest::force_all())
        .await
        .unwrap();
    drop(service);

    let restarted = fixture.service(registry(), Catalog::default(), environment);
    assert_eq!(restarted.installations().await, vec![installed.clone()]);
    assert_eq!(
        restarted.snapshot().await.tools[0].installation_ids,
        vec![installed.id.clone()]
    );
    assert_eq!(
        npm.calls().scan,
        1,
        "reading the cache does not run another scan"
    );
    restarted
        .refresh_tools(RefreshRequest {
            scope: RefreshScope::Provider {
                provider_id: ProviderId::new("mise").unwrap(),
            },
            force: true,
        })
        .await
        .unwrap();
    assert_eq!(restarted.installations().await, vec![installed]);
    assert_eq!(
        restarted.snapshot().await.tools.len(),
        1,
        "refreshing an empty provider preserves cached tools from other sources"
    );
}
