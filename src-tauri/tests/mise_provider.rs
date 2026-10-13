#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dbox_lib::api::{
    ApiEvent, ApiService, MemoryApiEventEmitter, PreviewRequestDto, RefreshRequestDto,
    RefreshScopeDto, UpdateSelectionDto,
};
use dbox_lib::application::{
    ApplicationError, ApplicationService, ApplicationServiceOptions, RefreshRequest,
    ResolverApplicationEnvironment, UpdateSelection,
};
use dbox_lib::catalog::{built_in_manifests, Catalog};
use dbox_lib::domain::{ComponentId, Installation, InstallationScope, StrategyId, Tool};
use dbox_lib::environment::{EnvironmentResolver, WhichLocator};
use dbox_lib::executor::{CommandExecutor, CommandSpec, NoopEventSink};
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use dbox_lib::providers::{
    MiseProvider, MiseProviderOptions, ProviderRegistry, ProviderUpdateRequest, ToolProvider,
};
use dbox_lib::version::{ComponentStatus, VersionValue};
use serde_json::{json, Value};
use tempfile::TempDir;

struct FakeMise {
    temporary: TempDir,
    program: PathBuf,
    home: PathBuf,
}

impl FakeMise {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let program = temporary.path().join("mise");
        fs::write(&program, include_str!("fixtures/fake-mise.sh")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        let home = temporary.path().join("home with spaces");
        fs::create_dir_all(&home).unwrap();
        let fake = Self {
            temporary,
            program,
            home,
        };
        fake.write_json("installed", json!({}));
        fake.write_json("global", json!({}));
        fake.write_json("current", json!({}));
        fake
    }

    fn options(&self) -> MiseProviderOptions {
        MiseProviderOptions {
            home_dir: self.home.clone(),
            ..MiseProviderOptions::default()
        }
    }

    fn provider(&self) -> MiseProvider {
        MiseProvider::with_options(&self.program, self.options())
    }

    fn write(&self, file: &str, value: &str) {
        fs::write(self.temporary.path().join(file), value).unwrap();
    }

    fn write_json(&self, name: &str, value: Value) {
        self.write(
            &format!("{name}.json"),
            &serde_json::to_string(&value).unwrap(),
        );
    }

    fn record(&self, tool: &str, version: &str, request: Option<&str>) -> Value {
        let mut record = json!({
            "version": version,
            "install_path": self.temporary.path().join("data/installs").join(tool).join(version),
            "installed": true,
            "active": request.is_some(),
        });
        if let Some(request) = request {
            record["requested_version"] = json!(request);
            record["source"] =
                json!({"type": "mise.toml", "path": self.home.join(".config/mise/config.toml")});
        }
        record
    }

