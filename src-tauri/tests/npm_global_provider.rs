#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dbox_lib::domain::{ComponentId, StrategyId, Tool};
use dbox_lib::providers::npm_global::{NpmGlobalProvider, NpmProviderOptions};
use dbox_lib::providers::{ProviderUpdateRequest, ToolProvider};
use dbox_lib::version::{ComponentStatus, VersionValue};
use serde_json::json;
use tempfile::TempDir;

struct FakeNpm {
    temporary: TempDir,
    program: PathBuf,
    root: PathBuf,
}

impl FakeNpm {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let program = temporary.path().join("npm");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-npm.sh"),
            &program,
        )
        .unwrap();
        let mut permissions = fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&program, permissions).unwrap();
        let root = temporary.path().join("prefix/lib/node_modules");
        fs::create_dir_all(&root).unwrap();
        Self {
            temporary,
            program,
            root,
        }
    }

    fn provider(&self) -> NpmGlobalProvider {
        NpmGlobalProvider::new(&self.program)
    }

    fn provider_with_options(&self, options: NpmProviderOptions) -> NpmGlobalProvider {
        NpmGlobalProvider::with_options(&self.program, options)
    }

    fn write_list(&self, dependencies: serde_json::Value) {
        fs::write(
            self.temporary.path().join("list.json"),
            serde_json::to_vec(&json!({ "dependencies": dependencies })).unwrap(),
        )
        .unwrap();
    }

    fn write_raw_list(&self, value: &str) {
        fs::write(self.temporary.path().join("list.json"), value).unwrap();
    }

    fn write_outdated(&self, value: serde_json::Value, exit_code: i32) {
        fs::write(
            self.temporary.path().join("outdated.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        fs::write(
            self.temporary.path().join("outdated.exit"),
            exit_code.to_string(),
        )
        .unwrap();
    }

    fn write_raw_outdated(&self, value: &str, exit_code: i32) {
        fs::write(self.temporary.path().join("outdated.json"), value).unwrap();
        fs::write(
            self.temporary.path().join("outdated.exit"),
            exit_code.to_string(),
        )
        .unwrap();
    }

    fn package(&self, name: &str, metadata: serde_json::Value) {
        let path = self.root.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("package.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.temporary.path().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[tokio::test]
async fn fake_npm_completes_probe_scan_check_and_single_package_plan() {
    let fake = FakeNpm::new();
    fake.write_list(json!({
        "random-uncatalogued-cli": { "version": "1.0.0" },
        "@scope/tool": { "version": "2.0.0" },
        "multi-bin": { "version": "3.0.0", "extraneous": true },
        "no-bin": { "version": "4.0.0", "invalid": true },
        "npm": { "version": "10.9.1" },
        "missing-package": { "missing": true }
    }));
    fake.package(
        "random-uncatalogued-cli",
        json!({ "name": "random-uncatalogued-cli", "version": "1.0.0", "bin": "cli.js" }),
    );
    fake.package(
        "@scope/tool",
        json!({ "name": "@scope/tool", "version": "2.0.0", "bin": "index.js" }),
    );
    fake.package(
        "multi-bin",
        json!({ "version": "3.0.0", "bin": { "multi": "cli.js", "multi-lsp": "lsp.js" } }),
    );
    fake.package("no-bin", json!({ "version": "4.0.0" }));
    fake.package(
        "npm",
        json!({ "version": "10.9.1", "bin": { "npm": "bin/npm-cli.js", "npx": "bin/npx-cli.js" } }),
    );
    fake.write_outdated(
        json!({
            "random-uncatalogued-cli": { "current": "1.0.0", "latest": "1.1.0" },
            "npm": { "current": "10.9.1", "latest": "11.0.0" }
        }),
        1,
    );

    let provider = fake.provider();
    let status = provider.probe().await.unwrap();
    assert_eq!(status.version.unwrap().raw(), "10.9.1");
    let installations = provider.scan().await.unwrap();
    assert_eq!(installations.len(), 6, "every npm list entry is retained");

    let random = installations
        .iter()
        .find(|installation| installation.package.name == "random-uncatalogued-cli")
        .unwrap();
    assert_eq!(random.executables[0].name, "random-uncatalogued-cli");
    let scoped = installations
        .iter()
        .find(|installation| installation.package.name == "@scope/tool")
        .unwrap();
    assert_eq!(scoped.executables[0].name, "tool");
    let multi = installations
        .iter()
        .find(|installation| installation.package.name == "multi-bin")
        .unwrap();
    assert_eq!(
        multi
            .executables
            .iter()
            .map(|executable| executable.name.as_str())
            .collect::<Vec<_>>(),
        ["multi", "multi-lsp"]
    );
    assert!(installations
        .iter()
        .find(|installation| installation.package.name == "no-bin")
        .unwrap()
        .executables
        .is_empty());
    assert!(installations
        .iter()
        .find(|installation| installation.package.name == "missing-package")
        .unwrap()
        .installed_version
        .is_none());

    let tools: Vec<_> = installations
        .iter()
        .map(Tool::from_installation)
        .collect::<Result<_, _>>()
        .unwrap();
    let random_tool = tools
        .iter()
        .find(|tool| tool.display_name == "random-uncatalogued-cli")
        .unwrap();
    assert_eq!(random_tool.components.len(), 1);
    assert_eq!(random_tool.components[0].id.as_str(), "core");

    let checks = provider.check_updates(&installations).await.unwrap();
    let random_check = checks
        .iter()
        .find(|check| check.installation_id == random.id)
        .unwrap();
    assert!(matches!(
        random_check.status,
        ComponentStatus::UpdateAvailable
    ));
    let scoped_check = checks
        .iter()
        .find(|check| check.installation_id == scoped.id)
        .unwrap();
    assert!(matches!(scoped_check.status, ComponentStatus::UpToDate));
    assert_eq!(scoped_check.latest_version.as_ref().unwrap().raw(), "2.0.0");

    let plan = provider
        .plan_update(ProviderUpdateRequest {
            installation_id: scoped.id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: StrategyId::new("provider-default").unwrap(),
            target_version: None,
        })
        .await
        .unwrap();
    assert_eq!(plan.program, fake.program.display().to_string());
    assert_eq!(
        plan.args,
        ["install", "--global", "--", "@scope/tool@latest"]
    );
    assert_eq!(
        plan.args
            .iter()
            .filter(|argument| argument.ends_with("@latest"))
            .count(),
        1,
        "the plan targets exactly one package"
    );

    let diagnostics = provider.diagnostics();
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "extraneous_package"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "invalid_package"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "missing_package"));

    let calls = fake.calls();
    assert!(calls.contains(&"[<--version>]".into()));
    assert!(calls.contains(&"[<prefix><--global>]".into()));
    assert!(calls.contains(&"[<root><--global>]".into()));
    assert!(calls.contains(&"[<ls><--global><--depth=0><--json><--long>]".into()));
    assert!(calls
        .iter()
        .any(|call| call.starts_with("[<outdated><--global><--depth=0><--json><-->")));
    assert!(!calls.iter().any(|call| call.starts_with("[<install>")));
}

#[tokio::test]
async fn update_checks_cache_success_and_keep_installed_state_on_registry_failure() {
    let fake = FakeNpm::new();
    fake.write_list(json!({ "tool": { "version": "1.0.0" } }));
    fake.package(
        "tool",
        json!({ "name": "tool", "version": "1.0.0", "bin": "cli.js" }),
    );
    fake.write_outdated(json!({ "tool": { "latest": "2.0.0" } }), 1);
    let provider = fake.provider();
    let installations = provider.scan().await.unwrap();

    let first = provider.check_updates(&installations).await.unwrap();
    let second = provider.check_updates(&installations).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|call| call.starts_with("[<outdated>"))
            .count(),
        1,
        "a fresh cached result avoids another registry command"
    );

    provider.clear_update_cache();
    fake.write_raw_outdated("not-json", 1);
    let failed = provider.check_updates(&installations).await.unwrap();
    assert_eq!(failed[0].installed_version.as_ref().unwrap().raw(), "1.0.0");
    assert!(matches!(
        failed[0].status,
        ComponentStatus::CheckFailed { .. }
    ));
}

