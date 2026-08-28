use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::application::{
    ApplicationError, ApplicationService, ApplicationServiceOptions, ConfirmUpdateRequest,
    ProviderRefreshProgress, RefreshRequest, RefreshScope, ResolverApplicationEnvironment,
    UpdateSelection,
};
use crate::catalog::{load_catalog_with_built_ins, CatalogError, CatalogLayer, LoadedManifest};
use crate::domain::{ComponentId, ProviderId, RunId, StrategyId, ToolId};
use crate::environment::{EnvironmentResolver, ResolveRequest};
use crate::executor::{CommandExecutor, ExecutionErrorKind, PlanError, PlanHash, PlanId};
use crate::persistence::{
    AppPaths, PersistenceStore, Revision, Settings, StoreError, StoreErrorKind,
};
use crate::providers::{
    HomebrewProvider, NpmGlobalProvider, ProviderRegistry, ProviderRegistryError, Recoverability,
};
use crate::version::VersionValue;

use super::dto::*;
use super::events::{
    ApiEvent, ApiEventEmitter, BufferedExecutionEventSink, RefreshPhaseDto,
    RefreshProgressEventDto, TauriApiEventEmitter, ToolStateEventDto,
};

pub struct ApiService {
    application: Arc<ApplicationService>,
    store: Arc<PersistenceStore>,
    emitter: Arc<dyn ApiEventEmitter>,
    active_runs: Mutex<BTreeMap<RunId, CancellationToken>>,
    event_sequence: AtomicU64,
}

impl ApiService {
    pub fn new(
        application: Arc<ApplicationService>,
        store: Arc<PersistenceStore>,
        emitter: Arc<dyn ApiEventEmitter>,
    ) -> Self {
        Self {
            application,
            store,
            emitter,
            active_runs: Mutex::new(BTreeMap::new()),
            event_sequence: AtomicU64::new(0),
        }
    }