    fn active_node(&self) {
        let versions = json!({"node": [self.record("node", "22.0.0", Some("22"))]});
        self.write_json("installed", versions.clone());
        self.write_json("global", versions.clone());
        self.write_json("current", versions);
        self.write_json("node", json!({"backend": "core:node"}));
        self.write("core_node@22.latest", "22.1.0\n");
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.temporary.path().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn store(&self) -> Arc<PersistenceStore> {
        Arc::new(PersistenceStore::new(AppPaths::new(
            self.temporary.path().join("config"),
            self.temporary.path().join("data"),
            self.temporary.path().join("logs"),
        )))
    }

    fn service(&self) -> ApplicationService {
        let store = self.store();
        let mut registry = ProviderRegistry::new();
        // A new provider instance is essential: a restarted process has no in-memory
        // scan state even though its persisted tools still show available updates.
        registry.register(Arc::new(self.provider())).unwrap();
        ApplicationService::new(
            Arc::new(registry),
            Catalog::default(),
            Arc::clone(&store),
            CommandExecutor::new(Some(store), Arc::new(NoopEventSink), 4096),
            Arc::new(ResolverApplicationEnvironment::new(
                EnvironmentResolver::new(
                    Arc::new(WhichLocator),
                    self.temporary.path().as_os_str().to_owned(),
                    self.home.clone(),
                ),
            )),
            ApplicationServiceOptions::default(),
        )
        .unwrap()
    }

    async fn persist_available_update(&self) -> UpdateSelection {
        let snapshot = self
            .service()
            .refresh_tools(RefreshRequest::all())
            .await
            .unwrap();
        assert_eq!(
            snapshot.tools[0].components[0].status,
            ComponentStatus::UpdateAvailable
        );
        UpdateSelection {
            tool_id: snapshot.tools[0].id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: None,
            target_version: None,
        }
    }
}

fn request(installation: &Installation) -> ProviderUpdateRequest {
    ProviderUpdateRequest {
        installation_id: installation.id.clone(),
        component_id: ComponentId::new("core").unwrap(),
        strategy_id: StrategyId::new("provider-default").unwrap(),
        target_version: None,
    }
}

#[tokio::test]
async fn global_node_lifecycle_snapshots_before_upgrade_and_restores_afterwards() {
    use dbox_lib::application::ConfirmUpdateRequest;
    use dbox_lib::domain::RunStatus;
    use tokio_util::sync::CancellationToken;
    for (failure, expected, phases) in [
        ("", RunStatus::Succeeded, "snapshot\nupgrade\nrestore\n"),
        ("snapshot", RunStatus::Failed, "snapshot\n"),
        ("upgrade", RunStatus::Failed, "snapshot\nupgrade\n"),
        (
            "restore",
            RunStatus::Partial,
            "snapshot\nupgrade\nrestore\n",
        ),
    ] {
        let fake = FakeMise::new();
        fake.active_node();
        let after = json!({"node": [fake.record("node", "22.1.0", Some("22"))]});
        fake.write_json("after-installed", after.clone());
        fake.write_json("after-global", after);
        if !failure.is_empty() {
            fake.write(&format!("{failure}.fail"), "yes");
        }
        let service = fake.service();
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
        assert_eq!(plan.command.before.len(), 1);
        assert_eq!(plan.command.after.len(), 1);
        assert!(plan.command.before[0]
            .command
            .args
            .contains(&"core:node@22.0.0".into()));
        assert!(plan.command.after[0]
            .command
            .args
            .contains(&"core:node@22".into()));
        assert!(plan.command.after[0].label.contains("最新版"));
        let result = service
            .confirm_update(
                ConfirmUpdateRequest {
                    plan_id: plan.plan_id,
                    plan_hash: plan.plan_hash,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.execution.status, expected, "{failure}");
        assert_eq!(
            fs::read_to_string(fake.temporary.path().join("lifecycle.log")).unwrap(),
            phases
        );
        if failure == "restore" {
            assert_eq!(
                result.snapshot.tools[0].components[0]
                    .installed_version
                    .as_ref()
                    .unwrap()
                    .raw(),
                "22.1.0"
            );
        }
    }
}

#[tokio::test]
async fn restart_preview_rehydrates_mise_before_planning_and_keeps_warm_previews_fast() {
    let fake = FakeMise::new();
    fake.active_node();
    let selection = fake.persist_available_update().await;
    let restarted = Arc::new(fake.service());
    assert_eq!(
        restarted.snapshot().await.tools[0].components[0].status,
        ComponentStatus::UpdateAvailable,
        "the update shown immediately after restart comes from persistent state"
    );
    let cached_installation = restarted.installations().await.remove(0);
    assert!(fake
        .provider()
        .plan_update(request(&cached_installation))
        .await
        .unwrap_err()
        .summary
        .contains("refresh mise"));

    let emitter = Arc::new(MemoryApiEventEmitter::default());
    let api = ApiService::new(restarted.clone(), fake.store(), emitter.clone());
    let request = PreviewRequestDto {
        selections: vec![UpdateSelectionDto {
            tool_id: selection.tool_id.to_string(),
            component_id: selection.component_id.to_string(),
            strategy_id: None,
            target_version: None,
        }],
    };
    let previous_revision = restarted.snapshot().await.state_revision;
    let plans = api.preview(request.clone()).await.unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].command.program, fake.program.display().to_string());
    assert_eq!(
        plans[0].command.args,
        [
            "--cd",
            fake.home.to_str().unwrap(),
            "upgrade",
            "--no-prune",
            "--yes",
            "--",
            "node@22"
        ]
    );
    let before = fake.calls();
    assert_eq!(
        before
            .iter()
            .filter(|call| call.starts_with("latest "))
            .count(),
        2
    );
    let snapshot = api.snapshot().await.unwrap();
    assert_ne!(snapshot.state_revision, previous_revision);
    let events = emitter.events();
    assert_eq!(events.len(), 1);
    let ApiEvent::ToolState(event) = &events[0] else {
        panic!("preview bootstrap must publish the refreshed inventory");
    };
    assert_eq!(event.state_revision, snapshot.state_revision);
    assert_eq!(event.tool_ids, vec![selection.tool_id.to_string()]);
    api.preview(request).await.unwrap();
    assert_eq!(
        emitter.events().len(),
        1,
        "warm previews do not emit spurious refreshes"
    );
    let after = fake.calls();
    for command in ["version", "ls --installed", "latest "] {
        assert_eq!(
            before
                .iter()
                .filter(|call| call.starts_with(command))
                .count(),
            after
                .iter()
                .filter(|call| call.starts_with(command))
                .count(),
            "a warm preview does not repeat {command}"
        );
    }
    assert!(!after
        .iter()
        .any(|call| call.starts_with("upgrade --no-prune")));
}

