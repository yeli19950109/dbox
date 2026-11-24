use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::{ApiErrorCode, ApiErrorDto, ApiState};
use crate::llama::{self, LlamaInstalledVersion, LlamaInventory, LlamaReleasePage};

#[derive(Deserialize, Serialize, Type)]
pub struct LlamaReleaseRequest {
    pub page: u32,
}

#[derive(Deserialize, Serialize, Type)]
pub struct LlamaVersionRequest {
    pub tag: String,
}

pub(super) fn llama_error(message: String) -> ApiErrorDto {
    ApiErrorDto::new(ApiErrorCode::Unavailable, message, true)
}

#[tauri::command]
#[specta::specta]
pub async fn llama_inventory(state: State<'_, ApiState>) -> Result<LlamaInventory, ApiErrorDto> {
    llama::inventory(&state.service().llama_root)
        .await
        .map_err(llama_error)
}

#[tauri::command]
#[specta::specta]
pub async fn llama_releases(request: LlamaReleaseRequest) -> Result<LlamaReleasePage, ApiErrorDto> {
    llama::releases(request.page).await.map_err(llama_error)
}

#[tauri::command]
#[specta::specta]
pub async fn llama_install(
    state: State<'_, ApiState>,
    request: LlamaVersionRequest,
) -> Result<LlamaInstalledVersion, ApiErrorDto> {
    llama::install(&state.service().llama_root, &request.tag)
        .await
        .map_err(llama_error)
}

#[tauri::command]
#[specta::specta]
pub async fn llama_remove(
    state: State<'_, ApiState>,
    request: LlamaVersionRequest,
) -> Result<(), ApiErrorDto> {
    llama::remove(&state.service().llama_root, &request.tag)
        .await
        .map_err(llama_error)
}

#[tauri::command]
#[specta::specta]
pub async fn llama_choose_model(window: tauri::Window) -> Result<Option<String>, ApiErrorDto> {
    #[cfg(target_os = "macos")]
    {
        let _guard = lock_model_picker()?;
        let owner = window.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        window
            .run_on_main_thread(move || open_model_sheet(owner, send))
            .map_err(|error| llama_error(error.to_string()))?;
        receive
            .await
            .map_err(|_| llama_error("模型选择窗口已关闭".into()))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Err(ApiErrorDto::new(
            ApiErrorCode::Unavailable,
            "模型文件选择器需要 macOS",
            false,
        ))
    }
}

#[cfg(target_os = "macos")]
pub(super) fn lock_model_picker() -> Result<tokio::sync::MutexGuard<'static, ()>, ApiErrorDto> {
    static PICKER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    PICKER
        .try_lock()
        .map_err(|_| ApiErrorDto::new(ApiErrorCode::Conflict, "模型选择窗口已打开", true))
}

#[cfg(target_os = "macos")]
fn open_model_sheet(
    owner: tauri::Window,
    send: tokio::sync::oneshot::Sender<Result<Option<String>, ApiErrorDto>>,
) {
    use block2::RcBlock;
    use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSOpenPanel, NSWindow};
    use objc2_foundation::{MainThreadMarker, NSArray, NSString};
    use std::cell::Cell;

    let mtm = MainThreadMarker::new().expect("Tauri dispatched to the main thread");
    let Ok(raw_window) = owner.ns_window() else {
        let _ = send.send(Err(llama_error("无法获取 DBox 窗口".into())));
        return;
    };
    // Tauri returns a live NSWindow on macOS. We borrow it only during this main-thread call.
    let parent = unsafe { &*raw_window.cast::<NSWindow>() };
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setTitle(Some(&NSString::from_str("选择 GGUF 模型")));
    // Also supported by macOS versions before allowedContentTypes was introduced.
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[NSString::from_str(
        "gguf",
    )])));
    let selected_panel = panel.clone();
    let send = Cell::new(Some(send));
    let completion = RcBlock::new(move |response| {
        let result = if response == NSModalResponseOK {
            selected_panel
                .URL()
                .and_then(|url| url.path())
                .map(|path| Some(path.to_string()))
                .ok_or_else(|| llama_error("无法读取所选模型的路径".into()))
        } else if response == NSModalResponseCancel {
            Ok(None)
        } else {
            Err(llama_error("无法显示模型选择窗口".into()))
        };
        if let Some(sender) = send.take() {
            let _ = sender.send(result);
        }
    });
    panel.beginSheetModalForWindow_completionHandler(parent, &completion);
}