#[tokio::test]
async fn stale_cache_survives_offline_registry_and_queries_are_batched_with_a_limit() {
    let fake = FakeNpm::new();
    fake.write_list(json!({
        "one": { "version": "1.0.0" },
        "two": { "version": "1.0.0" },
        "three": { "version": "1.0.0" }
    }));
    for name in ["one", "two", "three"] {
        fake.package(name, json!({ "version": "1.0.0" }));
    }
    fake.write_outdated(json!({ "one": { "latest": "2.0.0" } }), 1);
    let provider = fake.provider_with_options(NpmProviderOptions {
        update_cache_ttl: Duration::ZERO,
        max_concurrency: 2,
        update_batch_size: 1,
        ..NpmProviderOptions::default()
    });
    let installations = provider.scan().await.unwrap();
    let online = provider.check_updates(&installations).await.unwrap();
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|call| call.starts_with("[<outdated>"))
            .count(),
        3
    );

    fake.write_outdated(json!({ "error": { "code": "ENETUNREACH" } }), 1);
    let offline = provider.check_updates(&installations).await.unwrap();
    assert_eq!(
        offline, online,
        "expired successful entries remain an offline fallback"
    );
}

#[tokio::test]
async fn two_global_roots_never_collide_for_the_same_package() {
    let first = FakeNpm::new();
    let second = FakeNpm::new();
    for fake in [&first, &second] {
        fake.write_list(json!({ "same-name": { "version": "1.0.0" } }));
        fake.package("same-name", json!({ "version": "1.0.0" }));
    }
    let first_installation = first.provider().scan().await.unwrap().remove(0);
    let second_installation = second.provider().scan().await.unwrap().remove(0);
    assert_ne!(first_installation.id, second_installation.id);
    assert_ne!(
        first_installation.install_path,
        second_installation.install_path
    );
}

#[tokio::test]
async fn empty_and_malformed_lists_are_distinguished_without_touching_real_npm() {
    let fake = FakeNpm::new();
    fake.write_list(json!({}));
    let provider = fake.provider();
    assert!(provider.scan().await.unwrap().is_empty());

    fake.write_raw_list("not-json");
    let error = provider.scan().await.unwrap_err();
    assert_eq!(
        error.operation,
        dbox_lib::providers::ProviderOperation::Scan
    );
    assert!(error.summary.contains("malformed JSON"));
    assert!(fake
        .calls()
        .iter()
        .all(|call| !call.starts_with("[<install>")));
}

#[tokio::test]
async fn plan_rejects_broad_or_cross_prefix_requests() {
    let first = FakeNpm::new();
    let second = FakeNpm::new();
    for fake in [&first, &second] {
        fake.write_list(json!({ "tool": { "version": "1.0.0" } }));
        fake.package("tool", json!({ "version": "1.0.0" }));
    }
    let foreign = second.provider().scan().await.unwrap().remove(0);
    let provider = first.provider();
    let error = provider
        .plan_update(ProviderUpdateRequest {
            installation_id: foreign.id,
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: StrategyId::new("provider-default").unwrap(),
            target_version: Some(VersionValue::new("latest")),
        })
        .await
        .unwrap_err();
    assert!(error.summary.contains("different npm global root"));
}