#[tokio::test]
async fn restart_preview_refuses_failed_scan_and_retries_without_a_manual_refresh() {
    let fake = FakeMise::new();
    fake.active_node();
    let selection = fake.persist_available_update().await;
    let restarted = fake.service();
    fake.write("installed.fail", "");
    let error = restarted
        .preview_updates(std::slice::from_ref(&selection))
        .await
        .unwrap_err();
    assert!(matches!(error, ApplicationError::Provider(_)));
    assert!(!error.to_string().contains("secret-fixture"));
    fs::remove_file(fake.temporary.path().join("installed.fail")).unwrap();
    let plans = restarted.preview_updates(&[selection]).await.unwrap();
    assert_eq!(
        plans.len(),
        1,
        "a failed bootstrap must not be cached as ready"
    );
    assert!(!fake
        .calls()
        .iter()
        .any(|call| call.starts_with("upgrade --no-prune")));
}

#[tokio::test]
async fn restart_preview_refuses_failed_package_check_instead_of_using_cached_update() {
    let fake = FakeMise::new();
    fake.active_node();
    let selection = fake.persist_available_update().await;
    let restarted = fake.service();
    fake.write("core_node@22.fail", "");
    let error = restarted
        .preview_updates(std::slice::from_ref(&selection))
        .await
        .unwrap_err();
    assert!(matches!(error, ApplicationError::Provider(_)));
    assert!(!error.to_string().contains("secret-fixture"));
    assert!(matches!(
        restarted.snapshot().await.tools[0].components[0].status,
        ComponentStatus::CheckFailed { .. }
    ));
    fs::remove_file(fake.temporary.path().join("core_node@22.fail")).unwrap();
    assert_eq!(
        restarted.preview_updates(&[selection]).await.unwrap().len(),
        1
    );
    assert!(!fake
        .calls()
        .iter()
        .any(|call| call.starts_with("upgrade --no-prune")));
}

