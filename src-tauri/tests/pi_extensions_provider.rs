#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dbox_lib::domain::{ComponentId, Installation, InstallationScope, PackageKind, StrategyId};
use dbox_lib::providers::{
    PiExtensionsProvider, PiExtensionsProviderOptions, ProviderUpdateRequest, ToolProvider,
};
use dbox_lib::version::{ComponentStatus, VersionValue};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Fixture {
    temporary: TempDir,
    npm: PathBuf,
    pi: PathBuf,
    agent: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let npm = temporary.path().join("npm");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-pi-extension-npm.sh"),
            &npm,
        )
        .unwrap();
        fs::set_permissions(&npm, fs::Permissions::from_mode(0o755)).unwrap();
        let pi = temporary.path().join("pi");
        // It must never execute during provider discovery, checking, or planning.
        fs::write(&pi, "#!/bin/sh\nexit 99\n").unwrap();
        fs::set_permissions(&pi, fs::Permissions::from_mode(0o755)).unwrap();
        let agent = temporary.path().join("agent");
        fs::create_dir_all(&agent).unwrap();
        Self {
            temporary,
            npm,
            pi,
            agent,
        }
    }

    fn provider(&self) -> PiExtensionsProvider {
        PiExtensionsProvider::new(&self.pi, &self.npm, &self.agent)
    }

    fn settings(&self, value: Value) {
        fs::write(
            self.agent.join("settings.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }

    fn package(&self, name: &str, version: &str) {
        let path = self.agent.join("npm/node_modules").join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("package.json"),
            serde_json::to_vec(&json!({"name": name, "version": version})).unwrap(),
        )
        .unwrap();
    }

    fn response_file(&self, spec: &str, extension: &str, content: &str) {
        let path = self
            .temporary
            .path()
            .join("responses")
            .join(format!("{spec}.{extension}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn response(&self, spec: &str, value: Value) {
        self.response_file(spec, "json", &value.to_string());
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
async fn discovers_only_configured_managed_npm_packages_and_reports_each_package() {
    let fixture = Fixture::new();
    fixture.settings(json!({
        "apiKey": "never-print-this",
        "packages": ["npm:one", {"source": "npm:@scope/two@^2", "extensions": [], "skills": ["skills/review"], "autoload": false}, "git:github.com/example/project", "./local-extension"]
    }));
    fixture.package("one", "1.0.0");
    fixture.package("@scope/two", "2.1.0");
    fixture.package("transitive-dependency", "9.0.0");
    let provider = fixture.provider();
    let status = provider.probe().await.unwrap();
    assert!(status.available);
    let detail = status.detail.unwrap();
    assert!(detail.contains("Git/local") && detail.contains("legacy global npm"));
    assert!(!detail.contains("never-print-this"));
    let mut events = Vec::new();
    let installations = provider
        .scan_with_progress(&mut |event| events.push(event))
        .await
        .unwrap();
    assert_eq!(installations.len(), 2);
    assert!(installations
        .iter()
        .all(
            |installation| installation.provider_id.as_str() == "pi-extensions"
                && installation.package.kind == PackageKind::Other("pi_npm".into())
                && installation.scope == InstallationScope::User
                && installation.executables.is_empty()
        ));
    for installation in &installations {
        assert_eq!(
            installation.install_path.as_ref().unwrap(),
            &fixture
                .agent
                .join("npm/node_modules")
                .join(&installation.package.name)
        );
        assert!(
            events
                .iter()
                .any(|event| event.package_name.as_deref()
                    == Some(installation.package.name.as_str()))
        );
    }
    assert_eq!(events.last().unwrap().completed, Some(2));
    assert_eq!(events.last().unwrap().total, Some(2));
    assert!(fixture.calls().is_empty());
}

#[tokio::test]
async fn ranges_and_tags_use_the_original_spec_and_pins_never_claim_latest() {
    let fixture = Fixture::new();
    fixture.settings(json!({"packages": ["npm:one", "npm:@scope/two@^2", "npm:tagged@next", "npm:pinned@v1.0.0", "npm:stable@~1.0"]}));
    for (name, version) in [
        ("one", "1.0.0"),
        ("@scope/two", "2.1.0"),
        ("tagged", "1.0.0"),
        ("pinned", "1.0.0"),
        ("stable", "1.0.9"),
    ] {
        fixture.package(name, version);
    }
    fixture.response("one@latest", json!("1.1.0"));
    fixture.response("@scope/two@^2", json!(["2.1.2", "2.10.0", "2.9.0"]));
    fixture.response("tagged@next", json!("2.0.0-beta.1"));
    fixture.response("stable@~1.0", json!(["1.0.0", "1.0.9"]));
    let provider = fixture.provider();
    let installations = provider.scan().await.unwrap();
    let mut events = Vec::new();
    let checks = provider
        .check_updates_with_progress(&installations, &mut |event| events.push(event))
        .await
        .unwrap();
    let by_name = |name| {
        let index = installations
            .iter()
            .position(|installation| installation.package.name == name)
            .unwrap();
        &checks[index]
    };
    assert_eq!(by_name("one").status, ComponentStatus::UpdateAvailable);
    assert_eq!(
        by_name("@scope/two").latest_version.as_ref().unwrap().raw(),
        "2.10.0"
    );
    assert_eq!(by_name("tagged").status, ComponentStatus::UpdateAvailable);
    assert_eq!(by_name("stable").status, ComponentStatus::UpToDate);
    assert!(matches!(
        by_name("pinned").status,
        ComponentStatus::Unsupported { .. }
    ));
    assert!(by_name("pinned").latest_version.is_none());
    assert_eq!(fixture.calls().len(), 4);
    assert!(fixture
        .calls()
        .contains(&"[<view><@scope/two@^2><version><--json>]".into()));
    assert!(!fixture.calls().iter().any(|call| call.contains("pinned")));
    assert_eq!(
        events
            .iter()
            .map(|event| event.completed.unwrap())
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5]
    );
    assert!(events[1..]
        .iter()
        .all(|event| event.package_name.is_some() && event.total == Some(5)));
}

#[tokio::test]
async fn missing_and_invalid_metadata_are_unknown_without_network_requests() {
    let fixture = Fixture::new();
    fixture.settings(
        json!({"packages": ["npm:missing", "npm:invalid", "npm:wrong-name", "npm:broken"]}),
    );
    fixture.package("invalid", "error 1.2.3 bad version");
    fixture.package("wrong-name", "1.0.0");
    fs::write(
        fixture
            .agent
            .join("npm/node_modules/wrong-name/package.json"),
        r#"{"name":"different","version":"1.0.0"}"#,
    )
    .unwrap();
    fixture.package("broken", "1.0.0");
    fs::write(
        fixture.agent.join("npm/node_modules/broken/package.json"),
        "bad json",
    )
    .unwrap();
    let provider = fixture.provider();
    let installations = provider.scan().await.unwrap();
    let checks = provider.check_updates(&installations).await.unwrap();
    assert_eq!(checks.len(), 4);
    assert!(checks.iter().all(|check| check.installed_version.is_none()));
    assert!(checks.iter().all(
        |check| matches!(check.status, ComponentStatus::Unknown { .. })
            && check.latest_version.is_none()
    ));
    assert!(fixture.calls().is_empty());
    assert!(provider
        .plan_update(request(&installations[0]))
        .await
        .is_err());
}

#[tokio::test]
async fn registry_failure_and_timeout_are_isolated_and_never_reuse_a_successful_result() {
    let fixture = Fixture::new();
    fixture.settings(json!({"packages": ["npm:one", "npm:fails", "npm:slow", "npm:bad-response"]}));
    for name in ["one", "fails", "slow", "bad-response"] {
        fixture.package(name, "1.0.0");
    }
    fixture.response("one@latest", json!("1.1.0"));
    fixture.response("fails@latest", json!("1.1.0"));
    fixture.response("slow@latest", json!("1.1.0"));
    fixture.response("bad-response@latest", json!(["1.1.0", "not-a-version"]));
    fixture.response_file("slow@latest", "delay", "0.3");
    fixture.response_file("fails@latest", "exit", "1");
    let provider = PiExtensionsProvider::with_options(
        &fixture.pi,
        &fixture.npm,
        &fixture.agent,
        PiExtensionsProviderOptions {
            command_timeout: Duration::from_millis(120),
            max_concurrency: 2,
        },
    );
    let installations = provider.scan().await.unwrap();
    let mut events = Vec::new();
    let checks = provider
        .check_updates_with_progress(&installations, &mut |event| events.push(event))
        .await
        .unwrap();
    let one = installations
        .iter()
        .position(|installation| installation.package.name == "one")
        .unwrap();
    assert_eq!(checks[one].status, ComponentStatus::UpdateAvailable);
    assert_eq!(
        checks
            .iter()
            .filter(|check| matches!(check.status, ComponentStatus::CheckFailed { .. }))
            .count(),
        3
    );
    assert!(!serde_json::to_string(&checks)
        .unwrap()
        .contains("do-not-expose-this"));
    assert_eq!(events.last().unwrap().completed, Some(4));
    assert!(provider
        .plan_update(request(&installations[one]))
        .await
        .is_ok());
    fixture.response_file("one@latest", "exit", "1");
    let checks = provider.check_updates(&installations).await.unwrap();
    assert!(matches!(
        checks[one].status,
        ComponentStatus::CheckFailed { .. }
    ));
    assert!(provider
        .plan_update(request(&installations[one]))
        .await
        .is_err());
}

#[tokio::test]
async fn plans_exactly_one_source_and_rejects_changed_sources_versions_and_custom_targets() {
    let fixture = Fixture::new();
    fixture
        .settings(json!({"packages": [{"source": "npm:@scope/two@^2", "extensions": ["one.js"]}]}));
    fixture.package("@scope/two", "2.0.0");
    fixture.response("@scope/two@^2", json!("2.1.0"));
    let provider = fixture.provider();
    let installations = provider.scan().await.unwrap();
    assert!(provider
        .plan_update(request(&installations[0]))
        .await
        .is_err());
    provider.check_updates(&installations).await.unwrap();
    let plan = provider
        .plan_update(request(&installations[0]))
        .await
        .unwrap();
    assert_eq!(plan.program, fixture.pi.to_string_lossy());
    assert_eq!(
        plan.args,
        ["update", "--extension", "npm:@scope/two@^2", "--no-approve"]
    );
    assert!(!plan
        .args
        .iter()
        .any(|arg| arg == "--all" || arg == "--extensions"));
    assert!(provider.validate_update_plan(&plan).await.is_ok());
    let mut altered = plan.clone();
    altered.args = vec!["update".into(), "--all".into()];
    assert!(provider.validate_update_plan(&altered).await.is_err());
    let mut custom_target = request(&installations[0]);
    custom_target.target_version = Some(VersionValue::new("3.0.0"));
    assert!(provider.plan_update(custom_target).await.is_err());
    let mut wrong_component = request(&installations[0]);
    wrong_component.component_id = ComponentId::new("extensions").unwrap();
    assert!(provider.plan_update(wrong_component).await.is_err());
    let mut wrong_strategy = request(&installations[0]);
    wrong_strategy.strategy_id = StrategyId::new("custom").unwrap();
    assert!(provider.plan_update(wrong_strategy).await.is_err());
    fixture.settings(
        json!({"packages": [{"source": "npm:@scope/two@next", "extensions": ["one.js"]}]}),
    );
    assert!(provider
        .plan_update(request(&installations[0]))
        .await
        .unwrap_err()
        .summary
        .contains("changed"));
    fixture
        .settings(json!({"packages": [{"source": "npm:@scope/two@^2", "extensions": ["two.js"]}]}));
    assert!(provider
        .plan_update(request(&installations[0]))
        .await
        .is_err());
    fixture
        .settings(json!({"packages": [{"source": "npm:@scope/two@^2", "extensions": ["one.js"]}]}));
    fixture.package("@scope/two", "2.0.1");
    assert!(provider
        .plan_update(request(&installations[0]))
        .await
        .is_err());
    fixture.settings(json!({"packages": []}));
    assert!(provider
        .plan_update(request(&installations[0]))
        .await
        .is_err());
}

#[tokio::test]
async fn invalid_configuration_and_unsafe_names_fail_without_echoing_settings() {
    let fixture = Fixture::new();
    let provider = fixture.provider();
    assert!(provider.scan().await.unwrap().is_empty());
    for value in [
        json!([]),
        json!({"packages": "npm:one"}),
        json!({"packages": [7]}),
        json!({"packages": [{"source": 5}]}),
        json!({"packages": ["npm:one", "npm:one@next"]}),
    ] {
        fixture.settings(value);
        assert!(provider.scan().await.is_err());
    }
    for source in [
        "npm:../outside",
        "npm:@scope/../../outside",
        "npm:--global",
        "npm:@scope",
        "npm:with space",
        "npm:one@",
        "npm:one@file:/tmp/package",
        "npm:one@npm:another",
        "npm:bad%2fname",
        "npm:@scope/name/more",
        "npm:one\n",
    ] {
        fixture.settings(json!({"packages": [source], "apiKey": "never-print-this"}));
        let error = provider.scan().await.unwrap_err();
        assert!(!error.summary.contains("never-print-this") && !error.summary.contains(source));
    }
    fs::write(
        fixture.agent.join("settings.json"),
        "{\"apiKey\":\"never-print-this\"",
    )
    .unwrap();
    assert!(!provider
        .scan()
        .await
        .unwrap_err()
        .summary
        .contains("never-print-this"));
    assert!(fixture.calls().is_empty());
}

#[tokio::test]
async fn custom_npm_commands_are_explicitly_unsupported_and_default_command_is_accepted() {
    let fixture = Fixture::new();
    let provider = fixture.provider();
    for command in [
        json!(["npm", "--registry=https://example.invalid"]),
        json!(["wrapper", "npm"]),
        json!(["bun"]),
        json!("npm"),
    ] {
        fixture.settings(json!({"npmCommand": command, "packages": ["npm:one"]}));
        assert!(provider
            .probe()
            .await
            .unwrap_err()
            .summary
            .contains("npmCommand"));
    }
    for command in [json!(null), json!([]), json!(["npm"])] {
        fixture.settings(json!({"npmCommand": command, "packages": []}));
        assert!(provider.probe().await.unwrap().available);
    }
}

#[tokio::test]
async fn changed_packages_invalidate_old_plans_and_installation_ids_include_the_agent_directory() {
    let fixture = Fixture::new();
    fixture.settings(json!({"packages": ["npm:one"]}));
    fixture.package("one", "1.0.0");
    fixture.response("one@latest", json!("1.1.0"));
    let provider = fixture.provider();
    let installations = provider.scan().await.unwrap();
    provider.check_updates(&installations).await.unwrap();
    assert!(provider
        .plan_update(request(&installations[0]))
        .await
        .is_ok());
    fixture.package("one", "1.0.1");
    provider.scan().await.unwrap();
    assert!(provider
        .plan_update(request(&installations[0]))
        .await
        .is_err());
    let other = Fixture::new();
    other.settings(json!({"packages": ["npm:one"]}));
    other.package("one", "1.0.0");
    let other_installations = other.provider().scan().await.unwrap();
    assert_ne!(other_installations[0].id, installations[0].id);
    let checks = provider.check_updates(&other_installations).await.unwrap();
    assert!(matches!(checks[0].status, ComponentStatus::Unknown { .. }));
}

#[tokio::test]
async fn updating_one_package_preserves_the_other_packages_checked_plan() {
    let fixture = Fixture::new();
    fixture.settings(json!({"packages": ["npm:one", "npm:two"]}));
    for name in ["one", "two"] {
        fixture.package(name, "1.0.0");
        fixture.response(&format!("{name}@latest"), json!("1.1.0"));
    }
    let provider = fixture.provider();
    let installations = provider.scan().await.unwrap();
    provider.check_updates(&installations).await.unwrap();
    fixture.package("one", "1.1.0");
    provider.scan().await.unwrap();
    let one = installations
        .iter()
        .find(|installation| installation.package.name == "one")
        .unwrap();
    let two = installations
        .iter()
        .find(|installation| installation.package.name == "two")
        .unwrap();
    assert!(provider.plan_update(request(one)).await.is_err());
    assert!(provider.plan_update(request(two)).await.is_ok());
    assert_eq!(
        fs::read_to_string(fixture.temporary.path().join("query-cwd"))
            .unwrap()
            .trim(),
        fixture.agent.canonicalize().unwrap().to_string_lossy()
    );
    fs::write(fixture.agent.join("settings.json"), "invalid json").unwrap();
    assert!(provider.scan().await.is_err());
    fixture.settings(json!({"packages": ["npm:one", "npm:two"]}));
    assert!(provider.plan_update(request(two)).await.is_err());
}
