mod commands;
#[cfg(all(debug_assertions, feature = "dev-http"))]
mod dev_http;
mod dto;
mod events;
mod service;

use std::path::Path;

use specta_typescript::Typescript;
use tauri_specta::{collect_commands, collect_events, Builder};

pub use commands::*;
#[cfg(all(debug_assertions, feature = "dev-http"))]
pub use dev_http::*;
pub use dto::*;
pub use events::*;
pub use service::*;

pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            snapshot,
            refresh,
            preview,
            confirm,
            cancel,
            run_history,
            run_log,
            settings,
            save_settings,
            validate_manifest,
            read_manifest,
            save_manifest
        ])
        .events(collect_events![
            RefreshProgressEventDto,
            RunStateEventDto,
            RunOutputEventDto,
            ToolStateEventDto
        ])
}

pub fn export_bindings(path: impl AsRef<Path>) -> Result<(), specta_typescript::Error> {
    specta_builder().export(Typescript::default(), path)
}