#[tokio::test]
async fn preview_refuses_a_provider_disabled_after_it_was_refreshed() {
    let fake = FakeMise::new();
    fake.active_node();
    let service = fake.service();
    let snapshot = service.refresh_tools(RefreshRequest::all()).await.unwrap();
    let selection = UpdateSelection {
        tool_id: snapshot.tools[0].id.clone(),
        component_id: ComponentId::new("core").unwrap(),
        strategy_id: None,
        target_version: None,
    };
    let store = fake.store();
    let loaded = store.load_settings().unwrap();
    let mut settings = loaded.value;
    settings
        .provider_enabled
        .insert(fake.provider().id(), false);
    store.save_settings(&loaded.revision, &settings).unwrap();
    let before = fake.calls();
    let error = service.preview_updates(&[selection]).await.unwrap_err();
    assert!(matches!(error, ApplicationError::Provider(_)));
    assert_eq!(
        fake.calls(),
        before,
        "disabled sources must not be queried or planned"
    );
}

#[tokio::test]
async fn refresh_progress_reports_scan_packages_and_concurrent_check_completion() {
    let fake = FakeMise::new();
    let inventory = json!({
        "node": [fake.record("node", "22.0.0", Some("22"))],
        "python": [fake.record("python", "3.13.0", Some("3.13"))]
    });
    fake.write_json("installed", inventory.clone());
    fake.write_json("global", inventory);
    fake.write_json("node", json!({"backend": "core:node"}));
    fake.write_json("python", json!({"backend": "core:python"}));
    fake.write("core_node@22.latest", "22.1.0\n");
    fake.write("core_node@22.delay", "");
    fake.write("core_python@3.13.fail", "");
    let provider = fake.provider();
    let mut scans = Vec::new();
    let installations = provider
        .scan_with_progress(&mut |progress| scans.push(progress))
        .await
        .unwrap();
    assert_eq!(scans[0].completed, None);
    assert!(scans.iter().any(|progress| {
        progress.package_name.as_deref() == Some("node")
            && progress.completed == Some(0)
            && progress.total == Some(2)
    }));
    assert_eq!(scans.last().unwrap().completed, Some(2));

    let mut progress = Vec::new();
    let checks = provider
        .check_updates_with_progress(&installations, &mut |event| progress.push(event))
        .await
        .unwrap();
    assert_eq!(progress.len(), 3);
    assert_eq!(progress[0].package_name, None);
    assert_eq!(progress[0].completed, Some(0));
    assert_eq!(progress[1].package_name.as_deref(), Some("core:python"));
    assert_eq!(progress[1].completed, Some(1));
    assert_eq!(progress[2].package_name.as_deref(), Some("core:node"));
    assert_eq!(progress[2].completed, Some(2));
    assert!(progress.iter().all(|event| event.total == Some(2)));
    assert!(checks
        .iter()
        .any(|check| matches!(check.status, ComponentStatus::CheckFailed { .. })));
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|call| call.starts_with("latest "))
            .count(),
        2
    );
}