    pub fn production(
        paths: AppPaths,
        emitter: Arc<dyn ApiEventEmitter>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let store = Arc::new(PersistenceStore::new(paths));
        let settings = store.load_settings()?;
        let provider_resolver = EnvironmentResolver::from_current_process()?;
        let npm_path = resolve_provider_program(
            &provider_resolver,
            "npm",
            settings.value.executable_overrides.get("npm"),
            store.paths().data_dir.join("unavailable/npm"),
        );
        let brew_path = resolve_provider_program(
            &provider_resolver,
            "brew",
            settings.value.executable_overrides.get("brew"),
            store.paths().data_dir.join("unavailable/brew"),
        );
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(NpmGlobalProvider::new(npm_path)))?;
        registry.register(Arc::new(HomebrewProvider::new(brew_path)))?;
        let catalog = load_catalog_with_built_ins(store.paths().manifests_dir());
        let event_sink = Arc::new(BufferedExecutionEventSink::new(
            Arc::clone(&emitter),
            32,
            Duration::from_millis(50),
        ));
        let executor = CommandExecutor::new(Some(Arc::clone(&store)), event_sink, 64 * 1024);
        let environment = Arc::new(ResolverApplicationEnvironment::new(
            EnvironmentResolver::from_current_process()?,
        ));
        let application = Arc::new(ApplicationService::with_catalog_errors(
            Arc::new(registry),
            catalog.catalog,
            catalog.errors,
            Arc::clone(&store),
            executor,
            environment,
            ApplicationServiceOptions::default(),
        )?);
        Ok(Self::new(application, store, emitter))
    }

    pub async fn snapshot(&self) -> Result<SnapshotDto, ApiErrorDto> {
        self.snapshot_from(self.application.snapshot().await).await
    }

    pub async fn refresh(&self, request: RefreshRequestDto) -> Result<SnapshotDto, ApiErrorDto> {
        let request_id = uuid::Uuid::new_v4().to_string();
        self.emit_refresh(
            &request_id,
            RefreshPhaseDto::Started,
            "refresh started",
            None,
        );
        let request = match refresh_request(request) {
            Ok(request) => request,
            Err(error) => {
                self.emit_refresh(&request_id, RefreshPhaseDto::Failed, &error.message, None);
                return Err(error);
            }
        };
        let mut completed = 0;
        let mut total = 0;
        let result = self
            .application
            .refresh_tools_with_progress(request, |progress| {
                completed = progress.completed;
                total = progress.total;
                self.emit_refresh(
                    &request_id,
                    RefreshPhaseDto::Progress,
                    &format!("checking {}", progress.provider_id),
                    Some(&progress),
                );
            })
            .await;
        match result {
            Ok(snapshot) => {
                self.emit_refresh_counts(
                    &request_id,
                    RefreshPhaseDto::Completed,
                    "refresh completed",
                    total,
                    total,
                );
                let dto = self.snapshot_from(snapshot).await?;
                self.emit_tool_state(&dto);
                Ok(dto)
            }
            Err(error) => {
                self.emit_refresh_counts(
                    &request_id,
                    RefreshPhaseDto::Failed,
                    &error.to_string(),
                    completed,
                    total,
                );
                Err(application_error(error))
            }
        }
    }

    pub async fn preview(
        &self,
        request: PreviewRequestDto,
    ) -> Result<Vec<UpdatePlanDto>, ApiErrorDto> {
        if request.selections.is_empty() {
            return Err(ApiErrorDto::new(
                ApiErrorCode::InvalidRequest,
                "at least one update selection is required",
                false,
            ));
        }
        let selections = request
            .selections
            .into_iter()
            .map(update_selection)
            .collect::<Result<Vec<_>, _>>()?;
        self.application
            .preview_updates(&selections)
            .await
            .map(|plans| plans.iter().map(UpdatePlanDto::from).collect())
            .map_err(application_error)
    }

    pub async fn confirm(
        &self,
        request: ConfirmRequestDto,
    ) -> Result<ConfirmResponseDto, ApiErrorDto> {
        let plan_id = PlanId::from_str(&request.plan_id)
            .map_err(|_| invalid_field("planId", "plan ID must be a UUID"))?;
        let plan_hash = PlanHash::from_str(&request.plan_hash)
            .map_err(|error| invalid_field("planHash", error.to_string()))?;
        let run_id =
            RunId::new(uuid::Uuid::new_v4().to_string()).expect("a UUID is a valid run ID");
        let cancellation = CancellationToken::new();
        self.active_runs
            .lock()
            .await
            .insert(run_id.clone(), cancellation.clone());
        let result = self
            .application
            .confirm_update_with_run_id(
                ConfirmUpdateRequest { plan_id, plan_hash },
                run_id.clone(),
                cancellation,
            )
            .await;
        self.active_runs.lock().await.remove(&run_id);
        let confirmed = result.map_err(application_error)?;
        let snapshot = self.snapshot_from(confirmed.snapshot).await?;
        self.emit_tool_state(&snapshot);
        Ok(ConfirmResponseDto {
            execution: ExecutionResultDto::from(&confirmed.execution),
            snapshot,
        })
    }

    pub async fn cancel(
        &self,
        request: CancelRequestDto,
    ) -> Result<CancelResponseDto, ApiErrorDto> {
        let run_id = RunId::new(request.run_id.clone())
            .map_err(|error| invalid_field("runId", error.to_string()))?;
        if let Some(cancellation) = self.active_runs.lock().await.get(&run_id).cloned() {
            cancellation.cancel();
            return Ok(CancelResponseDto {
                run_id: run_id.to_string(),
                disposition: CancelDispositionDto::CancellationRequested,
            });
        }
        let history = self.store.load_state().map_err(store_error)?.value.runs;
        if history
            .iter()
            .any(|run| run.id == run_id && run.status.is_terminal())
        {
            return Ok(CancelResponseDto {
                run_id: run_id.to_string(),
                disposition: CancelDispositionDto::AlreadyFinished,
            });
        }
        Err(ApiErrorDto::new(
            ApiErrorCode::NotFound,
            format!("run {run_id} was not found"),
            false,
        ))
    }

    pub fn run_history(&self) -> Result<RunHistoryDto, ApiErrorDto> {
        let mut runs = self.store.load_state().map_err(store_error)?.value.runs;
        runs.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(RunHistoryDto {
            runs: runs.iter().map(RunDto::from).collect(),
        })
    }

    pub async fn run_log(&self, request: RunLogRequestDto) -> Result<RunLogDto, ApiErrorDto> {
        let run_id = RunId::new(request.run_id)
            .map_err(|error| invalid_field("runId", error.to_string()))?;
        let known = self
            .store
            .load_state()
            .map_err(store_error)?
            .value
            .runs
            .iter()
            .any(|run| run.id == run_id)
            || self.active_runs.lock().await.contains_key(&run_id);
        if !known {
            return Err(ApiErrorDto::new(
                ApiErrorCode::NotFound,
                format!("run {run_id} was not found"),
                false,
            ));
        }
        let path = self
            .store
            .paths()
            .runs_dir()
            .join(format!("{run_id}.jsonl"));
        let entries = if path.exists() {
            self.store.read_run_log(&run_id).map_err(store_error)?
        } else {
            Vec::new()
        };
        Ok(RunLogDto {
            run_id: run_id.to_string(),
            entries: entries.iter().map(RunLogEntryDto::from).collect(),
        })
    }

    pub fn settings(&self) -> Result<SettingsDocumentDto, ApiErrorDto> {
        let loaded = self.store.load_settings().map_err(store_error)?;
        Ok(SettingsDocumentDto {
            revision: loaded.revision.as_str().into(),
            settings: SettingsValueDto::from(&loaded.value),
        })
    }

    pub async fn save_settings(
        &self,
        request: SaveSettingsRequestDto,
    ) -> Result<SettingsDocumentDto, ApiErrorDto> {
        let expected = Revision::from_str(&request.expected_revision)
            .map_err(|error| invalid_field("expectedRevision", error.to_string()))?;
        let settings = settings_from_dto(request.settings)?;
        let saved = self
            .application
            .update_settings(&expected, &settings)
            .await
            .map_err(application_error)?;
        let snapshot = self.snapshot().await?;
        self.emit_tool_state(&snapshot);
        Ok(SettingsDocumentDto {
            revision: saved.revision.as_str().into(),
            settings: SettingsValueDto::from(&saved.value),
        })
    }

    pub fn validate_manifest(
        &self,
        request: ManifestInputDto,
    ) -> Result<ManifestValidationDto, ApiErrorDto> {
        self.validate_manifest_contents(&request.file_name, &request.contents)
    }

    pub fn read_manifest(
        &self,
        request: ManifestReadRequestDto,
    ) -> Result<ManifestDocumentDto, ApiErrorDto> {
        let loaded = self
            .store
            .load_user_manifest(&request.file_name)
            .map_err(store_error)?;
        Ok(ManifestDocumentDto {
            file_name: request.file_name,
            revision: loaded
                .as_ref()
                .map(|document| document.revision.as_str().to_owned()),
            contents: loaded.map(|document| document.value),
        })
    }

    pub async fn save_manifest(
        &self,
        request: SaveManifestRequestDto,
    ) -> Result<SavedManifestDto, ApiErrorDto> {
        let validation = self.validate_manifest_contents(&request.file_name, &request.contents)?;
        let expected = request
            .expected_revision
            .as_deref()
            .map(Revision::from_str)
            .transpose()
            .map_err(|error| invalid_field("expectedRevision", error.to_string()))?;
        let saved = self
            .store
            .save_user_manifest(&request.file_name, expected.as_ref(), &request.contents)
            .map_err(store_error)?;
        let report = load_catalog_with_built_ins(self.store.paths().manifests_dir());
        let snapshot = self
            .application
            .reload_catalog(report.catalog, report.errors)
            .await
            .map_err(application_error)?;
        let snapshot = self.snapshot_from(snapshot).await?;
        self.emit_tool_state(&snapshot);
        Ok(SavedManifestDto {
            revision: saved.revision.as_str().into(),
            validation,
            snapshot,
        })
    }

    fn validate_manifest_contents(
        &self,
        file_name: &str,
        contents: &str,
    ) -> Result<ManifestValidationDto, ApiErrorDto> {
        let source = self
            .store
            .user_manifest_path(file_name)
            .map_err(store_error)?;
        let loaded =
            LoadedManifest::parse(source, CatalogLayer::User, contents).map_err(catalog_error)?;
        let normalized_toml = loaded.to_toml().map_err(catalog_error)?;
        Ok(ManifestValidationDto {
            file_name: file_name.into(),
            manifest_id: loaded.manifest.id.to_string(),
            normalized_toml,
        })
    }

    async fn snapshot_from(
        &self,
        snapshot: crate::application::ApplicationSnapshot,
    ) -> Result<SnapshotDto, ApiErrorDto> {
        let installations = self.application.installations().await;
        Ok(SnapshotDto::from_domain(&snapshot, &installations))
    }

    fn emit_refresh(
        &self,
        request_id: &str,
        phase: RefreshPhaseDto,
        message: &str,
        progress: Option<&ProviderRefreshProgress>,
    ) {
        let _ = self
            .emitter
            .emit(ApiEvent::RefreshProgress(RefreshProgressEventDto {
                request_id: request_id.into(),
                sequence: self.next_event_sequence(),
                phase,
                message: message.into(),
                provider_id: progress.map(|item| item.provider_id.to_string()),
                completed: progress.map(|item| item.completed as u32),
                total: progress.map(|item| item.total as u32),
            }));
    }

    fn emit_refresh_counts(
        &self,
        request_id: &str,
        phase: RefreshPhaseDto,
        message: &str,
        completed: usize,
        total: usize,
    ) {
        let _ = self
            .emitter
            .emit(ApiEvent::RefreshProgress(RefreshProgressEventDto {
                request_id: request_id.into(),
                sequence: self.next_event_sequence(),
                phase,
                message: message.into(),
                provider_id: None,
                completed: Some(completed as u32),
                total: Some(total as u32),
            }));
    }

    fn emit_tool_state(&self, snapshot: &SnapshotDto) {
        let _ = self.emitter.emit(ApiEvent::ToolState(ToolStateEventDto {
            sequence: self.next_event_sequence(),
            state_revision: snapshot.state_revision.clone(),
            tool_ids: snapshot.tools.iter().map(|tool| tool.id.clone()).collect(),
        }));
    }

    fn next_event_sequence(&self) -> String {
        self.event_sequence
            .fetch_add(1, Ordering::Relaxed)
            .to_string()
    }
}

