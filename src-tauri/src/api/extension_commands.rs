use super::{ApiErrorDto, ApiState};
use crate::{agents::AgentTarget, extensions::*};
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn list_agent_targets(
    state: State<'_, ApiState>,
) -> Result<Vec<AgentTarget>, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.targets())
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn save_agent_targets(
    state: State<'_, ApiState>,
    request: SaveAgentsRequest,
) -> Result<Vec<AgentTarget>, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.save_agents(request))
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn list_skills(state: State<'_, ApiState>) -> Result<SkillsSnapshot, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.list_skills())
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn list_skill_sources(state: State<'_, ApiState>) -> Result<SkillsSnapshot, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.list_skills())
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn save_skill_source(
    state: State<'_, ApiState>,
    request: SaveSourceRequest,
) -> Result<SkillsSnapshot, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.save_source(request))
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn delete_skill_source(
    state: State<'_, ApiState>,
    request: DeleteSourceRequest,
) -> Result<SkillsSnapshot, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.delete_source(request))
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn discover_skills(
    state: State<'_, ApiState>,
    request: DiscoverRequest,
) -> Result<SkillDiscovery, ApiErrorDto> {
    state.service().extensions.discover(request).await
}

#[tauri::command]
#[specta::specta]
pub async fn search_skills(
    state: State<'_, ApiState>,
    request: SearchRequest,
) -> Result<Vec<SearchSkill>, ApiErrorDto> {
    state.service().extensions.search(request).await
}

#[tauri::command]
#[specta::specta]
pub async fn scan_skill_imports(state: State<'_, ApiState>) -> Result<SkillDiscovery, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.scan_skill_imports())
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn check_skill_updates(
    state: State<'_, ApiState>,
    request: CheckUpdatesRequest,
) -> Result<Vec<SkillUpdate>, ApiErrorDto> {
    state.service().extensions.check_updates(request).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_skill_backups(
    state: State<'_, ApiState>,
) -> Result<Vec<ExtensionBackup>, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.list_backups())
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn preview_skill_operation(
    state: State<'_, ApiState>,
    request: SkillOperationRequest,
) -> Result<ExtensionPlan, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.preview_skill(request))
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn list_mcp_servers(state: State<'_, ApiState>) -> Result<McpSnapshot, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.list_mcp())
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn scan_mcp_imports(state: State<'_, ApiState>) -> Result<McpScan, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.scan_mcp_imports())
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn validate_mcp_server(
    state: State<'_, ApiState>,
    request: McpEdit,
) -> Result<Vec<Diagnostic>, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.validate_mcp(request))
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn preview_mcp_operation(
    state: State<'_, ApiState>,
    request: McpOperationRequest,
) -> Result<ExtensionPlan, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.preview_mcp(request))
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}

#[tauri::command]
#[specta::specta]
pub async fn confirm_extension_operation(
    state: State<'_, ApiState>,
    request: ConfirmExtensionRequest,
) -> Result<ExtensionStarted, ApiErrorDto> {
    state.service().extensions.confirm(request)
}

#[tauri::command]
#[specta::specta]
pub async fn extension_operation_result(
    state: State<'_, ApiState>,
    request: IdRequest,
) -> Result<ExtensionResult, ApiErrorDto> {
    let service = std::sync::Arc::clone(&state.service().extensions);
    tokio::task::spawn_blocking(move || service.result(&request.id))
        .await
        .map_err(|_| err("failed", "操作执行任务失败"))?
}
