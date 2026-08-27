pub mod catalog;
pub mod domain;
pub mod environment;
pub mod executor;
pub mod persistence;
pub mod providers;
pub mod version;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
