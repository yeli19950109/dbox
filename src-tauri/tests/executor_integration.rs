#![cfg(unix)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use dbox_lib::domain::{ComponentId, InstallationId, RunId, RunStatus, StrategyId, ToolId};
use dbox_lib::executor::{
    CommandExecutor, CommandSpec, ConfirmationContext, NoopEventSink, PlanContext, UpdatePlan,
};
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn confirmed_plan_executes_fixture_and_persists_streams() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TempDir::new().unwrap();
    let fixture_dir = temporary.path().join("fixture path with spaces");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let fixture = fixture_dir.join("fake command");
    std::fs::write(&fixture, include_bytes!("fixtures/fake-command.sh")).unwrap();
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();

    let now = Utc::now();
    let plan = UpdatePlan::new(
        ToolId::new("integration-fixture").unwrap(),
        InstallationId::new("fixture:integration").unwrap(),
        ComponentId::new("core").unwrap(),
        StrategyId::new("fixture").unwrap(),
        CommandSpec::new(fixture, vec!["streams".into()]),
        PlanContext {
            config_revision: "config".into(),
            environment_revision: "environment".into(),
            version_snapshot: "version".into(),
            now,
            ttl: Duration::minutes(5),
        },
    )
    .unwrap();
    let confirmed = plan
        .confirm(
            plan.plan_id,
            &plan.plan_hash,
            &ConfirmationContext {
                config_revision: "config".into(),
                environment_revision: "environment".into(),
                version_snapshot: "version".into(),
                now,
            },
        )
        .unwrap();
    let paths = AppPaths::new(
        temporary.path().join("config"),
        temporary.path().join("data"),
        temporary.path().join("logs"),
    );
    let store = Arc::new(PersistenceStore::new(paths));
    let executor = CommandExecutor::new(Some(store.clone()), Arc::new(NoopEventSink), 4096);
    let run_id = RunId::new(uuid::Uuid::new_v4().to_string()).unwrap();

    let result = executor
        .execute(run_id.clone(), &confirmed, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(result.status, RunStatus::Succeeded);
    let log = store.read_run_log(&run_id).unwrap();
    assert!(log.iter().any(|entry| entry.message.contains("stdout-one")));
    assert!(log.iter().any(|entry| entry.message.contains("stderr-one")));
}