#[tokio::test]
async fn probe_scan_check_plan_preserve_versions_backends_and_global_activation() {
    let fake = FakeMise::new();
    let node22 = fake.record("node", "22.0.0", Some("22"));
    let node20 = fake.record("node", "20.0.0", Some("20"));
    let node18 = fake.record("node", "18.0.0", None);
    let mut project_node = fake.record("node", "24.0.0", Some("24"));
    project_node["source"]["path"] = json!(fake.home.join("project/mise.toml"));
    let scoped = fake.record("npm-scope-cli", "1.0.0", Some("latest"));
    let mut missing = fake.record("node", "26.0.0", Some("26"));
    missing["installed"] = json!(false);
    fake.write_json("installed", json!({"node": [node18, node20.clone(), node22.clone(), project_node, missing], "npm:@scope/cli": [scoped.clone()]}));
    let globals = json!({"node": [node20, node22], "npm:@scope/cli": [scoped]});
    fake.write_json("global", globals.clone());
    fake.write_json("current", globals);
    fake.write_json("node", json!({"backend": "core:node"}));
    fake.write_json("npm_@scope_cli", json!({"backend": "npm:@scope/cli"}));
    fake.write("core_node@20.latest", "20.0.0\n");
    fake.write("core_node@22.latest", "22.1.0\n");
    fake.write("npm_@scope_cli@latest.latest", "1.2.0\n");
    let provider = fake.provider();
    let status = provider.probe().await.unwrap();
    assert!(status.available);
    assert!(status.version.unwrap().raw().starts_with("2026.9.1"));
    assert!(provider.capabilities().multiple_versions);
    assert!(!provider.capabilities().version_pinning);
    let installations = provider.scan().await.unwrap();
    assert_eq!(installations.len(), 5);
    assert_eq!(
        installations
            .iter()
            .filter(|item| item.scope == InstallationScope::Global)
            .count(),
        3
    );
    let checks = provider.check_updates(&installations).await.unwrap();
    assert_eq!(
        checks
            .iter()
            .filter(|check| check.status == ComponentStatus::UpdateAvailable)
            .count(),
        2
    );
    assert_eq!(
        checks
            .iter()
            .filter(|check| matches!(check.status, ComponentStatus::Unsupported { .. }))
            .count(),
        2
    );
    let selected = installations
        .iter()
        .find(|item| item.installed_version.as_ref().unwrap().raw() == "22.0.0")
        .unwrap();
    assert_eq!(selected.package.name, "core:node");
    let plan = provider.plan_update(request(selected)).await.unwrap();
    assert_eq!(plan.program, fake.program.display().to_string());
    assert_eq!(
        plan.args,
        [
            "--cd",
            fake.home.to_str().unwrap(),
            "upgrade",
            "--no-prune",
            "--yes",
            "--",
            "node@22"
        ]
    );
    let command = CommandSpec::new(plan.program, plan.args);
    command.validate().unwrap();
    assert!(command
        .to_std_command()
        .unwrap()
        .status()
        .unwrap()
        .success());
    assert!(!fake.calls().iter().any(|call| call.contains("use ")
        || call.contains("--bump")
        || call.contains("--inactive")));
}

#[tokio::test]
async fn active_slot_survives_upgrade_and_inactive_versions_remain_distinct() {
    let fake = FakeMise::new();
    fake.active_node();
    let provider = fake.provider();
    let before = provider.scan().await.unwrap().remove(0);
    let plan = provider.plan_update(request(&before)).await.unwrap();
    let new_record = fake.record("node", "22.1.0", Some("22"));
    fake.write_json(
        "after-installed",
        json!({"node": [fake.record("node", "22.0.0", None), new_record.clone()]}),
    );
    fake.write_json("after-global", json!({"node": [new_record]}));
    assert!(CommandSpec::new(plan.program, plan.args)
        .to_std_command()
        .unwrap()
        .status()
        .unwrap()
        .success());
    let after = provider.scan().await.unwrap();
    assert_eq!(after.len(), 2);
    let active = after.iter().find(|item| item.id == before.id).unwrap();
    assert_eq!(active.installed_version.as_ref().unwrap().raw(), "22.1.0");
    assert!(after
        .iter()
        .any(|item| item.scope == InstallationScope::User && item.id != before.id));
    let other_provider = FakeMise::new();
    other_provider.active_node();
    let other = other_provider.provider().scan().await.unwrap().remove(0);
    assert_ne!(
        before.id, other.id,
        "separate mise data/config roots have distinct identities"
    );
    let mut npm = before.clone();
    npm.provider_id = dbox_lib::domain::ProviderId::new("npm-global").unwrap();
    npm.package =
        dbox_lib::domain::PackageCoordinate::new(dbox_lib::domain::PackageKind::NpmGlobal, "node")
            .unwrap();
    npm.id =
        dbox_lib::domain::InstallationId::from_package(&npm.provider_id, &npm.package).unwrap();
    assert_ne!(
        Tool::from_installation(&before).unwrap().id,
        Tool::from_installation(&npm).unwrap().id
    );
}