pub struct ApiState {
    service: Arc<ApiService>,
}

impl ApiState {
    pub fn new(service: Arc<ApiService>) -> Self {
        Self { service }
    }

    pub fn service(&self) -> &ApiService {
        &self.service
    }

    pub fn production(
        handle: &tauri::AppHandle<tauri::Wry>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let emitter: Arc<dyn ApiEventEmitter> = Arc::new(TauriApiEventEmitter::new(handle.clone()));
        let paths = AppPaths::from_tauri(handle.path())?;
        Ok(Self::new(Arc::new(ApiService::production(paths, emitter)?)))
    }
}

fn resolve_provider_program(
    resolver: &EnvironmentResolver,
    name: &str,
    user_override: Option<&PathBuf>,
    unavailable: PathBuf,
) -> PathBuf {
    let mut request = ResolveRequest::new(name);
    request.user_override = user_override.cloned();
    resolver
        .resolve(&request, chrono::Utc::now())
        .resolved
        .map_or(unavailable, |resolved| resolved.path)
}

fn refresh_request(request: RefreshRequestDto) -> Result<RefreshRequest, ApiErrorDto> {
    let scope = match request.scope {
        RefreshScopeDto::All => RefreshScope::All,
        RefreshScopeDto::Provider { provider_id } => RefreshScope::Provider {
            provider_id: ProviderId::new(provider_id)
                .map_err(|error| invalid_field("providerId", error.to_string()))?,
        },
        RefreshScopeDto::Tool { tool_id } => RefreshScope::Tool {
            tool_id: ToolId::new(tool_id)
                .map_err(|error| invalid_field("toolId", error.to_string()))?,
        },
    };
    Ok(RefreshRequest {
        scope,
        force: request.force,
    })
}

