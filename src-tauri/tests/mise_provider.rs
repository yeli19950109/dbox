#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dbox_lib::api::{ApiService, MemoryApiEventEmitter, RefreshRequestDto, RefreshScopeDto};
use dbox_lib::domain::{ComponentId, Installation, InstallationScope, StrategyId, Tool};
use dbox_lib::executor::CommandSpec;
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use dbox_lib::providers::{MiseProvider, MiseProviderOptions, ProviderUpdateRequest, ToolProvider};
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