#[tokio::test]
async fn offline_and_missing_plugins_do_not_hide_installations_or_report_up_to_date() {
    let fake = FakeMise::new();
    fake.active_node();
    fake.write("core_node@22.fail", "");
    let provider = fake.provider();
    let installations = provider.scan().await.unwrap();
    let checks = provider.check_updates(&installations).await.unwrap();
    let ComponentStatus::CheckFailed { reason } = &checks[0].status else {
        panic!("must report offline failure");
    };
    assert!(reason.retryable);
    assert!(!reason.summary.contains("secret-fixture"));
    fs::remove_file(fake.temporary.path().join("core_node@22.fail")).unwrap();
    assert_eq!(
        provider.check_updates(&installations).await.unwrap()[0].status,
        ComponentStatus::UpdateAvailable,
        "failed checks are not cached"
    );
    fake.write("node.fail", "");
    let installations = provider.scan().await.unwrap();
    assert_eq!(installations.len(), 1);
    let checks = provider.check_updates(&installations).await.unwrap();
    let ComponentStatus::CheckFailed { reason } = &checks[0].status else {
        panic!("must report missing plugin");
    };
    assert_eq!(reason.code, "mise_backend_unavailable");
    assert!(!reason.summary.contains("plugin-secret"));
    assert!(provider
        .plan_update(request(&installations[0]))
        .await
        .is_err());
}

#[tokio::test]
async fn cache_is_version_sensitive_and_queries_timeout_without_becoming_current() {
    let fake = FakeMise::new();
    fake.active_node();
    let provider = fake.provider();
    let installations = provider.scan().await.unwrap();
    provider.check_updates(&installations).await.unwrap();
    provider.check_updates(&installations).await.unwrap();
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|call| call.starts_with("latest "))
            .count(),
        1
    );
    provider.clear_update_cache();
    fake.write("core_node@22.latest", "not a version\n");
    assert!(matches!(
        provider.check_updates(&installations).await.unwrap()[0].status,
        ComponentStatus::CheckFailed { .. }
    ));
    let mut options = fake.options();
    options.command_timeout = Duration::from_millis(100);
    let timed = MiseProvider::with_options(&fake.program, options);
    let installations = timed.scan().await.unwrap();
    fake.write("core_node@22.delay", "");
    assert!(matches!(
        timed.check_updates(&installations).await.unwrap()[0].status,
        ComponentStatus::CheckFailed { .. }
    ));
}

#[tokio::test]
async fn pi_core_uses_mise_updates_and_respects_the_configured_version_request() {
    for (version_request, latest, status) in [
        ("1.2.0", "1.2.0", ComponentStatus::UpToDate),
        ("latest", "1.3.0", ComponentStatus::UpdateAvailable),
    ] {
        let fake = FakeMise::new();
        let versions = json!({"pi": [fake.record("pi", "1.2.0", Some(version_request))]});
        for listing in ["installed", "global", "current"] {
            fake.write_json(listing, versions.clone());
        }
        fake.write_json("pi", json!({"backend": "aqua:earendil-works/pi"}));
        fake.write(
            &format!("aqua_earendil-works_pi@{version_request}.latest"),
            latest,
        );
        let provider = fake.provider();
        let installations = provider.scan().await.unwrap();
        let checks = provider.check_updates(&installations).await.unwrap();
        assert_eq!(checks[0].status, status);
        let catalog = Catalog::build(built_in_manifests()).unwrap();
        let snapshot = catalog.enrich(&installations);
        let pi = &snapshot.tools[0];
        assert_eq!(pi.display_name, "Pi");
        let mut update_request = request(&installations[0]);
        update_request.strategy_id = pi.components[0].default_strategy.clone().unwrap();
        let plan = provider.plan_update(update_request).await.unwrap();
        assert_eq!(plan.program, fake.program.to_string_lossy());
        assert_eq!(
            plan.args,
            [
                "--cd",
                &fake.home.display().to_string(),
                "upgrade",
                "--no-prune",
                "--yes",
                "--",
                &format!("pi@{version_request}"),
            ]
        );
        assert!(fake.calls().contains(&format!(
            "latest -- aqua:earendil-works/pi@{version_request}"
        )));
    }
}

