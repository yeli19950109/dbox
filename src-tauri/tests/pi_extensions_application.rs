#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dbox_lib::application::{
    ApplicationEnvironment, ApplicationError, ApplicationService, ApplicationServiceOptions,
    ConfirmUpdateRequest, RefreshRequest, UpdateSelection,
};
use dbox_lib::catalog::{built_in_manifests, Catalog};
use dbox_lib::domain::{
    ComponentId, Installation, InstallationScope, PackageCoordinate, PackageKind, ProviderId,
    RunId, RunStatus, StrategyId,
};
use dbox_lib::executor::{CommandExecutor, NoopEventSink};
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use dbox_lib::providers::{FakeProvider, PiExtensionsProvider, ProviderRegistry, UpdateCheck};
use dbox_lib::version::{ComponentStatus, ToolStatus, VerificationStatus, VersionValue};
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const EXTENSION_NAME: &str = "@scope/pi-tools";
const EXTENSION_SOURCE: &str = "npm:@scope/pi-tools@^1.0.0";

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
        "pi-extensions-fixture".into()
    }
}

struct Fixture {
    _temporary: TempDir,
    root: PathBuf,
    pi: PathBuf,
    service: ApplicationService,
    store: Arc<PersistenceStore>,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path().join("Pi fixture with spaces");
        let agent_dir = root.join("agent");
        let extension_dir = agent_dir.join("npm/node_modules").join(EXTENSION_NAME);
        let other_dir = agent_dir.join("npm/node_modules/pi-other");
        fs::create_dir_all(&extension_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();
        fs::write(
            agent_dir.join("settings.json"),
            serde_json::to_vec(&json!({
                "packages": [
                    { "source": EXTENSION_SOURCE, "extensions": ["extensions/*.ts"] },
                    "npm:pi-other"
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            extension_dir.join("package.json"),
            serde_json::to_vec(&json!({"name": EXTENSION_NAME, "version": "1.0.0"})).unwrap(),
        )
        .unwrap();
        fs::write(
            other_dir.join("package.json"),
            r#"{"name":"pi-other","version":"2.0.0"}"#,
        )
        .unwrap();

        let pi = root.join("pi");
        write_program(
            &pi,
            r##"#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "$#" -eq 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' '1.2.0'
  exit 0
fi
[ "$#" -eq 4 ] || exit 64
[ "$1" = "update" ] || exit 64
[ "$2" = "--extension" ] || exit 64
[ "$4" = "--no-approve" ] || exit 64
case "$3" in
  'npm:@scope/pi-tools@^1.0.0')
    package_dir='@scope/pi-tools'
    metadata='{"name":"@scope/pi-tools","version":"1.1.0"}'
    ;;
  'npm:pi-other')
    package_dir='pi-other'
    metadata='{"name":"pi-other","version":"2.0.0"}'
    ;;
  *) exit 64 ;;
esac
printf '%s\n' "$@" > "$root/last-update.args"
printf '%s\n' "$3" >> "$root/update-calls.log"
if [ ! -f "$root/keep-version" ]; then
  printf '%s\n' "$metadata" > "$root/agent/npm/node_modules/$package_dir/package.json"
fi
printf '%s\n' 'Updated fixture extension'
"##,
        );
        let npm = root.join("npm");
        write_program(
            &npm,
            r##"#!/bin/sh
set -eu
if [ "$#" -eq 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' '11.0.0'
  exit 0
fi
[ "$#" -eq 4 ] || exit 64
[ "$1" = "view" ] || exit 64
[ "$3" = "version" ] || exit 64
[ "$4" = "--json" ] || exit 64
case "$2" in
  '@scope/pi-tools@^1.0.0') printf '%s\n' '["1.0.0","1.1.0"]' ;;
  'pi-other@latest') printf '%s\n' '"2.0.0"' ;;
  *) exit 64 ;;
esac
"##,
        );

