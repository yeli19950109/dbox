use tauri::State;

use super::dto::*;
use super::service::ApiState;

#[tauri::command]
#[specta::specta]
pub async fn snapshot(state: State<'_, ApiState>) -> Result<SnapshotDto, ApiErrorDto> {
    state.service().snapshot().await
}

#[tauri::command]
#[specta::specta]
pub async fn refresh(
    state: State<'_, ApiState>,
    request: RefreshRequestDto,
) -> Result<SnapshotDto, ApiErrorDto> {
    state.service().refresh(request).await
}

#[tauri::command]
#[specta::specta]
pub async fn preview(
    state: State<'_, ApiState>,
    request: PreviewRequestDto,
) -> Result<Vec<UpdatePlanDto>, ApiErrorDto> {
    state.service().preview(request).await
}

#[tauri::command]
#[specta::specta]
pub async fn confirm(
    state: State<'_, ApiState>,
    request: ConfirmRequestDto,
) -> Result<ConfirmResponseDto, ApiErrorDto> {
    state.service().confirm(request).await
}

#[tauri::command]
#[specta::specta]
pub async fn cancel(
    state: State<'_, ApiState>,
    request: CancelRequestDto,
) -> Result<CancelResponseDto, ApiErrorDto> {
    state.service().cancel(request).await
}

#[tauri::command]
#[specta::specta]
pub fn run_history(state: State<'_, ApiState>) -> Result<RunHistoryDto, ApiErrorDto> {
    state.service().run_history()
}

#[tauri::command]
#[specta::specta]
pub async fn run_log(
    state: State<'_, ApiState>,
    request: RunLogRequestDto,
) -> Result<RunLogDto, ApiErrorDto> {
    state.service().run_log(request).await
}

#[tauri::command]
#[specta::specta]
pub fn settings(state: State<'_, ApiState>) -> Result<SettingsDocumentDto, ApiErrorDto> {
    state.service().settings()
}

#[tauri::command]
#[specta::specta]
pub async fn save_settings(
    state: State<'_, ApiState>,
    request: SaveSettingsRequestDto,
) -> Result<SettingsDocumentDto, ApiErrorDto> {
    state.service().save_settings(request).await
}

#[tauri::command]
#[specta::specta]
pub fn validate_manifest(
    state: State<'_, ApiState>,
    request: ManifestInputDto,
) -> Result<ManifestValidationDto, ApiErrorDto> {
    state.service().validate_manifest(request)
}

#[tauri::command]
#[specta::specta]
pub fn read_manifest(
    state: State<'_, ApiState>,
    request: ManifestReadRequestDto,
) -> Result<ManifestDocumentDto, ApiErrorDto> {
    state.service().read_manifest(request)
}

#[tauri::command]
#[specta::specta]
pub async fn save_manifest(
    state: State<'_, ApiState>,
    request: SaveManifestRequestDto,
) -> Result<SavedManifestDto, ApiErrorDto> {
    state.service().save_manifest(request).await
}