fn update_selection(selection: UpdateSelectionDto) -> Result<UpdateSelection, ApiErrorDto> {
    Ok(UpdateSelection {
        tool_id: ToolId::new(selection.tool_id)
            .map_err(|error| invalid_field("toolId", error.to_string()))?,
        component_id: ComponentId::new(selection.component_id)
            .map_err(|error| invalid_field("componentId", error.to_string()))?,
        strategy_id: selection
            .strategy_id
            .map(StrategyId::new)
            .transpose()
            .map_err(|error| invalid_field("strategyId", error.to_string()))?,
        target_version: selection.target_version.map(VersionValue::new),
    })
}

fn settings_from_dto(value: SettingsValueDto) -> Result<Settings, ApiErrorDto> {
    let timeout = parse_u64("defaultTimeoutSeconds", &value.default_timeout_seconds)?;
    if timeout == 0 {
        return Err(invalid_field(
            "defaultTimeoutSeconds",
            "timeout must be greater than zero",
        ));
    }
    let provider_enabled = value
        .provider_enabled
        .into_iter()
        .map(|(id, enabled)| {
            ProviderId::new(id)
                .map(|id| (id, enabled))
                .map_err(|error| invalid_field("providerEnabled", error.to_string()))
        })
        .collect::<Result<_, _>>()?;
    let executable_overrides = value
        .executable_overrides
        .into_iter()
        .map(|(name, path)| {
            let path = PathBuf::from(path);
            if name.trim().is_empty() || name.chars().any(char::is_control) {
                return Err(invalid_field(
                    "executableOverrides",
                    "executable names must be non-empty and contain no control characters",
                ));
            }
            if !path.is_absolute() {
                return Err(invalid_field(
                    "executableOverrides",
                    "executable override paths must be absolute",
                ));
            }
            Ok((name, path))
        })
        .collect::<Result<_, _>>()?;
    let component_strategies = value
        .component_strategies
        .into_iter()
        .map(|(tool_id, components)| {
            let tool_id = ToolId::new(tool_id)
                .map_err(|error| invalid_field("componentStrategies", error.to_string()))?;
            let components = components
                .into_iter()
                .map(|(component_id, strategy_id)| {
                    Ok((
                        ComponentId::new(component_id).map_err(|error| {
                            invalid_field("componentStrategies", error.to_string())
                        })?,
                        StrategyId::new(strategy_id).map_err(|error| {
                            invalid_field("componentStrategies", error.to_string())
                        })?,
                    ))
                })
                .collect::<Result<_, ApiErrorDto>>()?;
            Ok((tool_id, components))
        })
        .collect::<Result<_, ApiErrorDto>>()?;
    Ok(Settings {
        provider_enabled,
        default_timeout_seconds: timeout,
        log_retention: value.log_retention.to_domain()?,
        executable_overrides,
        component_strategies,
    })
}