        let mut pi_core = Installation::new(
            ProviderId::new("npm-global").unwrap(),
            PackageCoordinate::new(PackageKind::NpmGlobal, "@earendil-works/pi-coding-agent")
                .unwrap(),
            InstallationScope::Global,
        )
        .unwrap();
        pi_core.installed_version = Some(VersionValue::new("1.2.0"));
        let npm_global = FakeProvider::new(ProviderId::new("npm-global").unwrap())
            .with_scan_result(Ok(vec![pi_core.clone()]))
            .with_check_result(Ok(vec![UpdateCheck {
                installation_id: pi_core.id.clone(),
                component_id: ComponentId::new("core").unwrap(),
                installed_version: pi_core.installed_version.clone(),
                latest_version: pi_core.installed_version,
                status: ComponentStatus::UpToDate,
            }]));
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(npm_global)).unwrap();
        registry
            .register(Arc::new(PiExtensionsProvider::new(&pi, &npm, &agent_dir)))
            .unwrap();
        let store = Arc::new(PersistenceStore::new(AppPaths::new(
            root.join("config"),
            root.join("data"),
            root.join("logs"),
        )));
        let executor =
            CommandExecutor::new(Some(Arc::clone(&store)), Arc::new(NoopEventSink), 4096);
        let service = ApplicationService::new(
            Arc::new(registry),
            Catalog::build(built_in_manifests()).unwrap(),
            Arc::clone(&store),
            executor,
            Arc::new(FixtureEnvironment),
            ApplicationServiceOptions::default(),
        )
        .unwrap();
        Self {
            _temporary: temporary,
            root,
            pi,
            service,
            store,
        }
    }
}

