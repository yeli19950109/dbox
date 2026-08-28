use std::sync::Arc;

use dbox_lib::api::{serve_dev_http, ApiEventEmitter, ApiService, DevHttpEventBus};
use dbox_lib::persistence::AppPaths;

const APP_IDENTIFIER: &str = "com.yel.dbox";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dbox_lib::environment::fix_gui_path();
    let events = DevHttpEventBus::new();
    let emitter: Arc<dyn ApiEventEmitter> = Arc::new(events.clone());
    let paths = AppPaths::from_desktop_identifier(APP_IDENTIFIER)?;
    let service = Arc::new(ApiService::production(paths, emitter)?);
    serve_dev_http(service, events).await?;
    Ok(())
}