fn invalid_field(field: &str, message: impl Into<String>) -> ApiErrorDto {
    ApiErrorDto::new(ApiErrorCode::InvalidRequest, message, false).with_detail("field", field)
}

fn application_error(error: ApplicationError) -> ApiErrorDto {
    match error {
        ApplicationError::Store(error) => store_error(error),
        ApplicationError::Plan(error) => plan_error(error),
        ApplicationError::UnknownPlan(plan_id) => ApiErrorDto::new(
            ApiErrorCode::InvalidPlan,
            format!("plan {plan_id} is unknown or already consumed"),
            false,
        ),
        ApplicationError::UnknownProvider(id) => not_found("provider", id.to_string()),
        ApplicationError::UnknownTool(id) => not_found("tool", id.to_string()),
        ApplicationError::UnknownComponent {
            tool_id,
            component_id,
        } => not_found("component", format!("{tool_id}/{component_id}")),
        ApplicationError::UnknownStrategy {
            tool_id,
            component_id,
            strategy_id,
        } => not_found(
            "strategy",
            format!("{tool_id}/{component_id}/{strategy_id}"),
        ),
        ApplicationError::InstallationUnavailable(id) => not_found("installation", id.to_string()),
        ApplicationError::ProgramUnavailable { program, summary } => ApiErrorDto::new(
            ApiErrorCode::Unavailable,
            format!("program {program} is unavailable: {summary}"),
            true,
        ),
        ApplicationError::Provider(error) => ApiErrorDto::new(
            ApiErrorCode::Unavailable,
            error.to_string(),
            error.recoverability == Recoverability::Retryable,
        ),
        ApplicationError::Registry(error) => registry_error(error),
        ApplicationError::Execution(error) => ApiErrorDto::new(
            if matches!(error.kind, ExecutionErrorKind::InvalidPlan) {
                ApiErrorCode::InvalidPlan
            } else {
                ApiErrorCode::Internal
            },
            error.to_string(),
            error.retryable,
        ),
        ApplicationError::DuplicateSelection { .. }
        | ApplicationError::NoDefaultStrategy { .. }
        | ApplicationError::ToolHasNoInstallation(_) => {
            ApiErrorDto::new(ApiErrorCode::InvalidRequest, error.to_string(), false)
        }
    }
}