fn write_program(path: &Path, source: &str) {
    fs::write(path, source).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn pi_extension_update_is_independent_of_core_and_verifies_its_own_version() {
    let fixture = Fixture::new();
    let snapshot = fixture
        .service
        .refresh_tools(RefreshRequest::all())
        .await
        .unwrap();
    assert_eq!(snapshot.tools.len(), 3);
    assert!(snapshot
        .providers
        .values()
        .all(|report| report.errors.is_empty()));
    let pi = snapshot
        .tools
        .iter()
        .find(|tool| tool.display_name == "Pi")
        .unwrap();
    assert_eq!(pi.components.len(), 1);
    assert_eq!(pi.status(), ToolStatus::UpToDate);
    let extension = snapshot
        .tools
        .iter()
        .find(|tool| tool.display_name == EXTENSION_NAME)
        .unwrap();
    assert_eq!(extension.components.len(), 1);
    assert_eq!(
        extension.components[0].status,
        ComponentStatus::UpdateAvailable
    );
    assert_ne!(extension.installation_ids, pi.installation_ids);
    assert_eq!(
        extension.components[0].installation_id.as_ref(),
        extension.installation_ids.first()
    );
    let plan = fixture
        .service
        .preview_updates(&[UpdateSelection {
            tool_id: extension.id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: Some(StrategyId::new("provider-default").unwrap()),
            target_version: None,
        }])
        .await
        .unwrap()
        .remove(0);
    assert_eq!(plan.installation_id, extension.installation_ids[0]);
    assert_eq!(plan.command.program, fixture.pi);
    assert_eq!(
        plan.command.args,
        ["update", "--extension", EXTENSION_SOURCE, "--no-approve"]
    );
    let result = fixture
        .service
        .confirm_update(
            ConfirmUpdateRequest {
                plan_id: plan.plan_id,
                plan_hash: plan.plan_hash,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.execution.status, RunStatus::Succeeded);
    assert_eq!(
        result.execution.verification,
        Some(VerificationStatus::Verified)
    );
    let updated = result
        .snapshot
        .tools
        .iter()
        .find(|tool| tool.id == extension.id)
        .unwrap();
    assert_eq!(
        updated.components[0].installed_version,
        Some(VersionValue::new("1.1.0"))
    );
    assert_eq!(
        updated.components[0].status,
        ComponentStatus::UpdateSucceeded {
            verification: VerificationStatus::Verified
        }
    );
    assert_eq!(
        result.snapshot.tools.iter().find(|tool| tool.id == pi.id),
        Some(pi)
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("last-update.args")).unwrap(),
        format!("update\n--extension\n{EXTENSION_SOURCE}\n--no-approve\n")
    );
    let stored = fixture.store.load_state().unwrap();
    assert_eq!(stored.value.runs.len(), 1);
    assert_eq!(stored.value.runs[0].tool_id, Some(extension.id.clone()));
    assert_eq!(
        stored
            .value
            .installations
            .iter()
            .find(|installation| installation.id == extension.installation_ids[0])
            .unwrap()
            .installed_version,
        Some(VersionValue::new("1.1.0"))
    );
    let refreshed = fixture
        .service
        .refresh_tools(RefreshRequest::force_all())
        .await
        .unwrap();
    assert_eq!(
        refreshed
            .tools
            .iter()
            .find(|tool| tool.id == extension.id)
            .unwrap()
            .status(),
        ToolStatus::UpToDate
    );
}

#[tokio::test]
async fn a_successful_command_without_an_extension_version_change_is_not_verified() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("keep-version"), "").unwrap();
    let snapshot = fixture
        .service
        .refresh_tools(RefreshRequest::all())
        .await
        .unwrap();
    let extension = snapshot
        .tools
        .iter()
        .find(|tool| tool.display_name == EXTENSION_NAME)
        .unwrap();
    let plan = fixture
        .service
        .preview_updates(&[UpdateSelection {
            tool_id: extension.id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: None,
            target_version: None,
        }])
        .await
        .unwrap()
        .remove(0);
    let result = fixture
        .service
        .confirm_update(
            ConfirmUpdateRequest {
                plan_id: plan.plan_id,
                plan_hash: plan.plan_hash,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.execution.exit_code, Some(0));
    assert_eq!(result.execution.status, RunStatus::Partial);
    assert!(matches!(
        result.execution.verification,
        Some(VerificationStatus::VerificationUnknown { ref reason })
            if reason.code == "version_unchanged"
    ));
    assert_eq!(
        result
            .snapshot
            .tools
            .iter()
            .find(|tool| tool.id == extension.id)
            .unwrap()
            .components[0]
            .installed_version,
        Some(VersionValue::new("1.0.0"))
    );
}

