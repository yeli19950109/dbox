use std::sync::Arc;

use dbox_lib::api::{serve_dev_http, ApiEventEmitter, ApiService, DevHttpEventBus};
use dbox_lib::persistence::AppPaths;

const APP_IDENTIFIER: &str = "com.yel.dbox";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        if args.len() != 2 || args[0] != "--fixture-root" {
            return Err("usage: dev-http [--fixture-root ABSOLUTE_PATH]".into());
        }
        let root = std::path::PathBuf::from(&args[1]);
        if !root.is_absolute() {
            return Err("fixture root must be absolute".into());
        }
        let events = DevHttpEventBus::new();
        let service = ApiService::isolated(
            AppPaths::new(root.join("config"), root.join("data"), root.join("logs")),
            root.join("home"),
            Arc::new(events.clone()),
        )?;
        dbox_lib::api::serve_dev_http_on_port(Arc::new(service), events, 15431).await?;
        return Ok(());
    }
    let _ = dbox_lib::environment::fix_gui_path();
    let events = DevHttpEventBus::new();
    let emitter: Arc<dyn ApiEventEmitter> = Arc::new(events.clone());
    let paths = AppPaths::from_desktop_identifier(APP_IDENTIFIER)?;
    let service = Arc::new(ApiService::production(paths, emitter)?);
    serve_dev_http(service, events).await?;
    Ok(())
}
