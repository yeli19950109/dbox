use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::application::{ApplicationSnapshot, ProviderRefreshReport};
use crate::catalog::{CatalogDiagnostic, CatalogError};
use crate::domain::{
    Component, Installation, InstallationScope, PackageKind, Run, RunStatus, Strategy,
    StrategyKind, Tool,
};
use crate::executor::{CommandSpecView, EnvironmentValueView, ExecutionResult, UpdatePlanView};
use crate::persistence::{LogRetentionPolicy, RunLogEntry, RunLogStream, Settings};
use crate::providers::{ProviderError, ProviderOperation, ProviderStatus, Recoverability};
use crate::version::{ComponentStatus, StatusReason, ToolStatus, VerificationStatus};

pub const API_SCHEMA_REVISION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    InvalidRequest,
    NotFound,
    InvalidPlan,
    Conflict,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorDto {
    pub code: ApiErrorCode,
    pub message: String,
    pub retryable: bool,
    pub details: BTreeMap<String, String>,
}

impl ApiErrorDto {
    pub fn new(code: ApiErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            details: BTreeMap::new(),
        }
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum RefreshScopeDto {
    All,
    Provider {
        #[serde(rename = "providerId")]
        provider_id: String,
    },
    Tool {
        #[serde(rename = "toolId")]
        tool_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRequestDto {
    pub scope: RefreshScopeDto,
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRequestDto {
    pub selections: Vec<UpdateSelectionDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSelectionDto {
    pub tool_id: String,
    pub component_id: String,
    pub strategy_id: Option<String>,
    pub target_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmRequestDto {
    pub plan_id: String,
    pub plan_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CancelRequestDto {
    pub run_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CancelDispositionDto {
    CancellationRequested,
    AlreadyFinished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CancelResponseDto {
    pub run_id: String,
    pub disposition: CancelDispositionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    pub schema_revision: u32,
    pub tools: Vec<ToolDto>,
    pub installations: Vec<InstallationDto>,
    pub providers: Vec<ProviderRefreshReportDto>,
    pub catalog_diagnostics: Vec<CatalogDiagnosticDto>,
    pub catalog_errors: Vec<CatalogErrorDto>,
    pub config_revision: String,
    pub state_revision: String,
    pub environment_revision: String,
    pub refreshed_at: Option<String>,
}

impl SnapshotDto {
    pub fn from_domain(snapshot: &ApplicationSnapshot, installations: &[Installation]) -> Self {
        Self {
            schema_revision: API_SCHEMA_REVISION,
            tools: snapshot.tools.iter().map(ToolDto::from).collect(),
            installations: installations.iter().map(InstallationDto::from).collect(),
            providers: snapshot
                .providers
                .values()
                .map(ProviderRefreshReportDto::from)
                .collect(),
            catalog_diagnostics: snapshot
                .catalog_diagnostics
                .iter()
                .map(CatalogDiagnosticDto::from)
                .collect(),
            catalog_errors: snapshot
                .catalog_errors
                .iter()
                .map(CatalogErrorDto::from)
                .collect(),
            config_revision: snapshot.config_revision.clone(),
            state_revision: snapshot.state_revision.clone(),
            environment_revision: snapshot.environment_revision.clone(),
            refreshed_at: snapshot.refreshed_at.map(|value| value.to_rfc3339()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolDto {
    pub id: String,
    pub display_name: String,
    pub installation_ids: Vec<String>,
    pub components: Vec<ComponentDto>,
    pub categories: Vec<String>,
    pub homepage: Option<String>,
    pub hidden: bool,
    pub status: ToolStatusDto,
}

impl From<&Tool> for ToolDto {
    fn from(tool: &Tool) -> Self {
        Self {
            id: tool.id.to_string(),
            display_name: tool.display_name.clone(),
            installation_ids: tool
                .installation_ids
                .iter()
                .map(ToString::to_string)
                .collect(),
            components: tool.components.iter().map(ComponentDto::from).collect(),
            categories: tool.categories.clone(),
            homepage: tool.homepage.clone(),
            hidden: tool.hidden,
            status: ToolStatusDto::from(tool.status()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatusDto {
    pub status: String,
    pub has_updates: Option<bool>,
    pub failed_components: Option<u32>,
    pub unknown_components: Option<u32>,
    pub unsupported_components: Option<u32>,
}

impl From<ToolStatus> for ToolStatusDto {
    fn from(status: ToolStatus) -> Self {
        let mut value = Self {
            status: tool_status_name(&status).into(),
            has_updates: None,
            failed_components: None,
            unknown_components: None,
            unsupported_components: None,
        };
        if let ToolStatus::Partial {
            has_updates,
            failed_components,
            unknown_components,
            unsupported_components,
        } = status
        {
            value.has_updates = Some(has_updates);
            value.failed_components = Some(to_u32(failed_components));
            value.unknown_components = Some(to_u32(unknown_components));
            value.unsupported_components = Some(to_u32(unsupported_components));
        }
        value
    }
}

fn tool_status_name(status: &ToolStatus) -> &'static str {
    match status {
        ToolStatus::NotInstalled => "not_installed",
        ToolStatus::Checking => "checking",
        ToolStatus::UpToDate => "up_to_date",
        ToolStatus::UpdateAvailable => "update_available",
        ToolStatus::Unknown => "unknown",
        ToolStatus::Unsupported => "unsupported",
        ToolStatus::Updating => "updating",
        ToolStatus::UpdateSucceeded => "update_succeeded",
        ToolStatus::UpdateFailed => "update_failed",
        ToolStatus::Cancelled => "cancelled",
        ToolStatus::Partial { .. } => "partial",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDto {
    pub id: String,
    pub display_name: String,
    pub installation_id: Option<String>,
    pub strategies: Vec<StrategyDto>,
    pub default_strategy: Option<String>,
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub status: ComponentStatusDto,
}

impl From<&Component> for ComponentDto {
    fn from(component: &Component) -> Self {
        Self {
            id: component.id.to_string(),
            display_name: component.display_name.clone(),
            installation_id: component.installation_id.as_ref().map(ToString::to_string),
            strategies: component.strategies.iter().map(StrategyDto::from).collect(),
            default_strategy: component.default_strategy.as_ref().map(ToString::to_string),
            installed_version: component
                .installed_version
                .as_ref()
                .map(|version| version.raw().to_owned()),
            latest_version: component
                .latest_version
                .as_ref()
                .map(|version| version.raw().to_owned()),
            status: ComponentStatusDto::from(&component.status),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStatusDto {
    pub status: String,
    pub reason: Option<StatusReasonDto>,
    pub verification: Option<VerificationStatusDto>,
}

impl From<&ComponentStatus> for ComponentStatusDto {
    fn from(status: &ComponentStatus) -> Self {
        let (name, reason, verification) = match status {
            ComponentStatus::NotInstalled => ("not_installed", None, None),
            ComponentStatus::Checking => ("checking", None, None),
            ComponentStatus::UpToDate => ("up_to_date", None, None),
            ComponentStatus::UpdateAvailable => ("update_available", None, None),
            ComponentStatus::Unknown { reason } => {
                ("unknown", Some(StatusReasonDto::from(reason)), None)
            }
            ComponentStatus::Unsupported { reason } => {
                ("unsupported", Some(StatusReasonDto::from(reason)), None)
            }
            ComponentStatus::CheckFailed { reason } => {
                ("check_failed", Some(StatusReasonDto::from(reason)), None)
            }
            ComponentStatus::Updating => ("updating", None, None),
            ComponentStatus::UpdateSucceeded { verification } => (
                "update_succeeded",
                None,
                Some(VerificationStatusDto::from(verification)),
            ),
            ComponentStatus::UpdateFailed { reason } => {
                ("update_failed", Some(StatusReasonDto::from(reason)), None)
            }
            ComponentStatus::Cancelled => ("cancelled", None, None),
        };
        Self {
            status: name.into(),
            reason,
            verification,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StrategyDto {
    pub id: String,
    pub display_name: String,
    pub kind: StrategyKindDto,
    pub enabled: bool,
}

impl From<&Strategy> for StrategyDto {
    fn from(strategy: &Strategy) -> Self {
        Self {
            id: strategy.id.to_string(),
            display_name: strategy.display_name.clone(),
            kind: StrategyKindDto::from(&strategy.kind),
            enabled: strategy.enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StrategyKindDto {
    ProviderDefault,
    Command { program: String, args: Vec<String> },
    PackageManager { manager: String, args: Vec<String> },
}

impl From<&StrategyKind> for StrategyKindDto {
    fn from(kind: &StrategyKind) -> Self {
        match kind {
            StrategyKind::ProviderDefault => Self::ProviderDefault,
            StrategyKind::Command { program, args } => Self::Command {
                program: program.clone(),
                args: args.clone(),
            },
            StrategyKind::PackageManager { manager, args } => Self::PackageManager {
                manager: manager.clone(),
                args: args.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstallationDto {
    pub id: String,
    pub provider_id: String,
    pub package_kind: String,
    pub package_name: String,
    pub scope: String,
    pub install_path: Option<String>,
    pub executables: Vec<ExecutableDto>,
    pub installed_version: Option<String>,
    pub hidden: bool,
}

impl From<&Installation> for InstallationDto {
    fn from(installation: &Installation) -> Self {
        Self {
            id: installation.id.to_string(),
            provider_id: installation.provider_id.to_string(),
            package_kind: package_kind_name(&installation.package.kind),
            package_name: installation.package.name.clone(),
            scope: installation_scope_name(installation.scope).into(),
            install_path: installation
                .install_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            executables: installation
                .executables
                .iter()
                .map(|executable| ExecutableDto {
                    name: executable.name.clone(),
                    path: executable.path.to_string_lossy().into_owned(),
                })
                .collect(),
            installed_version: installation
                .installed_version
                .as_ref()
                .map(|version| version.raw().to_owned()),
            hidden: installation.hidden,
        }
    }
}

fn package_kind_name(kind: &PackageKind) -> String {
    match kind {
        PackageKind::Other(value) => value.clone(),
        _ => kind.as_slug().into(),
    }
}

fn installation_scope_name(scope: InstallationScope) -> &'static str {
    match scope {
        InstallationScope::System => "system",
        InstallationScope::Global => "global",
        InstallationScope::User => "user",
        InstallationScope::Project => "project",
        InstallationScope::Unknown => "unknown",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableDto {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRefreshReportDto {
    pub provider_id: String,
    pub enabled: bool,
    pub status: Option<ProviderStatusDto>,
    pub errors: Vec<ProviderErrorDto>,
    pub refreshed_at: String,
}

impl From<&ProviderRefreshReport> for ProviderRefreshReportDto {
    fn from(report: &ProviderRefreshReport) -> Self {
        Self {
            provider_id: report.provider_id.to_string(),
            enabled: report.enabled,
            status: report.status.as_ref().map(ProviderStatusDto::from),
            errors: report.errors.iter().map(ProviderErrorDto::from).collect(),
            refreshed_at: report.refreshed_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatusDto {
    pub available: bool,
    pub version: Option<String>,
    pub detail: Option<String>,
}

impl From<&ProviderStatus> for ProviderStatusDto {
    fn from(status: &ProviderStatus) -> Self {
        Self {
            available: status.available,
            version: status
                .version
                .as_ref()
                .map(|version| version.raw().to_owned()),
            detail: status.detail.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderErrorDto {
    pub provider_id: String,
    pub operation: String,
    pub recoverability: String,
    pub summary: String,
}

impl From<&ProviderError> for ProviderErrorDto {
    fn from(error: &ProviderError) -> Self {
        Self {
            provider_id: error.provider_id.to_string(),
            operation: provider_operation_name(error.operation).into(),
            recoverability: recoverability_name(error.recoverability).into(),
            summary: error.summary.clone(),
        }
    }
}

fn provider_operation_name(operation: ProviderOperation) -> &'static str {
    match operation {
        ProviderOperation::Probe => "probe",
        ProviderOperation::Scan => "scan",
        ProviderOperation::CheckUpdates => "check_updates",
        ProviderOperation::PlanUpdate => "plan_update",
    }
}

fn recoverability_name(value: Recoverability) -> &'static str {
    match value {
        Recoverability::Retryable => "retryable",
        Recoverability::RequiresConfiguration => "requires_configuration",
        Recoverability::Permanent => "permanent",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDiagnosticDto {
    pub source: String,
    pub manifest_id: String,
    pub kind: String,
    pub matching_tool_ids: Vec<String>,
    pub message: String,
}

impl From<&CatalogDiagnostic> for CatalogDiagnosticDto {
    fn from(diagnostic: &CatalogDiagnostic) -> Self {
        Self {
            source: diagnostic.source.to_string_lossy().into_owned(),
            manifest_id: diagnostic.manifest_id.to_string(),
            kind: match diagnostic.kind {
                crate::catalog::CatalogDiagnosticKind::NoMatch => "no_match",
                crate::catalog::CatalogDiagnosticKind::AmbiguousMatch => "ambiguous_match",
            }
            .into(),
            matching_tool_ids: diagnostic
                .matching_tool_ids
                .iter()
                .map(ToString::to_string)
                .collect(),
            message: diagnostic.message.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CatalogErrorDto {
    pub source: String,
    pub field_path: String,
    pub kind: String,
    pub message: String,
    pub hint: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

impl From<&CatalogError> for CatalogErrorDto {
    fn from(error: &CatalogError) -> Self {
        Self {
            source: error.source.to_string_lossy().into_owned(),
            field_path: error.field_path.to_string(),
            kind: catalog_error_kind_name(&error.kind).into(),
            message: error.message.to_string(),
            hint: error.hint.to_string(),
            line: error.line.map(to_u32),
            column: error.column.map(to_u32),
        }
    }
}

fn catalog_error_kind_name(kind: &crate::catalog::CatalogErrorKind) -> &'static str {
    use crate::catalog::CatalogErrorKind;

    match kind {
        CatalogErrorKind::Io => "io",
        CatalogErrorKind::Parse => "parse",
        CatalogErrorKind::Serialize => "serialize",
        CatalogErrorKind::UnsupportedSchemaVersion => "unsupported_schema_version",
        CatalogErrorKind::DuplicateId => "duplicate_id",
        CatalogErrorKind::InvalidRegex => "invalid_regex",
        CatalogErrorKind::InvalidCommand => "invalid_command",
        CatalogErrorKind::InvalidMatcher => "invalid_matcher",
        CatalogErrorKind::IncompleteDefinition => "incomplete_definition",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePlanDto {
    pub plan_id: String,
    pub plan_hash: String,
    pub tool_id: String,
    pub installation_id: String,
    pub component_id: String,
    pub strategy_id: String,
    pub command: CommandSpecDto,
    pub config_revision: String,
    pub environment_revision: String,
    pub version_snapshot: String,
    pub created_at: String,
    pub expires_at: String,
}

impl From<&UpdatePlanView> for UpdatePlanDto {
    fn from(plan: &UpdatePlanView) -> Self {
        Self {
            plan_id: plan.plan_id.to_string(),
            plan_hash: plan.plan_hash.as_str().into(),
            tool_id: plan.tool_id.to_string(),
            installation_id: plan.installation_id.to_string(),
            component_id: plan.component_id.to_string(),
            strategy_id: plan.strategy_id.to_string(),
            command: CommandSpecDto::from(&plan.command),
            config_revision: plan.config_revision.clone(),
            environment_revision: plan.environment_revision.clone(),
            version_snapshot: plan.version_snapshot.clone(),
            created_at: plan.created_at.to_rfc3339(),
            expires_at: plan.expires_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentValueDto {
    pub value: String,
    pub sensitive: bool,
}

impl From<&EnvironmentValueView> for EnvironmentValueDto {
    fn from(value: &EnvironmentValueView) -> Self {
        Self {
            value: value.value.clone(),
            sensitive: value.sensitive,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpecDto {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, EnvironmentValueDto>,
    pub timeout_seconds: String,
    pub success_exit_codes: Vec<i32>,
    pub network_required: bool,
    pub post_check: Option<Box<CommandSpecDto>>,
    pub display_command: String,
}

impl From<&CommandSpecView> for CommandSpecDto {
    fn from(command: &CommandSpecView) -> Self {
        Self {
            program: command.program.to_string_lossy().into_owned(),
            args: command.args.clone(),
            cwd: command
                .cwd
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            env: command
                .env
                .iter()
                .map(|(name, value)| (name.clone(), EnvironmentValueDto::from(value)))
                .collect(),
            timeout_seconds: command.timeout_seconds.to_string(),
            success_exit_codes: command.success_exit_codes.iter().copied().collect(),
            network_required: command.network_required,
            post_check: command
                .post_check
                .as_deref()
                .map(CommandSpecDto::from)
                .map(Box::new),
            display_command: command.display_command.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmResponseDto {
    pub execution: ExecutionResultDto,
    pub snapshot: SnapshotDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResultDto {
    pub run_id: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub started_at: String,
    pub finished_at: String,
    pub output_tail: String,
    pub verification: Option<VerificationStatusDto>,
}

impl From<&ExecutionResult> for ExecutionResultDto {
    fn from(execution: &ExecutionResult) -> Self {
        Self {
            run_id: execution.run_id.to_string(),
            status: run_status_name(execution.status).into(),
            exit_code: execution.exit_code,
            started_at: execution.started_at.to_rfc3339(),
            finished_at: execution.finished_at.to_rfc3339(),
            output_tail: execution.output_tail.clone(),
            verification: execution
                .verification
                .as_ref()
                .map(VerificationStatusDto::from),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StatusReasonDto {
    pub code: String,
    pub summary: String,
    pub retryable: bool,
}

impl From<&StatusReason> for StatusReasonDto {
    fn from(reason: &StatusReason) -> Self {
        Self {
            code: reason.code.clone(),
            summary: reason.summary.clone(),
            retryable: reason.retryable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VerificationStatusDto {
    pub status: String,
    pub reason: Option<StatusReasonDto>,
}

impl From<&VerificationStatus> for VerificationStatusDto {
    fn from(status: &VerificationStatus) -> Self {
        match status {
            VerificationStatus::Verified => Self {
                status: "verified".into(),
                reason: None,
            },
            VerificationStatus::VerificationUnknown { reason } => Self {
                status: "verification_unknown".into(),
                reason: Some(StatusReasonDto::from(reason)),
            },
            VerificationStatus::VerificationFailed { reason } => Self {
                status: "verification_failed".into(),
                reason: Some(StatusReasonDto::from(reason)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunHistoryDto {
    pub runs: Vec<RunDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunDto {
    pub id: String,
    pub tool_id: Option<String>,
    pub subject: crate::domain::RunSubject,
    pub operation: Option<String>,
    pub resource_names: Vec<String>,
    pub component_ids: Vec<String>,
    pub status: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub batch_id: Option<String>,
    pub retry_of: Option<String>,
    pub has_log: bool,
    pub summary: Option<RunSummaryDto>,
}

impl From<&Run> for RunDto {
    fn from(run: &Run) -> Self {
        Self {
            id: run.id.to_string(),
            tool_id: run.tool_id.as_ref().map(ToString::to_string),
            subject: run.subject,
            operation: run.operation.clone(),
            resource_names: run.resource_names.clone(),
            component_ids: run.component_ids.iter().map(ToString::to_string).collect(),
            status: run_status_name(run.status).into(),
            created_at: run.created_at.to_rfc3339(),
            started_at: run.started_at.map(|value| value.to_rfc3339()),
            finished_at: run.finished_at.map(|value| value.to_rfc3339()),
            batch_id: run.batch_id.clone(),
            retry_of: run.retry_of.as_ref().map(ToString::to_string),
            has_log: run.log_path.is_some(),
            summary: run.summary.as_ref().map(|summary| RunSummaryDto {
                exit_code: summary.exit_code,
                output_tail: summary.output_tail.clone(),
                verification: summary
                    .verification
                    .as_ref()
                    .map(VerificationStatusDto::from),
                error: summary.error.clone(),
            }),
        }
    }
}

pub fn run_status_name(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::TimedOut => "timed_out",
        RunStatus::Interrupted => "interrupted",
        RunStatus::Partial => "partial",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryDto {
    pub exit_code: Option<i32>,
    pub output_tail: String,
    pub verification: Option<VerificationStatusDto>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunLogRequestDto {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunLogDto {
    pub run_id: String,
    pub entries: Vec<RunLogEntryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunLogEntryDto {
    pub sequence: String,
    pub timestamp: String,
    pub stream: String,
    pub message: String,
}

impl From<&RunLogEntry> for RunLogEntryDto {
    fn from(entry: &RunLogEntry) -> Self {
        Self {
            sequence: entry.sequence.to_string(),
            timestamp: entry.timestamp.to_rfc3339(),
            stream: match entry.stream {
                RunLogStream::Stdout => "stdout",
                RunLogStream::Stderr => "stderr",
                RunLogStream::System => "system",
            }
            .into(),
            message: entry.message.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDocumentDto {
    pub revision: String,
    pub settings: SettingsValueDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveSettingsRequestDto {
    pub expected_revision: String,
    pub settings: SettingsValueDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SettingsValueDto {
    pub provider_enabled: BTreeMap<String, bool>,
    pub default_timeout_seconds: String,
    pub log_retention: LogRetentionDto,
    pub executable_overrides: BTreeMap<String, String>,
    pub component_strategies: BTreeMap<String, BTreeMap<String, String>>,
}

impl From<&Settings> for SettingsValueDto {
    fn from(settings: &Settings) -> Self {
        Self {
            provider_enabled: settings
                .provider_enabled
                .iter()
                .map(|(id, enabled)| (id.to_string(), *enabled))
                .collect(),
            default_timeout_seconds: settings.default_timeout_seconds.to_string(),
            log_retention: LogRetentionDto {
                max_files: to_u32(settings.log_retention.max_files),
                max_total_bytes: settings.log_retention.max_total_bytes.to_string(),
            },
            executable_overrides: settings
                .executable_overrides
                .iter()
                .map(|(name, path)| (name.clone(), path.to_string_lossy().into_owned()))
                .collect(),
            component_strategies: settings
                .component_strategies
                .iter()
                .map(|(tool_id, strategies)| {
                    (
                        tool_id.to_string(),
                        strategies
                            .iter()
                            .map(|(component_id, strategy_id)| {
                                (component_id.to_string(), strategy_id.to_string())
                            })
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogRetentionDto {
    pub max_files: u32,
    pub max_total_bytes: String,
}

impl LogRetentionDto {
    pub fn to_domain(&self) -> Result<LogRetentionPolicy, ApiErrorDto> {
        let max_total_bytes = parse_u64("logRetention.maxTotalBytes", &self.max_total_bytes)?;
        Ok(LogRetentionPolicy {
            max_files: self.max_files as usize,
            max_total_bytes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManifestInputDto {
    pub file_name: String,
    pub contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManifestReadRequestDto {
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManifestDocumentDto {
    pub file_name: String,
    pub revision: Option<String>,
    pub contents: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveManifestRequestDto {
    pub file_name: String,
    pub contents: String,
    pub expected_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManifestValidationDto {
    pub file_name: String,
    pub manifest_id: String,
    pub normalized_toml: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedManifestDto {
    pub revision: String,
    pub validation: ManifestValidationDto,
    pub snapshot: SnapshotDto,
}

pub fn parse_u64(field: &str, value: &str) -> Result<u64, ApiErrorDto> {
    value.parse::<u64>().map_err(|_| {
        ApiErrorDto::new(
            ApiErrorCode::InvalidRequest,
            format!("{field} must be an unsigned decimal integer"),
            false,
        )
        .with_detail("field", field)
    })
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