#[tokio::test]
async fn refuses_foreign_stale_project_overridden_and_unsupported_plans() {
    let fake = FakeMise::new();
    fake.active_node();
    let provider = fake.provider();
    let installation = provider.scan().await.unwrap().remove(0);
    let mut pin = request(&installation);
    pin.target_version = Some(VersionValue::new("24.0.0"));
    assert!(provider.plan_update(pin).await.is_err());
    let mut foreign = request(&installation);
    foreign.installation_id =
        dbox_lib::domain::InstallationId::new("homebrew-formula:node").unwrap();
    assert!(provider.plan_update(foreign).await.is_err());
    let mut component = request(&installation);
    component.component_id = ComponentId::new("other").unwrap();
    assert!(provider.plan_update(component).await.is_err());
    let mut project = fake.record("node", "22.0.0", Some("22"));
    project["source"]["path"] = json!(fake.home.join("mise.toml"));
    fake.write_json("current", json!({"node": [project]}));
    assert!(provider
        .plan_update(request(&installation))
        .await
        .unwrap_err()
        .summary
        .contains("overridden"));
    fake.active_node();
    fake.write_json("node", json!({"backend": "asdf:node"}));
    assert!(provider
        .plan_update(request(&installation))
        .await
        .unwrap_err()
        .summary
        .contains("backend changed"));
    fake.active_node();
    fake.write_json(
        "global",
        json!({"node": [fake.record("node", "22.1.0", Some("22"))]}),
    );
    assert!(provider.plan_update(request(&installation)).await.is_err());
    assert!(!fake
        .calls()
        .iter()
        .any(|call| call.starts_with("upgrade --no-prune")));
}

#[tokio::test]
async fn inactive_linked_special_requests_and_old_mise_are_read_only() {
    for scenario in ["inactive", "linked", "special", "old"] {
        let fake = FakeMise::new();
        fake.active_node();
        match scenario {
            "inactive" => fake.write_json("global", json!({})),
            "linked" => {
                let mut record = fake.record("node", "22.0.0", Some("22"));
                record["symlinked_to"] = json!(fake.home.join("external/node"));
                fake.write_json("installed", json!({"node": [record]}));
            }
            "special" => fake.write_json(
                "global",
                json!({"node": [fake.record("node", "22.0.0", Some("path:/external/node"))]}),
            ),
            "old" => fake.write("old-version", ""),
            _ => unreachable!(),
        }
        let provider = fake.provider();
        let installations = provider.scan().await.unwrap();
        assert_eq!(installations.len(), 1);
        assert!(
            matches!(
                provider.check_updates(&installations).await.unwrap()[0].status,
                ComponentStatus::Unsupported { .. }
            ),
            "{scenario}"
        );
        assert!(
            provider
                .plan_update(request(&installations[0]))
                .await
                .is_err(),
            "{scenario}"
        );
        assert!(!fake.calls().iter().any(|call| call.starts_with("latest ")));
    }
}

#[tokio::test]
async fn validates_json_paths_names_and_empty_installations() {
    let fake = FakeMise::new();
    let provider = fake.provider();
    assert!(provider.scan().await.unwrap().is_empty());
    for data in ["not json", "[]", "{\"node\": [{\"version\": \"22\"}]}"] {
        fake.write("installed.json", data);
        assert!(provider.scan().await.is_err());
    }
    fake.active_node();
    let mut record = fake.record("node", "22.0.0", None);
    record["install_path"] = json!("relative/path");
    fake.write_json("installed", json!({"node": [record]}));
    assert!(provider.scan().await.is_err());
    fake.write_json(
        "installed",
        json!({"--help": [fake.record("node", "22.0.0", None)]}),
    );
    assert!(provider.scan().await.is_err());
    assert!(MiseProvider::new("mise").probe().await.is_err());
}