fn plan_error(error: PlanError) -> ApiErrorDto {
    let retryable = matches!(
        error,
        PlanError::Expired
            | PlanError::ConfigChanged
            | PlanError::EnvironmentChanged
            | PlanError::VersionChanged
    );
    ApiErrorDto::new(ApiErrorCode::InvalidPlan, error.to_string(), retryable)
}

fn registry_error(error: ProviderRegistryError) -> ApiErrorDto {
    match &error {
        ProviderRegistryError::UnknownProvider { provider_id } => {
            not_found("provider", provider_id.to_string())
        }
        ProviderRegistryError::Provider(provider) => ApiErrorDto::new(
            ApiErrorCode::Unavailable,
            provider.to_string(),
            provider.recoverability == Recoverability::Retryable,
        ),
        ProviderRegistryError::Disabled { .. }
        | ProviderRegistryError::UnsupportedOperation { .. }
        | ProviderRegistryError::DuplicateProvider { .. } => {
            ApiErrorDto::new(ApiErrorCode::InvalidRequest, error.to_string(), false)
        }
    }
}

fn store_error(error: StoreError) -> ApiErrorDto {
    let (code, retryable) = match error.kind {
        StoreErrorKind::RevisionConflict => (ApiErrorCode::Conflict, true),
        StoreErrorKind::InvalidRunId | StoreErrorKind::InvalidFileName => {
            (ApiErrorCode::InvalidRequest, false)
        }
        StoreErrorKind::PathResolution | StoreErrorKind::Io => (ApiErrorCode::Internal, true),
        StoreErrorKind::Corrupt | StoreErrorKind::Serialize | StoreErrorKind::UnsupportedSchema => {
            (ApiErrorCode::Internal, false)
        }
    };
    ApiErrorDto::new(code, error.to_string(), retryable)
        .with_detail("path", error.path.to_string_lossy())
}

fn catalog_error(error: CatalogError) -> ApiErrorDto {
    let mut dto = ApiErrorDto::new(ApiErrorCode::InvalidRequest, error.to_string(), false)
        .with_detail("path", error.source.to_string_lossy())
        .with_detail("field", error.field_path.to_string())
        .with_detail("hint", error.hint.to_string());
    if let Some(line) = error.line {
        dto = dto.with_detail("line", line.to_string());
    }
    if let Some(column) = error.column {
        dto = dto.with_detail("column", column.to_string());
    }
    dto
}

fn not_found(kind: &str, id: String) -> ApiErrorDto {
    ApiErrorDto::new(
        ApiErrorCode::NotFound,
        format!("{kind} {id} was not found"),
        false,
    )
}
