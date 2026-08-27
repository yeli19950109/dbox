#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use dbox_lib::domain::{ComponentId, PackageKind, StrategyId, Tool};
use dbox_lib::providers::{
    HomebrewProvider, ProviderOperation, ProviderUpdateRequest, Recoverability, ToolProvider,
};
use dbox_lib::version::ComponentStatus;
use serde_json::json;
use tempfile::TempDir;

struct FakeBrew {
    temporary: TempDir,
    program: PathBuf,
}

impl FakeBrew {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let program = temporary.path().join("brew");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-brew.sh"),
            &program,
        )
        .unwrap();
        let mut permissions = fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&program, permissions).unwrap();
        fs::create_dir_all(temporary.path().join("prefix")).unwrap();
        fs::create_dir_all(temporary.path().join("lists")).unwrap();
        Self { temporary, program }
    }

    fn provider(&self) -> HomebrewProvider {
        HomebrewProvider::new(&self.program)
    }

    fn write_info(&self, value: serde_json::Value) {
        fs::write(
            self.temporary.path().join("info.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }

    fn write_raw_info(&self, value: &str) {
        fs::write(self.temporary.path().join("info.json"), value).unwrap();
    }

    fn write_formula_files(&self, formula: &str, files: &[PathBuf]) {
        let path = self
            .temporary
            .path()
            .join("lists")
            .join(formatula_path(formula));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let body = files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, format!("{body}\n")).unwrap();
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
        let _ = fs::remove_file(self.temporary.path().join("outdated.stderr"));
    }

    fn fail_outdated(&self, stderr: &str) {
        fs::write(self.temporary.path().join("outdated.json"), "{}").unwrap();
        fs::write(self.temporary.path().join("outdated.exit"), "1").unwrap();
        fs::write(self.temporary.path().join("outdated.stderr"), stderr).unwrap();
    }

    fn prefix(&self) -> PathBuf {
        self.temporary.path().join("prefix")
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.temporary.path().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn formatula_path(name: &str) -> PathBuf {
    PathBuf::from(format!("{name}.txt"))
}

fn installed_info() -> serde_json::Value {
    json!({
        "formulae": [
            {
                "name": "shared",
                "installed": [{ "version": "1.0.0" }],
                "linked_keg": "1.0.0",
                "pinned": false,
                "keg_only": false,
                "deprecated": false,
                "disabled": false,
                "future_field": { "is": "ignored" }
            },
            {
                "name": "pinned-tool",
                "installed": [{ "version": "2.0.0" }],
                "linked_keg": "2.0.0",
                "pinned": true,
                "keg_only": false,
                "deprecated": false,
                "disabled": false
            },
            {
                "name": "multi-keg",
                "installed": [{ "version": "1.0.0" }, { "version": "1.1.0" }],
                "linked_keg": "1.1.0",
                "pinned": false,
                "keg_only": true,
                "deprecated": true,
                "disabled": false
            }
        ],
        "casks": [
            {
                "token": "shared",
                "installed": ["5.0.0"],
                "deprecated": false,
                "disabled": false,
                "artifacts": [{ "binary": ["Shared.app/Contents/MacOS/shared-cask", { "target": "shared-cli" }] }]
            },
            {
                "token": "no-cli",
                "installed": ["1.0.0"],
                "deprecated": false,
                "disabled": true,
                "artifacts": [{ "app": ["No CLI.app"] }]
            }
        ]
    })
}

#[tokio::test]
async fn fake_brew_completes_probe_scan_check_and_precise_formula_cask_plans() {
    let fake = FakeBrew::new();
    fake.write_info(installed_info());
    fake.write_formula_files(
        "shared",
        &[
            fake.prefix().join("Cellar/shared/1.0.0/bin/shared"),
            fake.prefix().join("Cellar/shared/1.0.0/share/readme"),
        ],
    );
    fake.write_formula_files("pinned-tool", &[]);
    fake.write_formula_files(
        "multi-keg",
        &[fake
            .prefix()
            .join("Cellar/multi-keg/1.1.0/sbin/multi-admin")],
    );
    fake.write_outdated(
        json!({
            "formulae": [
                { "name": "shared", "installed_versions": ["1.0.0"], "current_version": "1.2.0", "unknown": true },
                { "name": "multi-keg", "installed_versions": ["1.1.0"] }
            ],
            "casks": [
                { "name": "shared", "installed_versions": ["5.0.0"], "current_version": "6.0.0" }
            ],
            "future": []
        }),
        0,
    );

    let provider = fake.provider();
    let status = provider.probe().await.unwrap();
    assert_eq!(status.version.unwrap().raw(), "4.6.0");
    assert!(status.detail.unwrap().contains("architecture=arm64"));

    let installations = provider.scan().await.unwrap();
    assert_eq!(installations.len(), 5);
    let formula = installations
        .iter()
        .find(|installation| {
            installation.package.kind == PackageKind::HomebrewFormula
                && installation.package.name == "shared"
        })
        .unwrap();
    let cask = installations
        .iter()
        .find(|installation| {
            installation.package.kind == PackageKind::HomebrewCask
                && installation.package.name == "shared"
        })
        .unwrap();
    assert_eq!(formula.id.as_str(), "homebrew-formula:shared");
    assert_eq!(cask.id.as_str(), "homebrew-cask:shared");
    assert_ne!(formula.id, cask.id);
    assert_eq!(formula.executables[0].name, "shared");
    assert_eq!(cask.executables[0].name, "shared-cli");
    assert!(installations
        .iter()
        .find(|installation| installation.package.name == "no-cli")
        .unwrap()
        .executables
        .is_empty());

    let tools: Vec<_> = installations
        .iter()
        .map(Tool::from_installation)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(tools.len(), 5, "catalog is not required for generic tools");

    let checks = provider.check_updates(&installations).await.unwrap();
    assert!(matches!(
        checks
            .iter()
            .find(|check| check.installation_id == formula.id)
            .unwrap()
            .status,
        ComponentStatus::UpdateAvailable
    ));
    assert!(matches!(
        checks
            .iter()
            .find(|check| {
                installations
                    .iter()
                    .find(|installation| installation.id == check.installation_id)
                    .is_some_and(|installation| installation.package.name == "pinned-tool")
            })
            .unwrap()
            .status,
        ComponentStatus::Unsupported { .. }
    ));
    assert!(matches!(
        checks
            .iter()
            .find(|check| {
                installations
                    .iter()
                    .find(|installation| installation.id == check.installation_id)
                    .is_some_and(|installation| installation.package.name == "multi-keg")
            })
            .unwrap()
            .status,
        ComponentStatus::CheckFailed { .. }
    ));

    let formula_plan = provider
        .plan_update(ProviderUpdateRequest {
            installation_id: formula.id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: StrategyId::new("provider-default").unwrap(),
            target_version: None,
        })
        .await
        .unwrap();
    let cask_plan = provider
        .plan_update(ProviderUpdateRequest {
            installation_id: cask.id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: StrategyId::new("provider-default").unwrap(),
            target_version: None,
        })
        .await
        .unwrap();
    assert_eq!(formula_plan.args, ["upgrade", "--formula", "--", "shared"]);
    assert_eq!(cask_plan.args, ["upgrade", "--cask", "--", "shared"]);
    assert!(!fake
        .calls()
        .iter()
        .any(|call| call.starts_with("[<upgrade>")));

    let diagnostic_codes: Vec<_> = provider
        .diagnostics()
        .into_iter()
        .map(|diagnostic| diagnostic.code)
        .collect();
    assert!(diagnostic_codes.contains(&"pinned".into()));
    assert!(diagnostic_codes.contains(&"keg_only".into()));
    assert!(diagnostic_codes.contains(&"multiple_versions".into()));
    assert!(diagnostic_codes.contains(&"disabled".into()));
}

#[tokio::test]
async fn pinned_formula_is_never_unpinned_implicitly() {
    let fake = FakeBrew::new();
    fake.write_info(installed_info());
    for name in ["shared", "pinned-tool", "multi-keg"] {
        fake.write_formula_files(name, &[]);
    }
    let provider = fake.provider();
    let pinned = provider
        .scan()
        .await
        .unwrap()
        .into_iter()
        .find(|installation| installation.package.name == "pinned-tool")
        .unwrap();
    let error = provider
        .plan_update(ProviderUpdateRequest {
            installation_id: pinned.id,
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: StrategyId::new("provider-default").unwrap(),
            target_version: None,
        })
        .await
        .unwrap_err();
    assert!(error.summary.contains("unpinned explicitly"));
    assert!(!fake.calls().iter().any(|call| call.contains("<unpin>")));
}

#[tokio::test]
async fn remote_and_permission_failures_retain_installed_versions() {
    let fake = FakeBrew::new();
    fake.write_info(json!({
        "formulae": [{
            "name": "tool", "installed": [{ "version": "1.0.0" }],
            "linked_keg": "1.0.0", "pinned": false, "disabled": false
        }],
        "casks": []
    }));
    fake.write_formula_files("tool", &[]);
    fake.fail_outdated("Error: /opt/homebrew is not writable; TOKEN=secret-value\n");
    let provider = fake.provider();
    let installations = provider.scan().await.unwrap();
    let checks = provider.check_updates(&installations).await.unwrap();
    assert_eq!(checks[0].installed_version.as_ref().unwrap().raw(), "1.0.0");
    let ComponentStatus::CheckFailed { reason } = &checks[0].status else {
        panic!("expected a failed remote check")
    };
    assert!(!reason.summary.contains("secret-value"));
    assert!(
        !reason.retryable,
        "permission failures require configuration"
    );
}

#[tokio::test]
async fn empty_and_malformed_json_have_structured_results() {
    let fake = FakeBrew::new();
    fake.write_info(json!({ "formulae": [], "casks": [] }));
    assert!(fake.provider().scan().await.unwrap().is_empty());

    fake.write_raw_info("not-json");
    let error = fake.provider().scan().await.unwrap_err();
    assert_eq!(error.operation, ProviderOperation::Scan);
    assert!(error.summary.contains("malformed JSON"));

    fake.write_info(json!({ "formulae": [] }));
    let error = fake.provider().scan().await.unwrap_err();
    assert!(error.summary.contains("casks"));
}

#[tokio::test]
async fn permission_error_is_classified_as_configuration_work() {
    let fake = FakeBrew::new();
    fake.write_info(json!({ "formulae": [], "casks": [] }));
    fs::write(fake.temporary.path().join("info.exit"), "1").unwrap();
    fs::write(
        fake.temporary.path().join("info.stderr"),
        "Error: prefix is not writable",
    )
    .unwrap();
    let error = fake.provider().scan().await.unwrap_err();
    assert_eq!(error.recoverability, Recoverability::RequiresConfiguration);
}