#[tokio::test]
async fn environment_sources_without_paths_are_inventory_only() {
    let fake = FakeMise::new();
    fake.active_node();
    let mut environment = fake.record("node", "20.0.0", Some("20"));
    environment["source"] =
        json!({"type": "environment", "key": "MISE_NODE_VERSION", "value": "20"});
    fake.write_json("installed", json!({"node": [environment]}));
    let provider = fake.provider();
    let installations = provider.scan().await.unwrap();
    assert_eq!(installations.len(), 2);
    assert_eq!(
        installations
            .iter()
            .filter(|item| item.scope == InstallationScope::Global)
            .count(),
        1
    );
}

#[tokio::test]
async fn production_discovers_mise_from_the_executable_override() {
    let fake = FakeMise::new();
    fake.active_node();
    // Production chooses the user's home context. This fake still reads/writes only its
    // temporary fixtures and never invokes the real mise executable or config.
    let script = include_str!("fixtures/fake-mise.sh")
        .replace("[ \"$2\" = \"$base/home with spaces\" ]", "[ -d \"$2\" ]");
    fs::write(&fake.program, script).unwrap();
    let paths = AppPaths::new(
        fake.temporary.path().join("config"),
        fake.temporary.path().join("data"),
        fake.temporary.path().join("logs"),
    );
    let store = PersistenceStore::new(paths.clone());
    let settings = store.load_settings().unwrap();
    let mut value = settings.value;
    value
        .executable_overrides
        .insert("mise".into(), fake.program.clone());
    store.save_settings(&settings.revision, &value).unwrap();
    let service =
        ApiService::production(paths, Arc::new(MemoryApiEventEmitter::default())).unwrap();
    let snapshot = service
        .refresh(RefreshRequestDto {
            scope: RefreshScopeDto::Provider {
                provider_id: "mise".into(),
            },
            force: true,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.installations.len(), 1);
    assert_eq!(snapshot.installations[0].provider_id, "mise");
    assert_eq!(snapshot.installations[0].scope, "global");
    assert_eq!(
        snapshot.tools[0].components[0].status.status,
        "update_available"
    );
    assert!(snapshot
        .providers
        .iter()
        .find(|report| report.provider_id == "mise")
        .unwrap()
        .errors
        .is_empty());
}

#[tokio::test]
async fn production_reports_failed_mise_probe_without_scanning_others() {
    let fake = FakeMise::new();
    fs::write(&fake.program, "#!/bin/sh\nexit 1\n").unwrap();
    let paths = AppPaths::new(
        fake.temporary.path().join("config"),
        fake.temporary.path().join("data"),
        fake.temporary.path().join("cache"),
    );
    let store = PersistenceStore::new(paths.clone());
    let settings = store.load_settings().unwrap();
    let mut value = settings.value;
    value
        .executable_overrides
        .insert("mise".into(), fake.program.clone());
    store.save_settings(&settings.revision, &value).unwrap();
    let service =
        ApiService::production(paths, Arc::new(MemoryApiEventEmitter::default())).unwrap();
    let snapshot = service
        .refresh(RefreshRequestDto {
            scope: RefreshScopeDto::Provider {
                provider_id: "mise".into(),
            },
            force: true,
        })
        .await
        .unwrap();
    let report = snapshot
        .providers
        .iter()
        .find(|report| report.provider_id == "mise")
        .unwrap();
    assert!(!report.errors.is_empty());
    assert!(snapshot.installations.is_empty());
}
