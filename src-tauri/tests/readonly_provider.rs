use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dbox_lib::application::{
    ApplicationEnvironment, ApplicationError, ApplicationService, ApplicationServiceOptions,
    RefreshRequest, UpdateSelection,
};
use dbox_lib::catalog::{Catalog, CatalogLayer, LoadedManifest};
use dbox_lib::domain::{
    ComponentId, Installation, InstallationScope, PackageCoordinate, PackageKind, ProviderId,
    StrategyId, StrategyKind, Tool,
};
use dbox_lib::executor::{CommandExecutor, NoopEventSink};
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use dbox_lib::providers::{FakeProvider, ProviderCapabilities, ProviderRegistry, UpdateCheck};
use dbox_lib::version::{ComponentStatus, VersionValue};
use tempfile::TempDir;

struct FixtureEnvironment;

impl ApplicationEnvironment for FixtureEnvironment {
    fn resolve_program(
        &self,
        name: &str,
        _user_override: Option<&Path>,
        _now: DateTime<Utc>,
    ) -> Result<PathBuf, String> {
        Err(format!("unexpected program resolution: {name}"))
    }

    fn revision(&self) -> String {
        "readonly-provider-fixture".into()
    }
}

struct Fixture {
    _temporary: TempDir,
    store: Arc<PersistenceStore>,
    installation: Installation,
    provider: FakeProvider,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let store = Arc::new(PersistenceStore::new(AppPaths::new(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("logs"),
        )));
        let mut installation = Installation::new(
            ProviderId::new("readonly-releases").unwrap(),
            PackageCoordinate::new(PackageKind::Direct, "DashBeam").unwrap(),
            InstallationScope::User,
        )
        .unwrap();
        installation.installed_version = Some(VersionValue::new("0.7.1"));
        let provider = FakeProvider::new(installation.provider_id.clone())
            .with_capabilities(ProviderCapabilities {
                full_scan: true,
                check_updates: true,
                update: false,
                ..ProviderCapabilities::default()
            })
            .with_scan_result(Ok(vec![installation.clone()]))
            .with_check_result(Ok(vec![UpdateCheck {
                installation_id: installation.id.clone(),
                component_id: ComponentId::new("core").unwrap(),
                installed_version: installation.installed_version.clone(),
                latest_version: Some(VersionValue::new("0.8.0")),
                status: ComponentStatus::UpdateAvailable,
            }]));
        Self {
            _temporary: temporary,
            store,
            installation,
            provider,
        }
    }

    fn service(&self, catalog: Catalog) -> ApplicationService {
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(self.provider.clone())).unwrap();
        ApplicationService::new(
            Arc::new(registry),
            catalog,
            Arc::clone(&self.store),
            CommandExecutor::new(Some(Arc::clone(&self.store)), Arc::new(NoopEventSink), 4096),
            Arc::new(FixtureEnvironment),
            ApplicationServiceOptions::default(),
        )
        .unwrap()
    }

    fn seed_legacy_update_strategy(&self) -> Tool {
        // Older state files unconditionally included provider-default, even for
        // providers that only offered discovery and update checks.
        let mut tool = Tool::from_installation(&self.installation).unwrap();
        tool.components[0].latest_version = Some(VersionValue::new("0.8.0"));
        tool.components[0].status = ComponentStatus::UpdateAvailable;
        let loaded = self.store.load_state().unwrap();
        let mut state = loaded.value;
        state.installations = vec![self.installation.clone()];
        state.tools = vec![tool.clone()];
        state.last_refresh_at = Some(Utc::now());
        self.store.save_state(&loaded.revision, &state).unwrap();

        let loaded = self.store.load_settings().unwrap();
        let mut settings = loaded.value;
        settings
            .component_strategies
            .entry(tool.id.clone())
            .or_default()
            .insert(
                ComponentId::new("core").unwrap(),
                StrategyId::new("provider-default").unwrap(),
            );
        self.store
            .save_settings(&loaded.revision, &settings)
            .unwrap();
        tool
    }
}

fn assert_check_only(tool: &Tool) {
    assert_eq!(tool.components.len(), 1);
    let component = &tool.components[0];
    assert_eq!(component.status, ComponentStatus::UpdateAvailable);
    assert_eq!(
        component.installed_version,
        Some(VersionValue::new("0.7.1"))
    );
    assert_eq!(component.latest_version, Some(VersionValue::new("0.8.0")));
    assert!(!component
        .strategies
        .iter()
        .any(|strategy| { strategy.enabled && strategy.kind == StrategyKind::ProviderDefault }));
    assert_eq!(component.default_strategy, None);
}

