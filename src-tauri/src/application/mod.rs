mod run_queue;

pub use run_queue::*;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::catalog::{Catalog, CatalogDiagnostic, CatalogError};
use crate::domain::{
    ComponentId, Installation, InstallationId, ProviderId, Run, RunId, RunStatus, RunSummary,
    StrategyId, StrategyKind, Tool, ToolId,
};
use crate::environment::{executable_fingerprint, EnvironmentResolver, ResolveRequest};
use crate::executor::{
    CommandExecutor, CommandSpec, ConfirmationContext, ExecutionError, ExecutionResult,
    PlanContext, PlanError, PlanHash, PlanId, UpdatePlan, UpdatePlanView,
};
use crate::persistence::{PersistenceStore, Revision, Settings, StoreError, Versioned};
use crate::providers::{
    ProviderError, ProviderRegistry, ProviderRegistryError, ProviderStatus, ProviderUpdateRequest,
    Recoverability, UpdateCheck,
};
use crate::version::{
    verify_updated_version, ComponentStatus, StatusReason, VerificationStatus, VersionValue,
};

#[derive(Debug, Clone)]
pub struct ApplicationServiceOptions {
    pub refresh_cache_ttl: Duration,
    pub plan_ttl: ChronoDuration,
}

impl Default for ApplicationServiceOptions {
    fn default() -> Self {
        Self {
            refresh_cache_ttl: Duration::from_secs(30),
            plan_ttl: ChronoDuration::minutes(5),
        }
    }
}

pub trait ApplicationEnvironment: Send + Sync {
    fn resolve_program(
        &self,
        name: &str,
        user_override: Option<&Path>,
        now: DateTime<Utc>,
    ) -> Result<PathBuf, String>;

    fn revision(&self) -> String;
}

pub struct ResolverApplicationEnvironment {
    resolver: EnvironmentResolver,
    tracked: StdMutex<BTreeMap<String, PathBuf>>,
}

impl ResolverApplicationEnvironment {
    pub fn new(resolver: EnvironmentResolver) -> Self {
        Self {
            resolver,
            tracked: StdMutex::new(BTreeMap::new()),
        }
    }
}

impl ApplicationEnvironment for ResolverApplicationEnvironment {
    fn resolve_program(
        &self,
        name: &str,
        user_override: Option<&Path>,
        now: DateTime<Utc>,
    ) -> Result<PathBuf, String> {
        let mut request = ResolveRequest::new(name);
        request.user_override = user_override.map(Path::to_path_buf);
        let outcome = self.resolver.resolve(&request, now);
        let resolved = outcome.resolved.ok_or_else(|| {
            outcome
                .unavailable_reason
                .unwrap_or_else(|| format!("{name} is unavailable"))
        })?;
        self.tracked
            .lock()
            .expect("application environment mutex is not poisoned")
            .insert(name.to_owned(), resolved.path.clone());
        Ok(resolved.path)
    }

