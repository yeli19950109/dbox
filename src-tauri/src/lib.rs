#[cfg(all(not(debug_assertions), feature = "dev-http"))]
compile_error!("the dev-http feature is restricted to debug builds");

pub mod agents;
pub mod api;
pub mod application;
pub mod catalog;
pub mod domain;
pub mod environment;
pub mod executor;
pub mod extensions;
pub mod mcp;
pub mod persistence;
pub mod providers;
pub mod skills;
pub mod version;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::Manager;

    let api_builder = api::specta_builder();
    tauri::Builder::default()
        .invoke_handler(api_builder.invoke_handler())
        .setup(move |app| {
            api_builder.mount_events(app);
            app.manage(api::ApiState::production(app.handle())?);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_smoke_test() {
        assert_eq!(env!("CARGO_PKG_NAME"), "dbox");
    }
}