#[tokio::test]
async fn refresh_reports_available_versions_without_offering_provider_update() {
    let fixture = Fixture::new();
    let service = fixture.service(Catalog::default());
    let snapshot = service.refresh_tools(RefreshRequest::all()).await.unwrap();

    assert_eq!(snapshot.tools.len(), 1);
    assert_check_only(&snapshot.tools[0]);
    assert_eq!(fixture.provider.calls().check_updates, 1);
    assert_eq!(fixture.provider.calls().plan_update, 0);
    let persisted = fixture.store.load_state().unwrap();
    assert_check_only(&persisted.value.tools[0]);
}

#[tokio::test]
async fn startup_strips_stale_cached_update_strategy_without_requiring_refresh() {
    let fixture = Fixture::new();
    fixture.seed_legacy_update_strategy();
    let service = fixture.service(Catalog::default());

    let snapshot = service.snapshot().await;
    assert_eq!(snapshot.tools.len(), 1);
    assert_check_only(&snapshot.tools[0]);
    assert_eq!(fixture.provider.calls().scan, 0);
    assert_eq!(fixture.provider.calls().check_updates, 0);
}

#[tokio::test]
async fn catalog_reload_cannot_restore_provider_update_or_stale_preference() {
    let fixture = Fixture::new();
    fixture.seed_legacy_update_strategy();
    let service = fixture.service(Catalog::default());
    let snapshot = service
        .reload_catalog(Catalog::default(), Vec::new())
        .await
        .unwrap();

    assert_eq!(snapshot.tools.len(), 1);
    assert_check_only(&snapshot.tools[0]);
    assert_check_only(&fixture.store.load_state().unwrap().value.tools[0]);
}

#[tokio::test]
async fn explicit_provider_update_is_rejected_before_provider_plan_is_called() {
    let fixture = Fixture::new();
    let stale_tool = fixture.seed_legacy_update_strategy();
    let service = fixture.service(Catalog::default());

    for strategy_id in [Some(StrategyId::new("provider-default").unwrap()), None] {
        let explicit_strategy = strategy_id.is_some();
        let result = service
            .preview_updates(&[UpdateSelection {
                tool_id: stale_tool.id.clone(),
                component_id: ComponentId::new("core").unwrap(),
                strategy_id,
                target_version: None,
            }])
            .await;
        match result.unwrap_err() {
            ApplicationError::UnknownStrategy { .. } if explicit_strategy => {}
            ApplicationError::NoDefaultStrategy { .. } if !explicit_strategy => {}
            error => panic!("unexpected read-only preview rejection: {error}"),
        }
    }
    assert_eq!(fixture.provider.calls().scan, 0);
    assert_eq!(fixture.provider.calls().plan_update, 0);
    assert!(fixture.store.load_state().unwrap().value.runs.is_empty());
}

#[tokio::test]
async fn explicit_user_command_remains_available_for_a_readonly_provider() {
    let fixture = Fixture::new();
    let catalog = Catalog::build(vec![LoadedManifest::parse(
        "readonly-user-command.toml",
        CatalogLayer::User,
        r#"
schema_version = 1
id = "dashbeam"

[[installations]]
id = "application"
provider_id = "readonly-releases"
kind = "direct"
package = "DashBeam"

[[components]]
id = "core"
default_strategy = "user-command"

[[components.update_strategies]]
id = "user-command"
kind = "command"
program = "user-chosen-updater"
args = ["DashBeam"]
"#,
    )
    .unwrap()])
    .unwrap();
    let service = fixture.service(catalog.clone());

    service.refresh_tools(RefreshRequest::all()).await.unwrap();
    let snapshot = service.reload_catalog(catalog, Vec::new()).await.unwrap();
    assert_eq!(snapshot.tools.len(), 1);
    let component = &snapshot.tools[0].components[0];
    assert_eq!(component.status, ComponentStatus::UpdateAvailable);
    assert_eq!(
        component.default_strategy,
        Some(StrategyId::new("user-command").unwrap())
    );
    assert!(component.strategies.iter().any(|strategy| {
        strategy.id.as_str() == "user-command"
            && strategy.enabled
            && matches!(strategy.kind, StrategyKind::Command { .. })
    }));
    assert!(!component
        .strategies
        .iter()
        .any(|strategy| { strategy.enabled && strategy.kind == StrategyKind::ProviderDefault }));
    assert_eq!(fixture.provider.calls().plan_update, 0);
}
