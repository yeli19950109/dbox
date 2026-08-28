use std::convert::Infallible;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use super::dto::*;
use super::events::{ApiEvent, ApiEventEmitter};
use super::service::ApiService;

pub const DEV_HTTP_PORT: u16 = 1430;
pub const DEV_HTTP_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, DEV_HTTP_PORT);
const EVENT_BUFFER_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct DevHttpEventBus {
    sender: broadcast::Sender<ApiEvent>,
}

impl DevHttpEventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_BUFFER_CAPACITY);
        Self { sender }
    }

    fn subscribe(&self) -> broadcast::Receiver<ApiEvent> {
        self.sender.subscribe()
    }
}

impl Default for DevHttpEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiEventEmitter for DevHttpEventBus {
    fn emit(&self, event: ApiEvent) -> Result<(), String> {
        // A broadcast channel without subscribers is a normal idle state.
        let _ = self.sender.send(event);
        Ok(())
    }
}

#[derive(Clone)]
struct DevHttpState {
    service: Arc<ApiService>,
    events: DevHttpEventBus,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CommandResponse<T> {
    Ok { data: T },
    Error { error: ApiErrorDto },
}

impl<T> From<Result<T, ApiErrorDto>> for CommandResponse<T> {
    fn from(result: Result<T, ApiErrorDto>) -> Self {
        match result {
            Ok(data) => Self::Ok { data },
            Err(error) => Self::Error { error },
        }
    }
}

pub fn dev_http_router(service: Arc<ApiService>, event_bus: DevHttpEventBus) -> Router {
    Router::new()
        .route("/__dbox_http/health", get(health))
        .route("/__dbox_http/commands/snapshot", post(snapshot))
        .route("/__dbox_http/commands/refresh", post(refresh))
        .route("/__dbox_http/commands/preview", post(preview))
        .route("/__dbox_http/commands/confirm", post(confirm))
        .route("/__dbox_http/commands/cancel", post(cancel))
        .route("/__dbox_http/commands/run_history", post(run_history))
        .route("/__dbox_http/commands/run_log", post(run_log))
        .route("/__dbox_http/commands/settings", post(settings))
        .route("/__dbox_http/commands/save_settings", post(save_settings))
        .route(
            "/__dbox_http/commands/validate_manifest",
            post(validate_manifest),
        )
        .route("/__dbox_http/commands/read_manifest", post(read_manifest))
        .route("/__dbox_http/commands/save_manifest", post(save_manifest))
        .route("/__dbox_http/events", get(event_stream))
        .with_state(DevHttpState {
            service,
            events: event_bus,
        })
}

pub async fn serve_dev_http(service: Arc<ApiService>, events: DevHttpEventBus) -> io::Result<()> {
    let listener = bind_dev_http_listener(DEV_HTTP_PORT)?;
    let address = listener.local_addr()?;
    let router = dev_http_router(service, events);
    let listener = tokio::net::TcpListener::from_std(listener)?;
    eprintln!("dbox Dev HTTP listening on http://{address}");
    axum::serve(listener, router).await
}

fn bind_dev_http_listener(port: u16) -> io::Result<std::net::TcpListener> {
    let listener = std::net::TcpListener::bind(dev_http_address(port)).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot bind dbox Dev HTTP to 127.0.0.1:{port}: {error}"),
        )
    })?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn dev_http_address(port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn snapshot(State(state): State<DevHttpState>) -> Json<CommandResponse<SnapshotDto>> {
    Json(state.service.snapshot().await.into())
}

async fn refresh(
    State(state): State<DevHttpState>,
    Json(request): Json<RefreshRequestDto>,
) -> Json<CommandResponse<SnapshotDto>> {
    Json(state.service.refresh(request).await.into())
}

async fn preview(
    State(state): State<DevHttpState>,
    Json(request): Json<PreviewRequestDto>,
) -> Json<CommandResponse<Vec<UpdatePlanDto>>> {
    Json(state.service.preview(request).await.into())
}

async fn confirm(
    State(state): State<DevHttpState>,
    Json(request): Json<ConfirmRequestDto>,
) -> Json<CommandResponse<ConfirmResponseDto>> {
    Json(state.service.confirm(request).await.into())
}

async fn cancel(
    State(state): State<DevHttpState>,
    Json(request): Json<CancelRequestDto>,
) -> Json<CommandResponse<CancelResponseDto>> {
    Json(state.service.cancel(request).await.into())
}

async fn run_history(State(state): State<DevHttpState>) -> Json<CommandResponse<RunHistoryDto>> {
    Json(state.service.run_history().into())
}

async fn run_log(
    State(state): State<DevHttpState>,
    Json(request): Json<RunLogRequestDto>,
) -> Json<CommandResponse<RunLogDto>> {
    Json(state.service.run_log(request).await.into())
}

async fn settings(State(state): State<DevHttpState>) -> Json<CommandResponse<SettingsDocumentDto>> {
    Json(state.service.settings().into())
}

async fn save_settings(
    State(state): State<DevHttpState>,
    Json(request): Json<SaveSettingsRequestDto>,
) -> Json<CommandResponse<SettingsDocumentDto>> {
    Json(state.service.save_settings(request).await.into())
}

async fn validate_manifest(
    State(state): State<DevHttpState>,
    Json(request): Json<ManifestInputDto>,
) -> Json<CommandResponse<ManifestValidationDto>> {
    Json(state.service.validate_manifest(request).into())
}

async fn read_manifest(
    State(state): State<DevHttpState>,
    Json(request): Json<ManifestReadRequestDto>,
) -> Json<CommandResponse<ManifestDocumentDto>> {
    Json(state.service.read_manifest(request).into())
}

async fn save_manifest(
    State(state): State<DevHttpState>,
    Json(request): Json<SaveManifestRequestDto>,
) -> Json<CommandResponse<SavedManifestDto>> {
    Json(state.service.save_manifest(request).await.into())
}

async fn event_stream(
    State(state): State<DevHttpState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe()).filter_map(|result| match result {
        Ok(event) => Some(Ok(sse_event(event))),
        // A lagging browser can recover by requesting a fresh snapshot.
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn sse_event(event: ApiEvent) -> Event {
    match event {
        ApiEvent::RefreshProgress(payload) => Event::default()
            .event("refresh-progress")
            .json_data(payload),
        ApiEvent::RunState(payload) => Event::default().event("run-state").json_data(payload),
        ApiEvent::RunOutput(payload) => Event::default().event("run-output").json_data(payload),
        ApiEvent::ToolState(payload) => Event::default().event("tool-state").json_data(payload),
    }
    .expect("API event DTOs are serializable")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use serde_json::{json, Value};
    use tempfile::TempDir;
    use tokio_stream::StreamExt as _;
    use tower::ServiceExt;

    use crate::application::{
        ApplicationEnvironment, ApplicationService, ApplicationServiceOptions,
    };
    use crate::catalog::Catalog;
    use crate::executor::CommandExecutor;
    use crate::persistence::{AppPaths, PersistenceStore};
    use crate::providers::ProviderRegistry;

    use super::super::events::{
        BufferedExecutionEventSink, RefreshPhaseDto, RefreshProgressEventDto, RunOutputChunkDto,
        RunOutputEventDto, RunOutputStreamDto, RunStateEventDto, RunStateKindDto,
        ToolStateEventDto,
    };
    use super::*;

    #[derive(Default)]
    struct FakeEnvironment;

    impl ApplicationEnvironment for FakeEnvironment {
        fn resolve_program(
            &self,
            name: &str,
            user_override: Option<&Path>,
            _now: chrono::DateTime<Utc>,
        ) -> Result<PathBuf, String> {
            user_override
                .map(Path::to_path_buf)
                .ok_or_else(|| format!("{name} is unavailable"))
        }

        fn revision(&self) -> String {
            "environment-revision".into()
        }
    }

    struct Fixture {
        _temporary: TempDir,
        service: Arc<ApiService>,
        events: DevHttpEventBus,
        router: Router,
    }

    impl Fixture {
        fn new() -> Self {
            let temporary = TempDir::new().unwrap();
            let store = Arc::new(PersistenceStore::new(AppPaths::new(
                temporary.path().join("config"),
                temporary.path().join("data"),
                temporary.path().join("logs"),
            )));
            let events = DevHttpEventBus::new();
            let emitter: Arc<dyn ApiEventEmitter> = Arc::new(events.clone());
            let event_sink = Arc::new(BufferedExecutionEventSink::new(
                Arc::clone(&emitter),
                32,
                Duration::from_millis(50),
            ));
            let application = Arc::new(
                ApplicationService::new(
                    Arc::new(ProviderRegistry::new()),
                    Catalog::default(),
                    Arc::clone(&store),
                    CommandExecutor::new(Some(Arc::clone(&store)), event_sink, 4096),
                    Arc::new(FakeEnvironment),
                    ApplicationServiceOptions::default(),
                )
                .unwrap(),
            );
            let service = Arc::new(ApiService::new(application, store, emitter));
            let router = dev_http_router(Arc::clone(&service), events.clone());
            Self {
                _temporary: temporary,
                service,
                events,
                router,
            }
        }
    }

    async fn post(router: &Router, path: &str, body: Value) -> axum::response::Response {
        router
            .clone()
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn listener_is_always_bound_to_ipv4_loopback() {
        assert_eq!(*dev_http_address(DEV_HTTP_PORT).ip(), Ipv4Addr::LOCALHOST);
        assert_eq!(dev_http_address(DEV_HTTP_PORT), DEV_HTTP_ADDR);
    }

    #[test]
    fn no_sse_subscribers_is_not_an_emitter_error() {
        let events = DevHttpEventBus::new();
        assert!(events
            .emit(ApiEvent::ToolState(ToolStateEventDto {
                sequence: "1".into(),
                state_revision: "state".into(),
                tool_ids: vec![],
            }))
            .is_ok());
    }

    #[tokio::test]
    async fn router_exposes_every_typed_command_with_the_shared_envelope() {
        let fixture = Fixture::new();
        let settings = fixture.service.settings().unwrap();
        let manifest =
            "schema_version = 1\nid = \"dev-http-test\"\ndisplay_name = \"Dev HTTP Test\"\n";
        let cases = [
            ("snapshot", json!({}), "ok"),
            (
                "refresh",
                json!({ "scope": { "scope": "all" }, "force": true }),
                "ok",
            ),
            ("preview", json!({ "selections": [] }), "error"),
            (
                "confirm",
                json!({ "planId": "invalid", "planHash": "invalid" }),
                "error",
            ),
            ("cancel", json!({ "runId": "invalid run" }), "error"),
            ("run_history", json!({}), "ok"),
            ("run_log", json!({ "runId": "invalid run" }), "error"),
            ("settings", json!({}), "ok"),
            (
                "save_settings",
                serde_json::to_value(SaveSettingsRequestDto {
                    expected_revision: settings.revision,
                    settings: settings.settings,
                })
                .unwrap(),
                "ok",
            ),
            (
                "validate_manifest",
                json!({ "fileName": "dev-http-test.toml", "contents": manifest }),
                "ok",
            ),
            ("read_manifest", json!({ "fileName": "missing.toml" }), "ok"),
            (
                "save_manifest",
                json!({
                    "fileName": "dev-http-test.toml",
                    "contents": manifest,
                    "expectedRevision": null
                }),
                "ok",
            ),
        ];

        for (name, request, expected_status) in cases {
            let response = post(
                &fixture.router,
                &format!("/__dbox_http/commands/{name}"),
                request,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "route {name}");
            let envelope = response_json(response).await;
            assert_eq!(envelope["status"], expected_status, "route {name}");
            assert!(
                envelope
                    .get(if expected_status == "ok" {
                        "data"
                    } else {
                        "error"
                    })
                    .is_some(),
                "route {name}"
            );
        }
    }

    #[tokio::test]
    async fn snapshot_http_payload_matches_direct_service_serialization() {
        let fixture = Fixture::new();
        let direct = serde_json::to_value(fixture.service.snapshot().await.unwrap()).unwrap();
        let response = post(&fixture.router, "/__dbox_http/commands/snapshot", json!({})).await;
        let envelope = response_json(response).await;
        assert_eq!(envelope["data"], direct);
    }

    #[tokio::test]
    async fn malformed_json_and_unknown_routes_use_http_errors() {
        let fixture = Fixture::new();
        let health = fixture
            .router
            .clone()
            .oneshot(
                Request::get("/__dbox_http/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);
        assert_eq!(response_json(health).await, json!({ "status": "ok" }));

        let malformed = fixture
            .router
            .clone()
            .oneshot(
                Request::post("/__dbox_http/commands/refresh")
                    .header("content-type", "application/json")
                    .body(Body::from("{"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(malformed.status().is_client_error());

        let missing = fixture
            .router
            .clone()
            .oneshot(
                Request::get("/__dbox_http/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn sse_preserves_all_event_names_and_json_payloads() {
        let fixture = Fixture::new();
        let response = fixture
            .router
            .clone()
            .oneshot(
                Request::get("/__dbox_http/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        let mut body = response.into_body().into_data_stream();
        assert_eq!(fixture.events.sender.receiver_count(), 1);

        let cases = [
            (
                ApiEvent::RefreshProgress(RefreshProgressEventDto {
                    request_id: "request".into(),
                    sequence: "1".into(),
                    phase: RefreshPhaseDto::Progress,
                    message: "checking".into(),
                    provider_id: Some("provider".into()),
                    completed: Some(1),
                    total: Some(2),
                }),
                "refresh-progress",
                "\"requestId\":\"request\"",
            ),
            (
                ApiEvent::RunState(RunStateEventDto {
                    run_id: "run".into(),
                    sequence: "2".into(),
                    timestamp: "2026-08-28T00:00:00Z".into(),
                    state: RunStateKindDto::StateChanged {
                        status: "running".into(),
                    },
                }),
                "run-state",
                "\"runId\":\"run\"",
            ),
            (
                ApiEvent::RunOutput(RunOutputEventDto {
                    run_id: "run".into(),
                    first_sequence: "3".into(),
                    last_sequence: "3".into(),
                    timestamp: "2026-08-28T00:00:00Z".into(),
                    chunks: vec![RunOutputChunkDto {
                        sequence: "3".into(),
                        stream: RunOutputStreamDto::Stdout,
                        message: "hello".into(),
                    }],
                }),
                "run-output",
                "\"message\":\"hello\"",
            ),
            (
                ApiEvent::ToolState(ToolStateEventDto {
                    sequence: "4".into(),
                    state_revision: "state".into(),
                    tool_ids: vec!["tool".into()],
                }),
                "tool-state",
                "\"stateRevision\":\"state\"",
            ),
        ];

        for (event, name, payload_fragment) in cases {
            fixture.events.emit(event).unwrap();
            let chunk = tokio::time::timeout(Duration::from_secs(1), body.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let chunk = std::str::from_utf8(&chunk).unwrap();
            assert!(chunk.contains(&format!("event: {name}")), "{chunk}");
            assert!(chunk.contains(payload_fragment), "{chunk}");
        }

        drop(body);
        assert_eq!(fixture.events.sender.receiver_count(), 0);
        assert!(fixture
            .events
            .emit(ApiEvent::ToolState(ToolStateEventDto {
                sequence: "5".into(),
                state_revision: "state".into(),
                tool_ids: vec![],
            }))
            .is_ok());
    }
}
