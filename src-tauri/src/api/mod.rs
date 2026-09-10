mod commands;
mod extension_commands;
pub use extension_commands::*;
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
            save_manifest,
            list_agent_targets,
            save_agent_targets,
            list_skills,
            list_skill_sources,
            save_skill_source,
            delete_skill_source,
            discover_skills,
            search_skills,
            scan_skill_imports,
            check_skill_updates,
            list_skill_backups,
            preview_skill_operation,
            list_mcp_servers,
            scan_mcp_imports,
            validate_mcp_server,
            preview_mcp_operation,
            confirm_extension_operation,
            extension_operation_result
        ])
        .events(collect_events![
            RefreshProgressEventDto,
            RunStateEventDto,
            RunOutputEventDto,
            ToolStateEventDto,
            ExtensionChangedEventDto,
            McpChangedEventDto
        ])
}

pub fn export_bindings(path: impl AsRef<Path>) -> Result<(), specta_typescript::Error> {
    specta_builder().export(Typescript::default(), path)
}