#[tokio::test]
async fn one_batch_updates_and_verifies_each_pi_package_independently() {
    let fixture = Fixture::new();
    fs::write(
        fixture
            .root
            .join("agent/npm/node_modules/pi-other/package.json"),
        r#"{"name":"pi-other","version":"1.0.0"}"#,
    )
    .unwrap();
    let snapshot = fixture
        .service
        .refresh_tools(RefreshRequest::all())
        .await
        .unwrap();
    let selections = snapshot
        .tools
        .iter()
        .filter(|tool| tool.status() == ToolStatus::UpdateAvailable)
        .map(|tool| UpdateSelection {
            tool_id: tool.id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: None,
            target_version: None,
        })
        .collect::<Vec<_>>();
    assert_eq!(selections.len(), 2);
    let plans = fixture.service.preview_updates(&selections).await.unwrap();
    assert_eq!(plans.len(), 2);
    let requests = plans
        .into_iter()
        .enumerate()
        .map(|(index, plan)| {
            (
                ConfirmUpdateRequest {
                    plan_id: plan.plan_id,
                    plan_hash: plan.plan_hash,
                },
                RunId::new(format!("pi-batch-{index}")).unwrap(),
                CancellationToken::new(),
            )
        })
        .collect();
    let results = fixture
        .service
        .confirm_updates_with_run_ids(requests)
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    for result in results {
        let result = result.unwrap();
        assert_eq!(result.execution.status, RunStatus::Succeeded);
        assert_eq!(
            result.execution.verification,
            Some(VerificationStatus::Verified)
        );
    }
    let updated = fixture.service.snapshot().await;
    for (package, version) in [(EXTENSION_NAME, "1.1.0"), ("pi-other", "2.0.0")] {
        let tool = updated
            .tools
            .iter()
            .find(|tool| tool.display_name == package)
            .unwrap();
        assert_eq!(
            tool.components[0].installed_version,
            Some(VersionValue::new(version))
        );
        assert_eq!(
            tool.components[0].status,
            ComponentStatus::UpdateSucceeded {
                verification: VerificationStatus::Verified
            }
        );
    }
    assert_eq!(
        updated.tools.iter().find(|tool| tool.display_name == "Pi"),
        snapshot.tools.iter().find(|tool| tool.display_name == "Pi")
    );
    let mut calls = fs::read_to_string(fixture.root.join("update-calls.log"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    calls.sort();
    assert_eq!(calls, [EXTENSION_SOURCE, "npm:pi-other"]);
    assert_eq!(fixture.store.load_state().unwrap().value.runs.len(), 2);
}

#[tokio::test]
async fn changed_pi_source_after_preview_rejects_single_and_entire_batch_before_execution() {
    for batch in [false, true] {
        let fixture = Fixture::new();
        if batch {
            fs::write(
                fixture
                    .root
                    .join("agent/npm/node_modules/pi-other/package.json"),
                r#"{"name":"pi-other","version":"1.0.0"}"#,
            )
            .unwrap();
        }
        let snapshot = fixture
            .service
            .refresh_tools(RefreshRequest::all())
            .await
            .unwrap();
        let extension = snapshot
            .tools
            .iter()
            .find(|tool| tool.display_name == EXTENSION_NAME)
            .unwrap();
        let mut selections = snapshot
            .tools
            .iter()
            .filter(|tool| tool.status() == ToolStatus::UpdateAvailable)
            .map(|tool| UpdateSelection {
                tool_id: tool.id.clone(),
                component_id: ComponentId::new("core").unwrap(),
                strategy_id: None,
                target_version: None,
            })
            .collect::<Vec<_>>();
        assert_eq!(selections.len(), if batch { 2 } else { 1 });
        // Prepare the unchanged package first, so a late validation failure must
        // prevent the whole batch from starting, including that first package.
        selections.sort_by_key(|selection| selection.tool_id == extension.id);
        let plans = fixture.service.preview_updates(&selections).await.unwrap();
        fs::write(
            fixture.root.join("agent/settings.json"),
            serde_json::to_vec(&json!({
                "packages": [
                    {
                        "source": "npm:@scope/pi-tools@^2.0.0",
                        "extensions": ["extensions/*.ts"]
                    },
                    "npm:pi-other"
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let error = if batch {
            let requests = plans
                .into_iter()
                .enumerate()
                .map(|(index, plan)| {
                    (
                        ConfirmUpdateRequest {
                            plan_id: plan.plan_id,
                            plan_hash: plan.plan_hash,
                        },
                        RunId::new(format!("changed-pi-batch-{index}")).unwrap(),
                        CancellationToken::new(),
                    )
                })
                .collect();
            fixture
                .service
                .confirm_updates_with_run_ids(requests)
                .await
                .unwrap_err()
        } else {
            let plan = plans.into_iter().next().unwrap();
            fixture
                .service
                .confirm_update(
                    ConfirmUpdateRequest {
                        plan_id: plan.plan_id,
                        plan_hash: plan.plan_hash,
                    },
                    CancellationToken::new(),
                )
                .await
                .unwrap_err()
        };
        assert!(matches!(error, ApplicationError::Provider(_)));
        assert!(!fixture.root.join("last-update.args").exists());
        assert!(!fixture.root.join("update-calls.log").exists());
        assert!(fixture.store.load_state().unwrap().value.runs.is_empty());
        assert_eq!(fixture.service.snapshot().await.tools, snapshot.tools);
    }
}