    fn revision(&self) -> String {
        let tracked = self
            .tracked
            .lock()
            .expect("application environment mutex is not poisoned");
        let material: Vec<_> = tracked
            .iter()
            .map(|(name, path)| {
                (
                    name,
                    path,
                    executable_fingerprint(path).unwrap_or_else(|_| "missing".into()),
                )
            })
            .collect();
        blake3::hash(
            &serde_json::to_vec(&material).expect("environment revision material serializes"),
        )
        .to_hex()
        .to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum RefreshScope {
    All,
    Provider { provider_id: ProviderId },
    Tool { tool_id: ToolId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshRequest {
    pub scope: RefreshScope,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRefreshProgress {
    pub provider_id: ProviderId,
    pub completed: usize,
    pub total: usize,
}

impl RefreshRequest {
    pub const fn all() -> Self {
        Self {
            scope: RefreshScope::All,
            force: false,
        }
    }

    pub const fn force_all() -> Self {
        Self {
            scope: RefreshScope::All,
            force: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRefreshReport {
    pub provider_id: ProviderId,
    pub enabled: bool,
    pub status: Option<ProviderStatus>,
    pub errors: Vec<ProviderError>,
    pub refreshed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationSnapshot {
    pub tools: Vec<Tool>,
    pub providers: BTreeMap<ProviderId, ProviderRefreshReport>,
    pub catalog_diagnostics: Vec<CatalogDiagnostic>,
    pub catalog_errors: Vec<CatalogError>,
    pub config_revision: String,
    pub state_revision: String,
    pub environment_revision: String,
    pub refreshed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSelection {
    pub tool_id: ToolId,
    pub component_id: ComponentId,
    pub strategy_id: Option<StrategyId>,
    pub target_version: Option<VersionValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmUpdateRequest {
    pub plan_id: PlanId,
    pub plan_hash: PlanHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedUpdate {
    pub execution: ExecutionResult,
    pub snapshot: ApplicationSnapshot,
}

struct ServiceState {
    installations: BTreeMap<InstallationId, Installation>,
    snapshot: ApplicationSnapshot,
    provider_refreshed: BTreeMap<ProviderId, Instant>,
    plans: BTreeMap<PlanId, UpdatePlan>,
}

type ComponentKey = (InstallationId, ComponentId);
type RememberedComponent = (Option<VersionValue>, Option<VersionValue>, ComponentStatus);
type ComponentMemory = BTreeMap<ComponentKey, RememberedComponent>;

pub struct ApplicationService {
    registry: Arc<ProviderRegistry>,
    catalog: RwLock<Catalog>,
    store: Arc<PersistenceStore>,
    executor: CommandExecutor,
    environment: Arc<dyn ApplicationEnvironment>,
    options: ApplicationServiceOptions,
    operation: Mutex<()>,
    state: RwLock<ServiceState>,
}

impl ApplicationService {
    pub fn new(
        registry: Arc<ProviderRegistry>,
        catalog: Catalog,
        store: Arc<PersistenceStore>,
        executor: CommandExecutor,
        environment: Arc<dyn ApplicationEnvironment>,
        options: ApplicationServiceOptions,
    ) -> Result<Self, ApplicationError> {
        Self::with_catalog_errors(
            registry,
            catalog,
            Vec::new(),
            store,
            executor,
            environment,
            options,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_catalog_errors(
        registry: Arc<ProviderRegistry>,
        catalog: Catalog,
        catalog_errors: Vec<CatalogError>,
        store: Arc<PersistenceStore>,
        executor: CommandExecutor,
        environment: Arc<dyn ApplicationEnvironment>,
        options: ApplicationServiceOptions,
    ) -> Result<Self, ApplicationError> {
        let settings = store.load_settings()?;
        let cached = store.load_state()?;
        let snapshot = ApplicationSnapshot {
            tools: cached.value.tools,
            providers: registry
                .list()
                .into_iter()
                .map(|registration| {
                    let enabled = registration.enabled
                        && settings
                            .value
                            .provider_enabled
                            .get(&registration.id)
                            .copied()
                            .unwrap_or(true);
                    (
                        registration.id.clone(),
                        ProviderRefreshReport {
                            provider_id: registration.id,
                            enabled,
                            status: None,
                            errors: Vec::new(),
                            refreshed_at: cached.value.last_refresh_at.unwrap_or_else(Utc::now),
                        },
                    )
                })
                .collect(),
            catalog_diagnostics: Vec::new(),
            catalog_errors,
            config_revision: settings.revision.as_str().into(),
            state_revision: cached.revision.as_str().into(),
            environment_revision: environment.revision(),
            refreshed_at: cached.value.last_refresh_at,
        };
        Ok(Self {
            registry,
            catalog: RwLock::new(catalog),
            store,
            executor,
            environment,
            options,
            operation: Mutex::new(()),
            state: RwLock::new(ServiceState {
                installations: cached
                    .value
                    .installations
                    .into_iter()
                    .map(|installation| (installation.id.clone(), installation))
                    .collect(),
                snapshot,
                provider_refreshed: BTreeMap::new(),
                plans: BTreeMap::new(),
            }),
        })
    }

    pub async fn snapshot(&self) -> ApplicationSnapshot {
        self.state.read().await.snapshot.clone()
    }

    pub async fn installations(&self) -> Vec<Installation> {
        self.state
            .read()
            .await
            .installations
            .values()
            .cloned()
            .collect()
    }

    pub async fn refresh_tools(
        &self,
        request: RefreshRequest,
    ) -> Result<ApplicationSnapshot, ApplicationError> {
        self.refresh_tools_with_progress(request, |_| {}).await
    }

    pub async fn refresh_tools_with_progress<F>(
        &self,
        request: RefreshRequest,
        mut on_progress: F,
    ) -> Result<ApplicationSnapshot, ApplicationError>
    where
        F: FnMut(ProviderRefreshProgress) + Send,
    {
        let _operation = self.operation.lock().await;
        let settings = self.store.load_settings()?;
        let provider_ids = self.provider_ids_for_scope(&request.scope).await?;
        {
            let state = self.state.read().await;
            let cache_is_fresh = state.snapshot.refreshed_at.is_some()
                && provider_ids.iter().all(|provider_id| {
                    state
                        .provider_refreshed
                        .get(provider_id)
                        .is_some_and(|at| at.elapsed() < self.options.refresh_cache_ttl)
                });
            if !request.force && cache_is_fresh {
                return Ok(state.snapshot.clone());
            }
        }

        let now = Utc::now();
        let (mut installations, previous_snapshot, mut provider_reports) = {
            let state = self.state.read().await;
            (
                state.installations.clone(),
                state.snapshot.clone(),
                state.snapshot.providers.clone(),
            )
        };
        let previous_components = component_memory(&previous_snapshot.tools);
        let mut fresh_checks = BTreeMap::new();

        let provider_total = provider_ids.len();
        for (provider_index, provider_id) in provider_ids.iter().enumerate() {
            on_progress(ProviderRefreshProgress {
                provider_id: provider_id.clone(),
                completed: provider_index,
                total: provider_total,
            });
            let registration = self
                .registry
                .list()
                .into_iter()
                .find(|registration| &registration.id == provider_id)
                .ok_or_else(|| ApplicationError::UnknownProvider(provider_id.clone()))?;
            let enabled = registration.enabled
                && settings
                    .value
                    .provider_enabled
                    .get(provider_id)
                    .copied()
                    .unwrap_or(true);
            let mut report = ProviderRefreshReport {
                provider_id: provider_id.clone(),
                enabled,
                status: None,
                errors: Vec::new(),
                refreshed_at: now,
            };
            if !enabled {
                installations.retain(|_, installation| &installation.provider_id != provider_id);
                provider_reports.insert(provider_id.clone(), report);
                continue;
            }
            let provider = self
                .registry
                .get(provider_id)
                .ok_or_else(|| ApplicationError::UnknownProvider(provider_id.clone()))?;
            match provider.probe().await {
                Ok(status) => {
                    let available = status.available;
                    report.status = Some(status);
                    if !available {
                        provider_reports.insert(provider_id.clone(), report);
                        continue;
                    }
                }
                Err(error) => {
                    report.errors.push(error);
                    provider_reports.insert(provider_id.clone(), report);
                    continue;
                }
            }
            let scanned = match provider.scan().await {
                Ok(scanned) => scanned,
                Err(error) => {
                    report.errors.push(error);
                    provider_reports.insert(provider_id.clone(), report);
                    continue;
                }
            };
            installations.retain(|_, installation| &installation.provider_id != provider_id);
            installations.extend(
                scanned
                    .iter()
                    .cloned()
                    .map(|installation| (installation.id.clone(), installation)),
            );
            match provider.check_updates(&scanned).await {
                Ok(checks) => {
                    fresh_checks.extend(checks.into_iter().map(|check| {
                        (
                            (check.installation_id.clone(), check.component_id.clone()),
                            check,
                        )
                    }));
                }
                Err(error) => {
                    report.errors.push(error.clone());
                    fresh_checks.extend(scanned.iter().map(|installation| {
                        let check = UpdateCheck {
                            installation_id: installation.id.clone(),
                            component_id: ComponentId::new("core")
                                .expect("the embedded component ID is valid"),
                            installed_version: installation.installed_version.clone(),
                            latest_version: None,
                            status: ComponentStatus::CheckFailed {
                                reason: StatusReason::new(
                                    "provider_check_failed",
                                    error.summary.clone(),
                                    error.recoverability == Recoverability::Retryable,
                                ),
                            },
                        };
                        (
                            (check.installation_id.clone(), check.component_id.clone()),
                            check,
                        )
                    }));
                }
            }
            provider_reports.insert(provider_id.clone(), report);
        }

        let installation_values: Vec<_> = installations.values().cloned().collect();
        let catalog_snapshot = self.catalog.read().await.enrich(&installation_values);
        let mut tools = catalog_snapshot.tools;
        apply_component_memory(&mut tools, &previous_components);
        apply_checks(&mut tools, &fresh_checks);
        apply_strategy_preferences(&mut tools, &settings.value);
        tools.sort_by(|left, right| left.id.cmp(&right.id));

        let saved = self.persist_tools(&tools, Some(now), None, Some(&installations))?;
        let environment_revision = self.environment.revision();
        let snapshot = ApplicationSnapshot {
            tools,
            providers: provider_reports,
            catalog_diagnostics: catalog_snapshot.diagnostics,
            catalog_errors: previous_snapshot.catalog_errors,
            config_revision: settings.revision.as_str().into(),
            state_revision: saved.as_str().into(),
            environment_revision,
            refreshed_at: Some(now),
        };
        let mut state = self.state.write().await;
        state.installations = installations;
        for provider_id in provider_ids {
            state.provider_refreshed.insert(provider_id, Instant::now());
        }
        state.snapshot = snapshot.clone();
        Ok(snapshot)
    }

    pub async fn select_strategy(
        &self,
        tool_id: &ToolId,
        component_id: &ComponentId,
        strategy_id: &StrategyId,
    ) -> Result<ApplicationSnapshot, ApplicationError> {
        let _operation = self.operation.lock().await;
        {
            let state = self.state.read().await;
            let component = find_component(&state.snapshot.tools, tool_id, component_id)?;
            if !component
                .strategies
                .iter()
                .any(|strategy| &strategy.id == strategy_id && strategy.enabled)
            {
                return Err(ApplicationError::UnknownStrategy {
                    tool_id: tool_id.clone(),
                    component_id: component_id.clone(),
                    strategy_id: strategy_id.clone(),
                });
            }
        }
        let loaded = self.store.load_settings()?;
        let mut settings = loaded.value;
        settings
            .component_strategies
            .entry(tool_id.clone())
            .or_default()
            .insert(component_id.clone(), strategy_id.clone());
        let saved_settings = self.store.save_settings(&loaded.revision, &settings)?;

        let mut state = self.state.write().await;
        let component = find_component_mut(&mut state.snapshot.tools, tool_id, component_id)?;
        component.default_strategy = Some(strategy_id.clone());
        let saved_state = self.persist_tools(
            &state.snapshot.tools,
            state.snapshot.refreshed_at,
            None,
            None,
        )?;
        state.snapshot.config_revision = saved_settings.revision.as_str().into();
        state.snapshot.state_revision = saved_state.as_str().into();
        Ok(state.snapshot.clone())
    }

    pub async fn update_settings(
        &self,
        expected_revision: &Revision,
        settings: &Settings,
    ) -> Result<Versioned<Settings>, ApplicationError> {
        let _operation = self.operation.lock().await;
        let saved_settings = self.store.save_settings(expected_revision, settings)?;
        let mut state = self.state.write().await;
        apply_strategy_preferences(&mut state.snapshot.tools, &saved_settings.value);
        let saved_state = self.persist_tools(
            &state.snapshot.tools,
            state.snapshot.refreshed_at,
            None,
            None,
        )?;
        state.snapshot.config_revision = saved_settings.revision.as_str().into();
        state.snapshot.state_revision = saved_state.as_str().into();
        Ok(saved_settings)
    }

    pub async fn preview_updates(
        &self,
        selections: &[UpdateSelection],
    ) -> Result<Vec<UpdatePlanView>, ApplicationError> {
        let _operation = self.operation.lock().await;
        let settings = self.store.load_settings()?;
        let (tools, installations) = {
            let state = self.state.read().await;
            (state.snapshot.tools.clone(), state.installations.clone())
        };
        let mut unique = BTreeSet::new();
        let mut plans = Vec::with_capacity(selections.len());
        for selection in selections {
            if !unique.insert((selection.tool_id.clone(), selection.component_id.clone())) {
                return Err(ApplicationError::DuplicateSelection {
                    tool_id: selection.tool_id.clone(),
                    component_id: selection.component_id.clone(),
                });
            }
            let tool = tools
                .iter()
                .find(|tool| tool.id == selection.tool_id)
                .ok_or_else(|| ApplicationError::UnknownTool(selection.tool_id.clone()))?;
            let component = tool
                .components
                .iter()
                .find(|component| component.id == selection.component_id)
                .ok_or_else(|| ApplicationError::UnknownComponent {
                    tool_id: selection.tool_id.clone(),
                    component_id: selection.component_id.clone(),
                })?;
            let strategy_id = selection
                .strategy_id
                .as_ref()
                .or(component.default_strategy.as_ref())
                .ok_or_else(|| ApplicationError::NoDefaultStrategy {
                    tool_id: selection.tool_id.clone(),
                    component_id: selection.component_id.clone(),
                })?;
            let strategy = component
                .strategies
                .iter()
                .find(|strategy| &strategy.id == strategy_id && strategy.enabled)
                .ok_or_else(|| ApplicationError::UnknownStrategy {
                    tool_id: selection.tool_id.clone(),
                    component_id: selection.component_id.clone(),
                    strategy_id: strategy_id.clone(),
                })?;
            let installation_id = component
                .installation_id
                .as_ref()
                .or_else(|| tool.installation_ids.first())
                .ok_or_else(|| ApplicationError::ToolHasNoInstallation(tool.id.clone()))?;
            let installation = installations.get(installation_id).ok_or_else(|| {
                ApplicationError::InstallationUnavailable(installation_id.clone())
            })?;
            let mut command = match &strategy.kind {
                StrategyKind::ProviderDefault => {
                    let provider =
                        self.registry
                            .get(&installation.provider_id)
                            .ok_or_else(|| {
                                ApplicationError::UnknownProvider(installation.provider_id.clone())
                            })?;
                    let provider_plan = provider
                        .plan_update(ProviderUpdateRequest {
                            installation_id: installation_id.clone(),
                            component_id: component.id.clone(),
                            strategy_id: strategy.id.clone(),
                            target_version: selection.target_version.clone(),
                        })
                        .await?;
                    let mut command = CommandSpec::new(provider_plan.program, provider_plan.args);
                    command.network_required = true;
                    command
                }
                StrategyKind::Command { program, args } => {
                    let resolved = self.resolve_program(program, &settings.value)?;
                    CommandSpec::new(resolved, args.clone())
                }
                StrategyKind::PackageManager { manager, args } => {
                    let resolved = self.resolve_program(manager, &settings.value)?;
                    let mut command = CommandSpec::new(resolved, args.clone());
                    command.network_required = true;
                    command
                }
            };
            command.timeout_seconds = settings.value.default_timeout_seconds;
            let environment_revision =
                command_environment_revision(self.environment.as_ref(), &command.program);
            let plan = UpdatePlan::new(
                tool.id.clone(),
                installation_id.clone(),
                component.id.clone(),
                strategy.id.clone(),
                command,
                PlanContext {
                    config_revision: settings.revision.as_str().into(),
                    environment_revision,
                    version_snapshot: version_snapshot(tool, component),
                    now: Utc::now(),
                    ttl: self.options.plan_ttl,
                },
            )?;
            plans.push(plan);
        }
        let views = plans.iter().map(UpdatePlan::view).collect();
        let mut state = self.state.write().await;
        state
            .plans
            .extend(plans.into_iter().map(|plan| (plan.plan_id, plan)));
        Ok(views)
    }

    pub async fn confirm_update(
        &self,
        request: ConfirmUpdateRequest,
        cancellation: CancellationToken,
    ) -> Result<ConfirmedUpdate, ApplicationError> {
        let run_id =
            RunId::new(uuid::Uuid::new_v4().to_string()).expect("a UUID is a valid run ID");
        self.confirm_update_with_run_id(request, run_id, cancellation)
            .await
    }

    pub async fn confirm_update_with_run_id(
        &self,
        request: ConfirmUpdateRequest,
        run_id: RunId,
        cancellation: CancellationToken,
    ) -> Result<ConfirmedUpdate, ApplicationError> {
        let _operation = self.operation.lock().await;
        let plan = self
            .state
            .read()
            .await
            .plans
            .get(&request.plan_id)
            .cloned()
            .ok_or(ApplicationError::UnknownPlan(request.plan_id))?;
        let settings = self.store.load_settings()?;
        let (tool, component) = {
            let state = self.state.read().await;
            let tool = state
                .snapshot
                .tools
                .iter()
                .find(|tool| tool.id == plan.tool_id)
                .ok_or_else(|| ApplicationError::UnknownTool(plan.tool_id.clone()))?;
            let component = tool
                .components
                .iter()
                .find(|component| component.id == plan.component_id)
                .ok_or_else(|| ApplicationError::UnknownComponent {
                    tool_id: plan.tool_id.clone(),
                    component_id: plan.component_id.clone(),
                })?;
            (tool.clone(), component.clone())
        };
        let confirmed = plan.confirm(
            request.plan_id,
            &request.plan_hash,
            &ConfirmationContext {
                config_revision: settings.revision.as_str().into(),
                environment_revision: command_environment_revision(
                    self.environment.as_ref(),
                    &plan.command.program,
                ),
                version_snapshot: version_snapshot(&tool, &component),
                now: Utc::now(),
            },
        )?;

        {
            let mut state = self.state.write().await;
            find_component_mut(&mut state.snapshot.tools, &plan.tool_id, &plan.component_id)?
                .status = ComponentStatus::Updating;
        }

        let mut execution = match self
            .executor
            .execute(run_id.clone(), &confirmed, cancellation)
            .await
        {
            Ok(execution) => execution,
            Err(error) => {
                let mut state = self.state.write().await;
                find_component_mut(&mut state.snapshot.tools, &plan.tool_id, &plan.component_id)?
                    .status = ComponentStatus::UpdateFailed {
                    reason: StatusReason::new(
                        "execution_error",
                        error.summary.to_string(),
                        error.retryable,
                    ),
                };
                state.plans.remove(&request.plan_id);
                let saved = self.persist_tools(
                    &state.snapshot.tools,
                    state.snapshot.refreshed_at,
                    None,
                    None,
                )?;
                state.snapshot.state_revision = saved.as_str().into();
                return Err(ApplicationError::Execution(error));
            }
        };

        let previous_version = component.installed_version.clone();
        let (component_status, detected_version) = match execution.status {
            RunStatus::Succeeded => {
                let (verification, detected) = self
                    .post_check(&plan.installation_id, previous_version.as_ref())
                    .await;
                if !matches!(verification, VerificationStatus::Verified) {
                    execution.status = RunStatus::Partial;
                }
                execution.verification = Some(verification.clone());
                (ComponentStatus::UpdateSucceeded { verification }, detected)
            }
            RunStatus::Partial => {
                let verification = execution.verification.clone().unwrap_or_else(|| {
                    VerificationStatus::VerificationFailed {
                        reason: StatusReason::new(
                            "post_check_failed",
                            "the update completed but its post-check failed",
                            true,
                        ),
                    }
                });
                (ComponentStatus::UpdateSucceeded { verification }, None)
            }
            RunStatus::Cancelled => (ComponentStatus::Cancelled, None),
            RunStatus::Failed
            | RunStatus::TimedOut
            | RunStatus::Interrupted
            | RunStatus::Queued
            | RunStatus::Running => (
                ComponentStatus::UpdateFailed {
                    reason: StatusReason::new(
                        "update_command_failed",
                        format!("update finished with status {:?}", execution.status),
                        matches!(
                            execution.status,
                            RunStatus::TimedOut | RunStatus::Interrupted
                        ),
                    ),
                },
                None,
            ),
        };
        let run = Run {
            id: run_id,
            tool_id: Some(plan.tool_id.clone()),
            subject: Default::default(),
            operation: None,
            resource_names: vec![],
            component_ids: vec![plan.component_id.clone()],
            status: execution.status,
            created_at: execution.started_at,
            started_at: Some(execution.started_at),
            finished_at: Some(execution.finished_at),
            batch_id: None,
            retry_of: None,
            log_path: Some(
                self.store
                    .paths()
                    .runs_dir()
                    .join(format!("{}.jsonl", execution.run_id)),
            ),
            summary: Some(RunSummary {
                exit_code: execution.exit_code,
                output_tail: execution.output_tail.clone(),
                verification: execution.verification.clone(),
                error: None,
            }),
        };

        let mut state = self.state.write().await;
        let component =
            find_component_mut(&mut state.snapshot.tools, &plan.tool_id, &plan.component_id)?;
        component.status = component_status;
        if detected_version.is_some() {
            component.installed_version.clone_from(&detected_version);
            if let Some(installation) = state.installations.get_mut(&plan.installation_id) {
                installation.installed_version = detected_version;
            }
        }
        state.plans.remove(&request.plan_id);
        let saved = self.persist_tools(
            &state.snapshot.tools,
            state.snapshot.refreshed_at,
            Some(run),
            Some(&state.installations),
        )?;
        state.snapshot.state_revision = saved.as_str().into();
        let snapshot = state.snapshot.clone();
        Ok(ConfirmedUpdate {
            execution,
            snapshot,
        })
    }

    pub async fn reload_catalog(
        &self,
        catalog: Catalog,
        catalog_errors: Vec<CatalogError>,
    ) -> Result<ApplicationSnapshot, ApplicationError> {
        let _operation = self.operation.lock().await;
        let settings = self.store.load_settings()?;
        let (installations, previous_snapshot) = {
            let state = self.state.read().await;
            (
                state.installations.values().cloned().collect::<Vec<_>>(),
                state.snapshot.clone(),
            )
        };
        let memory = component_memory(&previous_snapshot.tools);
        let catalog_snapshot = catalog.enrich(&installations);
        let mut tools = catalog_snapshot.tools;
        apply_component_memory(&mut tools, &memory);
        apply_strategy_preferences(&mut tools, &settings.value);
        tools.sort_by(|left, right| left.id.cmp(&right.id));
        let saved = self.persist_tools(&tools, previous_snapshot.refreshed_at, None, None)?;

        *self.catalog.write().await = catalog;
        let mut state = self.state.write().await;
        state.snapshot.tools = tools;
        state.snapshot.catalog_diagnostics = catalog_snapshot.diagnostics;
        state.snapshot.catalog_errors = catalog_errors;
        state.snapshot.config_revision = settings.revision.as_str().into();
        state.snapshot.state_revision = saved.as_str().into();
        state.provider_refreshed.clear();
        Ok(state.snapshot.clone())
    }

    async fn provider_ids_for_scope(
        &self,
        scope: &RefreshScope,
    ) -> Result<Vec<ProviderId>, ApplicationError> {
        match scope {
            RefreshScope::All => Ok(self
                .registry
                .list()
                .into_iter()
                .map(|registration| registration.id)
                .collect()),
            RefreshScope::Provider { provider_id } => {
                if self.registry.get(provider_id).is_none() {
                    return Err(ApplicationError::UnknownProvider(provider_id.clone()));
                }
                Ok(vec![provider_id.clone()])
            }
            RefreshScope::Tool { tool_id } => {
                let state = self.state.read().await;
                let tool = state
                    .snapshot
                    .tools
                    .iter()
                    .find(|tool| &tool.id == tool_id)
                    .ok_or_else(|| ApplicationError::UnknownTool(tool_id.clone()))?;
                let providers: BTreeSet<_> = tool
                    .installation_ids
                    .iter()
                    .filter_map(|installation_id| state.installations.get(installation_id))
                    .map(|installation| installation.provider_id.clone())
                    .collect();
                if providers.is_empty() {
                    return Err(ApplicationError::ToolHasNoInstallation(tool_id.clone()));
                }
                Ok(providers.into_iter().collect())
            }
        }
    }

    fn resolve_program(
        &self,
        name: &str,
        settings: &Settings,
    ) -> Result<PathBuf, ApplicationError> {
        self.environment
            .resolve_program(
                name,
                settings
                    .executable_overrides
                    .get(name)
                    .map(PathBuf::as_path),
                Utc::now(),
            )
            .map_err(|summary| ApplicationError::ProgramUnavailable {
                program: name.into(),
                summary,
            })
    }

    async fn post_check(
        &self,
        installation_id: &InstallationId,
        previous_version: Option<&VersionValue>,
    ) -> (VerificationStatus, Option<VersionValue>) {
        let provider_id = {
            let state = self.state.read().await;
            state
                .installations
                .get(installation_id)
                .map(|installation| installation.provider_id.clone())
        };
        let Some(provider_id) = provider_id else {
            return (
                VerificationStatus::VerificationFailed {
                    reason: StatusReason::new(
                        "post_check_installation_missing",
                        "the installation disappeared before post-check",
                        true,
                    ),
                },
                None,
            );
        };
        let Some(provider) = self.registry.get(&provider_id) else {
            return (
                VerificationStatus::VerificationFailed {
                    reason: StatusReason::new(
                        "post_check_provider_missing",
                        "the provider is unavailable for post-check",
                        true,
                    ),
                },
                None,
            );
        };
        match provider.scan().await {
            Ok(installations) => {
                let detected = installations
                    .into_iter()
                    .find(|installation| &installation.id == installation_id)
                    .and_then(|installation| installation.installed_version);
                let verification = verify_updated_version(previous_version, Ok(detected.as_ref()));
                (verification, detected)
            }
            Err(error) => (
                VerificationStatus::VerificationFailed {
                    reason: StatusReason::new(
                        "post_check_provider_failed",
                        error.summary,
                        error.recoverability == Recoverability::Retryable,
                    ),
                },
                None,
            ),
        }
    }

    fn persist_tools(
        &self,
        tools: &[Tool],
        refreshed_at: Option<DateTime<Utc>>,
        run: Option<Run>,
        installations: Option<&BTreeMap<InstallationId, Installation>>,
    ) -> Result<Revision, ApplicationError> {
        Ok(self.store.update_cached_state(|cached| {
            cached.tools = tools.to_vec();
            if let Some(installations) = installations {
                cached.installations = installations.values().cloned().collect();
            }
            cached.last_refresh_at = refreshed_at;
            if let Some(run) = run {
                cached.runs.push(run);
            }
        })?)
    }
}

fn component_memory(tools: &[Tool]) -> ComponentMemory {
    tools
        .iter()
        .flat_map(|tool| &tool.components)
        .filter_map(|component| {
            component.installation_id.as_ref().map(|installation_id| {
                (
                    (installation_id.clone(), component.id.clone()),
                    (
                        component.installed_version.clone(),
                        component.latest_version.clone(),
                        component.status.clone(),
                    ),
                )
            })
        })
        .collect()
}

fn apply_component_memory(tools: &mut [Tool], memory: &ComponentMemory) {
    for component in tools.iter_mut().flat_map(|tool| &mut tool.components) {
        let Some(installation_id) = &component.installation_id else {
            continue;
        };
        if let Some((installed, latest, status)) =
            memory.get(&(installation_id.clone(), component.id.clone()))
        {
            component.installed_version.clone_from(installed);
            component.latest_version.clone_from(latest);
            component.status.clone_from(status);
        }
    }
}

fn apply_checks(tools: &mut [Tool], checks: &BTreeMap<(InstallationId, ComponentId), UpdateCheck>) {
    for component in tools.iter_mut().flat_map(|tool| &mut tool.components) {
        let Some(installation_id) = &component.installation_id else {
            continue;
        };
        if let Some(check) = checks.get(&(installation_id.clone(), component.id.clone())) {
            component
                .installed_version
                .clone_from(&check.installed_version);
            component.latest_version.clone_from(&check.latest_version);
            component.status.clone_from(&check.status);
        }
    }
}

fn apply_strategy_preferences(tools: &mut [Tool], settings: &Settings) {
    for tool in tools {
        let Some(preferences) = settings.component_strategies.get(&tool.id) else {
            continue;
        };
        for component in &mut tool.components {
            let Some(strategy_id) = preferences.get(&component.id) else {
                continue;
            };
            if component
                .strategies
                .iter()
                .any(|strategy| &strategy.id == strategy_id && strategy.enabled)
            {
                component.default_strategy = Some(strategy_id.clone());
            }
        }
    }
}

fn find_component<'a>(
    tools: &'a [Tool],
    tool_id: &ToolId,
    component_id: &ComponentId,
) -> Result<&'a crate::domain::Component, ApplicationError> {
    let tool = tools
        .iter()
        .find(|tool| &tool.id == tool_id)
        .ok_or_else(|| ApplicationError::UnknownTool(tool_id.clone()))?;
    tool.components
        .iter()
        .find(|component| &component.id == component_id)
        .ok_or_else(|| ApplicationError::UnknownComponent {
            tool_id: tool_id.clone(),
            component_id: component_id.clone(),
        })
}

fn find_component_mut<'a>(
    tools: &'a mut [Tool],
    tool_id: &ToolId,
    component_id: &ComponentId,
) -> Result<&'a mut crate::domain::Component, ApplicationError> {
    let tool = tools
        .iter_mut()
        .find(|tool| &tool.id == tool_id)
        .ok_or_else(|| ApplicationError::UnknownTool(tool_id.clone()))?;
    tool.components
        .iter_mut()
        .find(|component| &component.id == component_id)
        .ok_or_else(|| ApplicationError::UnknownComponent {
            tool_id: tool_id.clone(),
            component_id: component_id.clone(),
        })
}

fn version_snapshot(tool: &Tool, component: &crate::domain::Component) -> String {
    let material = (
        &tool.id,
        &component.id,
        &component.installed_version,
        &component.latest_version,
        &component.status,
    );
    blake3::hash(&serde_json::to_vec(&material).expect("version snapshot material serializes"))
        .to_hex()
        .to_string()
}

fn command_environment_revision(
    environment: &dyn ApplicationEnvironment,
    program: &Path,
) -> String {
    let material = (
        environment.revision(),
        program,
        executable_fingerprint(program).unwrap_or_else(|_| "missing".into()),
    );
    blake3::hash(&serde_json::to_vec(&material).expect("command environment material serializes"))
        .to_hex()
        .to_string()
}

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Registry(#[from] ProviderRegistryError),
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Execution(#[from] ExecutionError),
    #[error("provider {0} is unknown")]
    UnknownProvider(ProviderId),
    #[error("tool {0} is unknown")]
    UnknownTool(ToolId),
    #[error("component {component_id} is unknown for tool {tool_id}")]
    UnknownComponent {
        tool_id: ToolId,
        component_id: ComponentId,
    },
    #[error("strategy {strategy_id} is unknown for {tool_id}/{component_id}")]
    UnknownStrategy {
        tool_id: ToolId,
        component_id: ComponentId,
        strategy_id: StrategyId,
    },
    #[error("component {component_id} on {tool_id} has no default strategy")]
    NoDefaultStrategy {
        tool_id: ToolId,
        component_id: ComponentId,
    },
    #[error("tool {0} has no available installation")]
    ToolHasNoInstallation(ToolId),
    #[error("installation {0} is unavailable")]
    InstallationUnavailable(InstallationId),
    #[error("program {program} is unavailable: {summary}")]
    ProgramUnavailable { program: String, summary: String },
    #[error("duplicate update selection for {tool_id}/{component_id}")]
    DuplicateSelection {
        tool_id: ToolId,
        component_id: ComponentId,
    },
    #[error("plan {0} is unknown or already consumed")]
    UnknownPlan(PlanId),
}
