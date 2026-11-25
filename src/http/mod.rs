pub mod areas;
pub mod auth;
pub mod automations;
pub mod dashboard;
pub mod entities;
pub mod events;
pub mod openapi;
pub mod skills;
pub mod users;
pub mod webhooks;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::routing::{any, delete, get, post, put};
use axum::{Json, Router};
use serde::Serialize;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing::warn;

use crate::auth::AuthService;
use crate::automation::{ActionExecutor, AutomationEngine};
use crate::config::{AreasConfig, ConfigStore, DevicesConfig, SecretsConfig, WebhookConfig};
use crate::http::webhooks::{MAX_WEBHOOK_BODY_BYTES, handle_webhook};
use crate::skills::SkillBridge;
use crate::state::{EventBus, StateStore};
use crate::storage::HistoryStore;

#[derive(Clone)]
pub struct AppState {
    pub config_store: ConfigStore,
    pub areas: Arc<RwLock<AreasConfig>>,
    pub devices: Arc<RwLock<DevicesConfig>>,
    pub secrets: Arc<RwLock<SecretsConfig>>,
    pub history: Arc<dyn HistoryStore>,
    pub state: StateStore,
    pub events: EventBus,
    pub webhooks: Arc<Vec<WebhookConfig>>,
    pub skills: Option<SkillBridge>,
    pub automations: Option<AutomationEngine>,
    pub executor: Arc<dyn ActionExecutor>,
    pub auth: AuthService,
}

#[derive(Debug, Serialize)]
struct LivenessResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct ReadinessResponse {
    status: &'static str,
    storage: &'static str,
    runtime: &'static str,
}

/// Build the `axum` HTTP router with health probes, auth/RBAC, full REST API, SSE, and OpenAPI docs.
pub fn build_router(state: AppState) -> Router {
    build_router_with_spa(state, None)
}

/// Build the `axum` HTTP router with optional static SolidJS SPA serving (`web_dist_dir`).
///
/// Unknown `/api/*` paths always return JSON `404 {"error":"not_found"}` rather than `index.html`.
pub fn build_router_with_spa(state: AppState, web_dist_dir: Option<PathBuf>) -> Router {
    let webhook_route = Router::new()
        .route("/api/webhooks/{webhook_id}", post(handle_webhook))
        .layer(DefaultBodyLimit::max(MAX_WEBHOOK_BODY_BYTES));

    let mut router = Router::new()
        .route("/health", get(health))
        .route("/health/ready", get(health_ready))
        .route("/docs", get(openapi::docs_html))
        .route("/api/openapi.json", get(openapi::openapi_json))
        // Auth
        .route("/api/auth/bootstrap", post(auth::bootstrap))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        // Areas
        .route(
            "/api/areas",
            get(areas::list_areas).post(areas::create_area),
        )
        .route(
            "/api/areas/{area_id}",
            put(areas::update_area).delete(areas::delete_area),
        )
        // Dashboard Layout
        .route(
            "/api/dashboard",
            get(dashboard::get_dashboard).put(dashboard::update_dashboard),
        )
        // Entities & History
        .route("/api/entities", get(entities::list_entities))
        .route("/api/entities/{entity_id}", get(entities::get_entity))
        .route(
            "/api/entities/{entity_id}/command",
            post(entities::command_entity),
        )
        .route(
            "/api/entities/{entity_id}/history",
            get(entities::entity_history),
        )
        // Skills (Admin)
        .route("/api/skills", get(skills::list_skills))
        .route("/api/skills/{skill_id}", get(skills::get_skill))
        .route(
            "/api/skills/{skill_id}/config",
            put(skills::update_skill_config),
        )
        .route("/api/skills/{skill_id}/start", post(skills::start_skill))
        .route("/api/skills/{skill_id}/stop", post(skills::stop_skill))
        .route(
            "/api/skills/{skill_id}/restart",
            post(skills::restart_skill),
        )
        .route(
            "/api/skills/{skill_id}/reinstall",
            post(skills::reinstall_skill),
        )
        .route("/api/skills/{skill_id}/logs", get(skills::get_skill_logs))
        .route("/api/skills/{skill_id}/invoke", post(skills::invoke_skill))
        // Automations (Admin)
        .route(
            "/api/automations",
            get(automations::list_automations).post(automations::create_automation),
        )
        .route(
            "/api/automations/runs",
            get(automations::list_automation_runs),
        )
        .route(
            "/api/automations/reload",
            post(automations::reload_automations),
        )
        .route(
            "/api/automations/{id}",
            get(automations::get_automation)
                .put(automations::update_automation)
                .delete(automations::delete_automation),
        )
        .route(
            "/api/automations/{id}/toggle",
            post(automations::toggle_automation),
        )
        .route(
            "/api/automations/{id}/trigger",
            post(automations::trigger_automation),
        )
        // Users & API Tokens (Admin)
        .route(
            "/api/users",
            get(users::list_users).post(users::create_user),
        )
        .route(
            "/api/users/{user_id}",
            put(users::update_user).delete(users::delete_user),
        )
        .route("/api/users/{user_id}/tokens", post(users::mint_token))
        .route(
            "/api/users/{user_id}/tokens/{token_id}",
            delete(users::revoke_token),
        )
        // Live Events (SSE, RBAC-filtered)
        .route("/api/events", get(events::sse_events))
        .merge(webhook_route)
        // Ensure unknown /api/* routes return JSON 404 rather than SPA index.html
        .route("/api", any(api_not_found))
        .route("/api/", any(api_not_found))
        .route("/api/{*rest}", any(api_not_found));

    if let Some(dist_dir) = web_dist_dir {
        let index_html = dist_dir.join("index.html");
        if index_html.is_file() {
            // Serve /assets strictly without index.html fallback so missing hashed chunks return 404.
            router = router.nest_service("/assets", ServeDir::new(dist_dir.join("assets")));
            let spa_service = ServeDir::new(&dist_dir).fallback(ServeFile::new(index_html));
            router = router.fallback_service(spa_service);
        }
    }

    router.layer(TraceLayer::new_for_http()).with_state(state)
}

async fn api_not_found() -> axum::response::Response {
    auth::api_error(StatusCode::NOT_FOUND, "not_found", None)
}

async fn health() -> Json<LivenessResponse> {
    Json(LivenessResponse { status: "ok" })
}

async fn health_ready(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    let runtime = match &state.skills {
        Some(bridge) if bridge.is_connected() => "ok",
        Some(_) => "disconnected",
        None => "disabled",
    };

    match state.history.health_check().await {
        Ok(()) => (
            StatusCode::OK,
            Json(ReadinessResponse {
                status: "ok",
                storage: "ok",
                runtime,
            }),
        ),
        Err(err) => {
            warn!(error = %err, "storage health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReadinessResponse {
                    status: "degraded",
                    storage: "error",
                    runtime,
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::http::webhooks::hash_webhook_secret;
    use crate::state::CoreEvent;
    use crate::storage::{
        AutomationRunRecord, EntityHistorySeries, SqliteStore, StateHistoryEntry, StorageError,
        TimeRange,
    };

    struct FailingStore;

    #[async_trait]
    impl HistoryStore for FailingStore {
        async fn health_check(&self) -> Result<(), StorageError> {
            Err(StorageError::Connection("simulated outage".into()))
        }

        async fn record_state_batch(
            &self,
            _records: &[StateHistoryEntry],
        ) -> Result<(), StorageError> {
            unimplemented!()
        }

        async fn record_automation_run(
            &self,
            _run: &AutomationRunRecord,
        ) -> Result<(), StorageError> {
            unimplemented!()
        }

        async fn query_entity_history(
            &self,
            _entity_id: &str,
            _range: TimeRange,
            _bucket_seconds: Option<u32>,
        ) -> Result<EntityHistorySeries, StorageError> {
            unimplemented!()
        }

        async fn query_automation_runs(
            &self,
            _automation_id: Option<&str>,
            _limit: u32,
            _offset: u32,
        ) -> Result<Vec<AutomationRunRecord>, StorageError> {
            unimplemented!()
        }

        async fn prune_before(&self, _cutoff_unix_ms: i64) -> Result<u64, StorageError> {
            unimplemented!()
        }
    }

    use std::sync::Mutex as StdMutex;
    use tokio::time::{Duration, timeout};

    use crate::automation::AutomationEngine;
    use crate::config::LocationConfig;
    use crate::protocol::domain::{
        Availability, DomainState, EntityRecord, EntitySource, LockState,
    };
    use crate::protocol::skill::SkillStatus;
    use crate::storage::{AutomationOutcome, HistoryEntryKind};

    type RecordedCommand = (String, String, serde_json::Map<String, Value>);

    struct TestExecutor {
        commands: StdMutex<Vec<RecordedCommand>>,
    }

    impl TestExecutor {
        fn new() -> Self {
            Self {
                commands: StdMutex::new(Vec::new()),
            }
        }

        fn recorded(&self) -> Vec<RecordedCommand> {
            self.commands.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl ActionExecutor for TestExecutor {
        async fn call_entity(
            &self,
            entity_id: &str,
            action: &str,
            params: serde_json::Map<String, Value>,
        ) -> Result<Value, String> {
            self.commands.lock().expect("lock").push((
                entity_id.to_owned(),
                action.to_owned(),
                params,
            ));
            Ok(json!({ "ok": true }))
        }

        async fn invoke_skill(
            &self,
            _skill_id: &str,
            _action: &str,
            _params: serde_json::Map<String, Value>,
            _context: serde_json::Map<String, Value>,
            _timeout: Duration,
        ) -> Result<Value, String> {
            Ok(json!({ "ok": true }))
        }
    }

    fn make_entity(
        entity_id: &str,
        name: &str,
        area_id: Option<&str>,
        state: DomainState,
    ) -> EntityRecord {
        EntityRecord {
            entity_id: entity_id.to_owned(),
            device_id: None,
            area_id: area_id.map(str::to_owned),
            name: name.to_owned(),
            source: EntitySource::EspHome {
                node_id: "node1".into(),
            },
            availability: Availability::Online,
            state,
            capabilities: vec![],
            extra: serde_json::Map::new(),
            custom_actions: vec![],
            last_changed_at: 0,
            last_updated_at: 0,
        }
    }

    async fn json_body(response: axum::response::Response) -> Value {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("parse json")
    }

    struct TestFixture {
        _dir: tempfile::TempDir,
        _shutdown_tx: tokio::sync::watch::Sender<bool>,
        state: AppState,
        executor: Arc<TestExecutor>,
    }

    async fn test_fixture(
        history: Arc<dyn HistoryStore>,
        webhooks: Vec<WebhookConfig>,
        with_automations: bool,
    ) -> TestFixture {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let config_store = ConfigStore::new(dir.path());
        let events = EventBus::new();
        let state_store = StateStore::new(events.clone());
        let executor = Arc::new(TestExecutor::new());
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let automations = if with_automations {
            let auto_dir = dir.path().join("automations");
            std::fs::create_dir_all(&auto_dir).expect("create automations dir");
            let (engine, _errors) = AutomationEngine::start(
                auto_dir,
                LocationConfig {
                    latitude: Some(39.9),
                    longitude: Some(116.4),
                    ..LocationConfig::default()
                },
                state_store.clone(),
                events.clone(),
                history.clone(),
                executor.clone(),
                shutdown_rx,
            );
            Some(engine)
        } else {
            None
        };
        let auth = AuthService::new(config_store.clone(), crate::config::UsersConfig::default());
        let app_state = AppState {
            config_store,
            areas: Arc::new(RwLock::new(AreasConfig::default())),
            devices: Arc::new(RwLock::new(DevicesConfig::default())),
            secrets: Arc::new(RwLock::new(SecretsConfig::default())),
            history,
            state: state_store,
            events,
            webhooks: Arc::new(webhooks),
            skills: None,
            automations,
            executor: executor.clone(),
            auth,
        };
        TestFixture {
            _dir: dir,
            _shutdown_tx: shutdown_tx,
            state: app_state,
            executor,
        }
    }

    async fn bootstrap_admin_cookie(app: &Router) -> String {
        let boot = app
            .clone()
            .oneshot(
                Request::post("/api/auth/bootstrap")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"password123","display_name":"Admin"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(boot.status(), StatusCode::CREATED);
        boot.headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("Set-Cookie")
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }

    async fn create_user_and_token(
        app: &Router,
        admin_cookie: &str,
        username: &str,
        role: &str,
        allowed_areas: &[&str],
    ) -> String {
        let payload = json!({
            "username": username,
            "display_name": username,
            "password": "password123",
            "role": role,
            "allowed_areas": allowed_areas,
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/users")
                    .header("cookie", admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let user_id = json_body(resp).await["user"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let mint = app
            .clone()
            .oneshot(
                Request::post(format!("/api/users/{user_id}/tokens"))
                    .header("cookie", admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mint.status(), StatusCode::CREATED);
        json_body(mint).await["token"].as_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn health_and_ready_return_200_with_healthy_store() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], false).await;
        let app = build_router(fixture.state);

        let resp = app
            .clone()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(json_body(resp).await["status"], "ok");

        let ready = app
            .oneshot(Request::get("/health/ready").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::OK);
        let body = json_body(ready).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["storage"], "ok");
    }

    #[tokio::test]
    async fn health_ready_returns_503_when_storage_fails() {
        let fixture = test_fixture(Arc::new(FailingStore), vec![], false).await;
        let app = build_router(fixture.state);

        let ready = app
            .oneshot(Request::get("/health/ready").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = json_body(ready).await;
        assert_eq!(body["status"], "degraded");
        assert_eq!(body["storage"], "error");
    }

    #[tokio::test]
    async fn webhook_accepts_bearer_and_header_secret_and_publishes_sanitized_event() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let secret = "my-super-secret-webhook-token";
        // Independently computed via `printf '%s' 'my-super-secret-webhook-token' | sha256sum`.
        let expected_hex = "bddbaa946a00f5f1c2add380d6eac35745b05f011dfd408f619d43c48ec1a97b";
        let expected_hash = format!("sha256:{expected_hex}");
        assert_eq!(hash_webhook_secret(secret), expected_hash);

        let webhooks = vec![
            WebhookConfig {
                id: "ios_arriving".into(),
                name: Some("iOS Shortcuts".into()),
                secret_hash: expected_hash,
                enabled: true,
            },
            WebhookConfig {
                id: "raw_hex_hook".into(),
                name: None,
                secret_hash: expected_hex.to_ascii_uppercase(),
                enabled: true,
            },
            WebhookConfig {
                id: "disabled_hook".into(),
                name: None,
                secret_hash: format!("sha256:{expected_hex}"),
                enabled: false,
            },
        ];

        let fixture = test_fixture(store, webhooks, false).await;
        let mut rx = fixture.state.events.subscribe();
        let app = build_router(fixture.state);

        // 1. Bearer auth + JSON body -> 202 + WebhookReceived with auth headers stripped
        let req = Request::post("/api/webhooks/ios_arriving")
            .header("authorization", format!("Bearer {secret}"))
            .header("x-custom-source", "iphone")
            .header("cookie", "xiaohe_session=sensitive")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"person":"alice","distance_m":15}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        assert_eq!(
            json_body(resp).await,
            json!({ "accepted": true, "webhook_id": "ios_arriving" })
        );

        let ev = rx.recv().await.expect("webhook event");
        match ev {
            CoreEvent::WebhookReceived {
                webhook_id,
                payload,
                headers,
                at,
            } => {
                assert_eq!(webhook_id, "ios_arriving");
                assert_eq!(payload["person"], "alice");
                assert_eq!(payload["distance_m"], 15);
                assert_eq!(
                    headers.get("x-custom-source").map(String::as_str),
                    Some("iphone")
                );
                assert!(!headers.contains_key("authorization"));
                assert!(!headers.contains_key("cookie"));
                assert!(at > 0);
            }
            other => panic!("expected WebhookReceived, got {other:?}"),
        }

        // 2. X-Webhook-Secret + empty body -> 202 + Value::Null payload
        let req2 = Request::post("/api/webhooks/ios_arriving")
            .header("x-webhook-secret", secret)
            .body(Body::empty())
            .unwrap();
        let resp2 = app.clone().oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::ACCEPTED);
        let ev2 = rx.recv().await.expect("webhook event 2");
        match ev2 {
            CoreEvent::WebhookReceived {
                payload, headers, ..
            } => {
                assert_eq!(payload, Value::Null);
                assert!(!headers.contains_key("x-webhook-secret"));
            }
            other => panic!("expected WebhookReceived, got {other:?}"),
        }

        // 2b. Raw uppercase 64-hex secret_hash without `sha256:` prefix -> 202
        let req2b = Request::post("/api/webhooks/raw_hex_hook")
            .header("x-webhook-secret", secret)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(req2b).await.unwrap().status(),
            StatusCode::ACCEPTED
        );
        let _ = rx.recv().await.expect("webhook event 2b");

        // 3. Invalid secret -> 401
        let bad_auth = Request::post("/api/webhooks/ios_arriving")
            .header("x-webhook-secret", "wrong-secret")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(bad_auth).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );

        // 4. Unknown or disabled webhook -> 404
        let unknown = Request::post("/api/webhooks/does_not_exist")
            .header("x-webhook-secret", secret)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(unknown).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );

        let disabled = Request::post("/api/webhooks/disabled_hook")
            .header("x-webhook-secret", secret)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(disabled).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );

        // 5. Malformed JSON -> 400
        let bad_json = Request::post("/api/webhooks/ios_arriving")
            .header("x-webhook-secret", secret)
            .body(Body::from("{not valid json"))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(bad_json).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );

        // 6. Body > 64 KB -> 413 Payload Too Large
        let too_large = Request::post("/api/webhooks/ios_arriving")
            .header("x-webhook-secret", secret)
            .body(Body::from(vec![b' '; MAX_WEBHOOK_BODY_BYTES + 1]))
            .unwrap();
        assert_eq!(
            app.oneshot(too_large).await.unwrap().status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[tokio::test]
    async fn auth_bootstrap_login_tokens_and_rbac_enforcement() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], false).await;
        let app = build_router(fixture.state);

        // 1. Before bootstrap: /api/auth/me reports initialized=false, user=null
        let me0 = app
            .clone()
            .oneshot(Request::get("/api/auth/me").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(me0.status(), StatusCode::OK);
        let me0_body = json_body(me0).await;
        assert_eq!(me0_body["initialized"], false);
        assert!(me0_body["user"].is_null());

        // 2. Unauthenticated access to /api/users -> 401
        let unauth = app
            .clone()
            .oneshot(Request::get("/api/users").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        // 3. Bootstrap first Admin -> 201 + Set-Cookie
        let boot = app
            .clone()
            .oneshot(
                Request::post("/api/auth/bootstrap")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"password123","display_name":"Admin"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(boot.status(), StatusCode::CREATED);
        let cookie_header = boot
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("Set-Cookie present")
            .to_str()
            .unwrap()
            .to_owned();
        assert!(cookie_header.contains("xiaohe_session="));
        assert!(cookie_header.contains("HttpOnly"));
        let session_cookie = cookie_header.split(';').next().unwrap().to_owned();
        let boot_body = json_body(boot).await;
        let admin_id = boot_body["user"]["id"].as_str().unwrap().to_owned();

        // Second bootstrap -> 409 Conflict
        let boot2 = app
            .clone()
            .oneshot(
                Request::post("/api/auth/bootstrap")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin2","password":"password123","display_name":"Admin 2"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(boot2.status(), StatusCode::CONFLICT);

        // 4. Admin creates a Member user with restricted allowed_areas
        let create_member = app
            .clone()
            .oneshot(
                Request::post("/api/users")
                    .header("cookie", &session_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"alice","display_name":"Alice","password":"password123","role":"member","allowed_areas":["living_room"]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_member.status(), StatusCode::CREATED);
        let member_body = json_body(create_member).await;
        let member_id = member_body["user"]["id"].as_str().unwrap().to_owned();
        assert_eq!(member_body["user"]["role"], "member");
        assert!(member_body["user"].get("password_hash").is_none());

        // 5. Mint Bearer token for Member -> Member calling Admin route /api/users gets 403
        let mint = app
            .clone()
            .oneshot(
                Request::post(format!("/api/users/{member_id}/tokens"))
                    .header("cookie", &session_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"phone"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mint.status(), StatusCode::CREATED);
        let mint_body = json_body(mint).await;
        let bearer = mint_body["token"].as_str().unwrap().to_owned();
        let token_id = mint_body["id"].as_str().unwrap().to_owned();

        let forbidden = app
            .clone()
            .oneshot(
                Request::get("/api/users")
                    .header("authorization", format!("Bearer {bearer}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

        // Member /api/auth/me succeeds with Bearer token
        let me_member = app
            .clone()
            .oneshot(
                Request::get("/api/auth/me")
                    .header("authorization", format!("Bearer {bearer}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me_member.status(), StatusCode::OK);
        assert_eq!(json_body(me_member).await["user"]["username"], "alice");

        // Revoke token -> Bearer token no longer authenticates
        let revoke = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/users/{member_id}/tokens/{token_id}"))
                    .header("cookie", &session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::NO_CONTENT);

        let after_revoke = app
            .clone()
            .oneshot(
                Request::get("/api/users")
                    .header("authorization", format!("Bearer {bearer}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(after_revoke.status(), StatusCode::UNAUTHORIZED);

        // 6. Deleting last remaining admin -> 409 Conflict
        let del_admin = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/users/{admin_id}"))
                    .header("cookie", &session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_admin.status(), StatusCode::CONFLICT);

        // 7. Logout invalidates session cookie
        let logout = app
            .clone()
            .oneshot(
                Request::post("/api/auth/logout")
                    .header("cookie", &session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);

        let after_logout = app
            .oneshot(
                Request::get("/api/users")
                    .header("cookie", &session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(after_logout.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn areas_entities_commands_and_history_enforce_rbac() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        store
            .record_state_batch(&[StateHistoryEntry {
                entity_id: "light.living".into(),
                ts: 1_700_000_000_000,
                kind: HistoryEntryKind::Discrete {
                    state: "on".into(),
                    attributes: Some(json!({"on": true})),
                },
            }])
            .await
            .expect("seed history");

        let fixture = test_fixture(store, vec![], false).await;
        // Register 3 entities across living_room and bedroom
        fixture.state.state.upsert_entity(make_entity(
            "light.living",
            "Living Light",
            Some("living_room"),
            DomainState::Light {
                on: true,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
        ));
        fixture.state.state.upsert_entity(make_entity(
            "lock.living",
            "Front Door",
            Some("living_room"),
            DomainState::Lock {
                state: LockState::Locked,
            },
        ));
        fixture.state.state.upsert_entity(make_entity(
            "light.bedroom",
            "Bedroom Light",
            Some("bedroom"),
            DomainState::Light {
                on: false,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
        ));

        let executor = fixture.executor.clone();
        let app = build_router(fixture.state);
        let admin_cookie = bootstrap_admin_cookie(&app).await;
        let guest_token =
            create_user_and_token(&app, &admin_cookie, "guest1", "guest", &["living_room"]).await;

        // Admin creates two areas
        for (area_id, name) in [("living_room", "Living Room"), ("bedroom", "Bedroom")] {
            let r = app
                .clone()
                .oneshot(
                    Request::post("/api/areas")
                        .header("cookie", &admin_cookie)
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({"area_id": area_id, "name": name}).to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::CREATED);
        }

        // Guest sees only living_room in /api/areas (with entity_count = 2)
        let guest_areas = app
            .clone()
            .oneshot(
                Request::get("/api/areas")
                    .header("authorization", format!("Bearer {guest_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(guest_areas.status(), StatusCode::OK);
        let areas_json = json_body(guest_areas).await;
        assert_eq!(areas_json["areas"].as_array().unwrap().len(), 1);
        assert_eq!(areas_json["areas"][0]["area_id"], "living_room");
        assert_eq!(areas_json["areas"][0]["entity_count"], 2);

        // Guest cannot create areas -> 403
        let guest_create = app
            .clone()
            .oneshot(
                Request::post("/api/areas")
                    .header("authorization", format!("Bearer {guest_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"area_id":"kitchen","name":"Kitchen"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(guest_create.status(), StatusCode::FORBIDDEN);

        // Guest sees only living_room entities in /api/entities
        let guest_ents = app
            .clone()
            .oneshot(
                Request::get("/api/entities")
                    .header("authorization", format!("Bearer {guest_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(guest_ents.status(), StatusCode::OK);
        let ents_json = json_body(guest_ents).await;
        assert_eq!(ents_json["entities"].as_array().unwrap().len(), 2);

        // Guest GET /api/entities/light.bedroom -> 403
        let bedroom_get = app
            .clone()
            .oneshot(
                Request::get("/api/entities/light.bedroom")
                    .header("authorization", format!("Bearer {guest_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bedroom_get.status(), StatusCode::FORBIDDEN);

        // Guest POST command on lock.living -> 403 (guests cannot control locks)
        let guest_lock = app
            .clone()
            .oneshot(
                Request::post("/api/entities/lock.living/command")
                    .header("authorization", format!("Bearer {guest_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"unlock"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(guest_lock.status(), StatusCode::FORBIDDEN);

        // Guest POST command on light.living -> 200 and recorded by executor
        let guest_light = app
            .clone()
            .oneshot(
                Request::post("/api/entities/light.living/command")
                    .header("authorization", format!("Bearer {guest_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"action":"turn_off","params":{"transition":1}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(guest_light.status(), StatusCode::OK);
        assert_eq!(executor.recorded().len(), 1);
        assert_eq!(executor.recorded()[0].0, "light.living");
        assert_eq!(executor.recorded()[0].1, "turn_off");

        // Guest GET history for light.living -> 200 with 1 transition
        let hist = app
            .clone()
            .oneshot(
                Request::get(
                    "/api/entities/light.living/history?from=1699999000000&to=1700001000000",
                )
                .header("authorization", format!("Bearer {guest_token}"))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hist.status(), StatusCode::OK);
        let hist_json = json_body(hist).await;
        assert_eq!(hist_json["entity_id"], "light.living");
        assert_eq!(hist_json["transitions"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn automations_crud_validation_toggle_trigger_and_runs() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], true).await;
        let executor = fixture.executor.clone();
        let app = build_router(fixture.state);
        let admin_cookie = bootstrap_admin_cookie(&app).await;

        let valid_toml = r#"
id = "auto_test"
name = "Test Rule"
enabled = true

[[triggers]]
type = "webhook"
webhook_id = "hook1"

[[actions]]
type = "entity.call"
entity_id = "light.living"
action = "turn_on"
"#;

        // 1. Invalid TOML -> 422
        let bad = app
            .clone()
            .oneshot(
                Request::post("/api/automations")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"raw_toml": "id = \"bad\"\n"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // 2. Create valid automation -> 201
        let created = app
            .clone()
            .oneshot(
                Request::post("/api/automations")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"raw_toml": valid_toml}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        assert_eq!(json_body(created).await["id"], "auto_test");

        // 3. Trigger manually -> 200 and executes action asynchronously
        let trig = app
            .clone()
            .oneshot(
                Request::post("/api/automations/auto_test/trigger")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"skip_conditions":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(trig.status(), StatusCode::OK);
        for _ in 0..50 {
            if executor.recorded().len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(executor.recorded().len(), 1);

        // 4. Query runs -> includes our manual run
        let mut run_count = 0;
        for _ in 0..50 {
            let runs = app
                .clone()
                .oneshot(
                    Request::get("/api/automations/runs?automation_id=auto_test")
                        .header("cookie", &admin_cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(runs.status(), StatusCode::OK);
            run_count = json_body(runs).await["runs"].as_array().unwrap().len();
            if run_count == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(run_count, 1);

        // 5. Toggle disabled -> 200
        let toggle = app
            .clone()
            .oneshot(
                Request::post("/api/automations/auto_test/toggle")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"enabled":false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(toggle.status(), StatusCode::OK);
        assert_eq!(json_body(toggle).await["enabled"], false);

        // 6. Delete -> 204
        let del = app
            .clone()
            .oneshot(
                Request::delete("/api/automations/auto_test")
                    .header("cookie", &admin_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn sse_events_filters_by_rbac_and_openapi_serves_spec_and_docs() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], false).await;
        let events = fixture.state.events.clone();
        fixture.state.state.upsert_entity(make_entity(
            "light.living",
            "Living Light",
            Some("living_room"),
            DomainState::Light {
                on: false,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
        ));
        fixture.state.state.upsert_entity(make_entity(
            "light.bedroom",
            "Bedroom Light",
            Some("bedroom"),
            DomainState::Light {
                on: false,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
        ));

        let app = build_router(fixture.state);
        let admin_cookie = bootstrap_admin_cookie(&app).await;
        let member_token =
            create_user_and_token(&app, &admin_cookie, "alice", "member", &["living_room"]).await;

        // 1. OpenAPI 3.1 JSON + /docs HTML
        let spec_resp = app
            .clone()
            .oneshot(
                Request::get("/api/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(spec_resp.status(), StatusCode::OK);
        let spec = json_body(spec_resp).await;
        assert!(spec["openapi"].as_str().unwrap().starts_with("3.1"));
        assert!(spec["paths"]["/api/entities"].is_object());
        assert!(spec["paths"]["/api/events"].is_object());

        let docs_resp = app
            .clone()
            .oneshot(Request::get("/docs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(docs_resp.status(), StatusCode::OK);

        // 2. Unauthenticated SSE -> 401
        let unauth_sse = app
            .clone()
            .oneshot(Request::get("/api/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauth_sse.status(), StatusCode::UNAUTHORIZED);

        // 3. Member SSE receives ONLY living_room entity_state, NOT bedroom, NOT skill_status, NOT automation_run
        let member_sse = app
            .clone()
            .oneshot(
                Request::get("/api/events")
                    .header("authorization", format!("Bearer {member_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(member_sse.status(), StatusCode::OK);
        let mut body = member_sse.into_body();

        // Publish forbidden events first, then an allowed living_room event
        events.publish(CoreEvent::EntityStateChanged {
            entity_id: "light.bedroom".into(),
            area_id: Some("bedroom".into()),
            old_state: None,
            new_state: DomainState::Light {
                on: true,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
            extra: serde_json::Map::new(),
            at: 1,
        });
        events.publish(CoreEvent::SkillStatusChanged {
            skill_id: "miio".into(),
            status: SkillStatus::Running,
            message: None,
            at: 2,
        });
        events.publish(CoreEvent::AutomationRunRecorded {
            automation_id: "a1".into(),
            run_id: "r1".into(),
            outcome: AutomationOutcome::Succeeded,
            at: 3,
        });
        events.publish(CoreEvent::EntityStateChanged {
            entity_id: "light.living".into(),
            area_id: Some("living_room".into()),
            old_state: None,
            new_state: DomainState::Light {
                on: true,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
            extra: serde_json::Map::new(),
            at: 4,
        });

        let frame = timeout(Duration::from_secs(2), body.frame())
            .await
            .expect("sse frame within timeout")
            .expect("some frame")
            .expect("ok frame");
        let chunk = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
        assert!(
            chunk.contains("event: entity_state"),
            "expected entity_state event, got: {chunk}"
        );
        assert!(
            chunk.contains("light.living"),
            "expected light.living, got: {chunk}"
        );
        assert!(
            !chunk.contains("light.bedroom") && !chunk.contains("miio") && !chunk.contains("a1"),
            "forbidden event leaked to member SSE: {chunk}"
        );
    }

    #[tokio::test]
    async fn regression_sse_live_area_revocation_and_demotion_terminates_stream() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], false).await;
        let events = fixture.state.events.clone();
        let app = build_router(fixture.state);
        let admin_cookie = bootstrap_admin_cookie(&app).await;

        // Create member alice with access to both living_room and bedroom
        let create_alice = app
            .clone()
            .oneshot(
                Request::post("/api/users")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"alice","display_name":"Alice","password":"password123","role":"member","allowed_areas":["living_room","bedroom"]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_alice.status(), StatusCode::CREATED);
        let alice_id = json_body(create_alice).await["user"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mint = app
            .clone()
            .oneshot(
                Request::post(format!("/api/users/{alice_id}/tokens"))
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"sse"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let alice_token = json_body(mint).await["token"].as_str().unwrap().to_owned();

        // Open SSE stream for alice
        let sse = app
            .clone()
            .oneshot(
                Request::get("/api/events")
                    .header("authorization", format!("Bearer {alice_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(sse.status(), StatusCode::OK);
        let mut body = sse.into_body();

        // Revoke bedroom from alice while stream is open
        let narrow = app
            .clone()
            .oneshot(
                Request::put(format!("/api/users/{alice_id}"))
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"allowed_areas":["living_room"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(narrow.status(), StatusCode::OK);

        // Publish bedroom (now forbidden) then living_room (still allowed)
        events.publish(CoreEvent::EntityStateChanged {
            entity_id: "light.bedroom".into(),
            area_id: Some("bedroom".into()),
            old_state: None,
            new_state: DomainState::Light {
                on: true,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
            extra: serde_json::Map::new(),
            at: 1,
        });
        events.publish(CoreEvent::EntityStateChanged {
            entity_id: "light.living".into(),
            area_id: Some("living_room".into()),
            old_state: None,
            new_state: DomainState::Light {
                on: true,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
            extra: serde_json::Map::new(),
            at: 2,
        });

        let frame = timeout(Duration::from_secs(2), body.frame())
            .await
            .expect("frame")
            .expect("some")
            .expect("ok");
        let chunk = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
        assert!(chunk.contains("light.living"), "got: {chunk}");
        assert!(
            !chunk.contains("light.bedroom"),
            "revoked area leaked: {chunk}"
        );

        // Delete alice -> stream must terminate cleanly on next event without emitting it
        let del = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/users/{alice_id}"))
                    .header("cookie", &admin_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::NO_CONTENT);

        events.publish(CoreEvent::EntityStateChanged {
            entity_id: "light.living".into(),
            area_id: Some("living_room".into()),
            old_state: None,
            new_state: DomainState::Light {
                on: false,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
            extra: serde_json::Map::new(),
            at: 3,
        });
        let next = timeout(Duration::from_secs(2), body.frame())
            .await
            .expect("stream should close promptly");
        assert!(
            next.is_none(),
            "expected SSE stream to close after user deletion"
        );
    }

    #[tokio::test]
    async fn regression_demoted_bootstrapped_admin_to_guest_cannot_control_locks() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], false).await;
        fixture.state.state.upsert_entity(make_entity(
            "lock.living",
            "Front Door",
            Some("living_room"),
            DomainState::Lock {
                state: LockState::Locked,
            },
        ));
        let app = build_router(fixture.state);

        // Bootstrap first admin
        let boot = app
            .clone()
            .oneshot(
                Request::post("/api/auth/bootstrap")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin1","password":"password123","display_name":"Admin 1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(boot.status(), StatusCode::CREATED);
        let admin1_cookie = boot
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let admin1_id = json_body(boot).await["user"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        // Create second admin so admin1 can be demoted to Guest
        let admin2_token =
            create_user_and_token(&app, &admin1_cookie, "admin2", "admin", &[]).await;
        let demote = app
            .clone()
            .oneshot(
                Request::put(format!("/api/users/{admin1_id}"))
                    .header("authorization", format!("Bearer {admin2_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"role":"guest","allowed_areas":["living_room"]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(demote.status(), StatusCode::OK);

        // Re-login as demoted admin1 (now Guest) and attempt to unlock lock.living -> 403
        let relogin = app
            .clone()
            .oneshot(
                Request::post("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin1","password":"password123"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(relogin.status(), StatusCode::OK);
        let guest_cookie = relogin
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();

        let cmd = app
            .clone()
            .oneshot(
                Request::post("/api/entities/lock.living/command")
                    .header("cookie", &guest_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"unlock"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cmd.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn regression_auth_rollback_on_save_failure_and_login_password_change_race() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], false).await;
        let auth = fixture.state.auth.clone();
        let users_path = fixture._dir.path().join("users.toml");

        // Block users.toml write by placing a directory at its path
        std::fs::create_dir(&users_path).expect("create blocking dir");
        assert!(
            auth.bootstrap_admin("admin", "password123", "Admin")
                .is_err()
        );
        assert!(
            !auth.is_initialized(),
            "failed bootstrap must roll back in-memory users"
        );

        // Remove blocker and retry -> succeeds
        std::fs::remove_dir(&users_path).expect("remove blocking dir");
        let (admin, _) = auth
            .bootstrap_admin("admin", "password123", "Admin")
            .expect("retry bootstrap should succeed");

        // Concurrent login with old password vs password change: no surviving old-password session
        let auth_login = auth.clone();
        let auth_update = auth.clone();
        let admin_id = admin.id.clone();
        let login_task =
            tokio::task::spawn_blocking(move || auth_login.login("admin", "password123"));
        let update_task = tokio::task::spawn_blocking(move || {
            auth_update.update_user(
                &admin_id,
                crate::auth::UpdateUserInput {
                    password: Some("new_password_456".into()),
                    ..Default::default()
                },
            )
        });
        let (login_res, update_res) = tokio::join!(login_task, update_task);
        update_res.unwrap().expect("password update succeeds");
        if let Ok((_, sid)) = login_res.unwrap() {
            assert!(
                auth.authenticate_session(&sid).is_none(),
                "session minted with old password must not survive password change"
            );
        }
    }

    #[tokio::test]
    async fn regression_automations_no_file_clobber_and_skills_config_preserve_and_rollback() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], true).await;
        let auto_dir = fixture._dir.path().join("automations");
        let devices_path = fixture._dir.path().join("devices.toml");

        // Pre-seed morning.toml whose rule id is "wake_up"
        let wake_up_toml = r#"
id = "wake_up"
name = "Wake Up"
enabled = true

[[triggers]]
type = "webhook"
webhook_id = "hook1"

[[actions]]
type = "entity.call"
entity_id = "light.living"
action = "turn_on"
"#;
        std::fs::write(auto_dir.join("morning.toml"), wake_up_toml).expect("write morning.toml");
        fixture.state.automations.as_ref().unwrap().reload();

        let devices_arc = fixture.state.devices.clone();
        let app = build_router(fixture.state);
        let admin_cookie = bootstrap_admin_cookie(&app).await;

        let clobber_toml = wake_up_toml.replace("wake_up", "morning");

        // 1. POST /api/automations with id = "morning" must return 409 and NOT overwrite morning.toml
        let post_clobber = app
            .clone()
            .oneshot(
                Request::post("/api/automations")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"raw_toml": clobber_toml}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(post_clobber.status(), StatusCode::CONFLICT);

        // 2. PUT /api/automations/morning (nonexistent id) must return 404 and NOT overwrite morning.toml
        let put_clobber = app
            .clone()
            .oneshot(
                Request::put("/api/automations/morning")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"raw_toml": clobber_toml}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_clobber.status(), StatusCode::NOT_FOUND);

        // Verify wake_up is still intact
        let get_wake = app
            .clone()
            .oneshot(
                Request::get("/api/automations/wake_up")
                    .header("cookie", &admin_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_wake.status(), StatusCode::OK);

        // 3. PUT /api/skills/miio/config sets host, then omitting config preserves host
        let set_cfg = app
            .clone()
            .oneshot(
                Request::put("/api/skills/miio/config")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"enabled":true,"config":{"host":"192.168.1.42"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(set_cfg.status(), StatusCode::OK);

        let update_secret_only = app
            .clone()
            .oneshot(
                Request::put("/api/skills/miio/config")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"secrets":{"MIIO_TOKEN":"abc"}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update_secret_only.status(), StatusCode::OK);
        {
            let guard = devices_arc.read().unwrap();
            assert_eq!(
                guard.skills["miio"]
                    .config
                    .get("host")
                    .and_then(|v| v.as_str()),
                Some("192.168.1.42")
            );
        }

        // 4. Failed save_devices must roll back in-memory devices config
        std::fs::remove_file(&devices_path).ok();
        std::fs::create_dir(&devices_path).expect("block devices.toml");
        let failed_save = app
            .clone()
            .oneshot(
                Request::put("/api/skills/miio/config")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"config":{"host":"10.0.0.99"}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(failed_save.status(), StatusCode::INTERNAL_SERVER_ERROR);
        {
            let guard = devices_arc.read().unwrap();
            assert_eq!(
                guard.skills["miio"]
                    .config
                    .get("host")
                    .and_then(|v| v.as_str()),
                Some("192.168.1.42")
            );
        }
    }

    #[tokio::test]
    async fn spa_static_serving_and_api_404_isolation() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], false).await;
        let dist_dir = fixture._dir.path().join("web_dist");
        std::fs::create_dir_all(dist_dir.join("assets")).expect("create dist");
        std::fs::write(
            dist_dir.join("index.html"),
            "<!doctype html><html><body>xiaohe-spa</body></html>",
        )
        .expect("write index.html");
        std::fs::write(dist_dir.join("assets/app.js"), "console.log('ok');").expect("write asset");

        let app = build_router_with_spa(fixture.state, Some(dist_dir));

        // 1. Root serves index.html
        let root = app
            .clone()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(root.status(), StatusCode::OK);
        let root_body = String::from_utf8(
            root.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(root_body.contains("xiaohe-spa"));

        // 2. Client-side SPA route falls back to index.html
        let spa_route = app
            .clone()
            .oneshot(Request::get("/automations").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(spa_route.status(), StatusCode::OK);
        let spa_body = String::from_utf8(
            spa_route
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(spa_body.contains("xiaohe-spa"));

        // 3. Static asset serves real JS file
        let asset = app
            .clone()
            .oneshot(Request::get("/assets/app.js").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
        let asset_body = String::from_utf8(
            asset
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert_eq!(asset_body, "console.log('ok');");

        // 4. Unknown /api/* (and /api/ trailing slash) returns JSON 404, NOT index.html
        for req in [
            Request::get("/api/does_not_exist")
                .body(Body::empty())
                .unwrap(),
            Request::get("/api/").body(Body::empty()).unwrap(),
            Request::post("/api/").body(Body::empty()).unwrap(),
        ] {
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
            assert_eq!(json_body(resp).await["error"], "not_found");
        }

        // 5. Missing static asset under /assets/* returns 404, NOT 200 text/html index.html
        let missing_asset = app
            .clone()
            .oneshot(
                Request::get("/assets/nonexistent.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_asset.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_api_rbac_and_persistence() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let fixture = test_fixture(store, vec![], false).await;
        let app = build_router(fixture.state.clone());

        // Unauthenticated request -> 401
        let unauth = app
            .clone()
            .oneshot(Request::get("/api/dashboard").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        let admin_cookie = bootstrap_admin_cookie(&app).await;
        let member_token =
            create_user_and_token(&app, &admin_cookie, "alice", "member", &["living_room"]).await;

        // Default layout returned before dashboard.toml exists
        let get_default = app
            .clone()
            .oneshot(
                Request::get("/api/dashboard")
                    .header("authorization", format!("Bearer {member_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_default.status(), StatusCode::OK);
        let default_json = json_body(get_default).await;
        assert!(default_json["widgets"].as_array().unwrap().len() >= 2);

        // Member cannot PUT /api/dashboard -> 403
        let new_layout = json!({
            "widgets": [
                {
                    "id": "pm25-kpi",
                    "title": "Living Room PM2.5",
                    "size": "sm",
                    "type": "metric",
                    "entity_id": "sensor.purifier_pm25",
                    "accent": "emerald",
                    "warn_above": 35.0,
                    "crit_above": 75.0
                },
                {
                    "id": "temp-trend",
                    "title": "Temperature 24h",
                    "size": "md",
                    "type": "sparkline",
                    "entity_id": "sensor.weather_temperature",
                    "window_hours": 24
                }
            ]
        });
        let member_put = app
            .clone()
            .oneshot(
                Request::put("/api/dashboard")
                    .header("authorization", format!("Bearer {member_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(new_layout.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(member_put.status(), StatusCode::FORBIDDEN);

        // Admin invalid layout (duplicate id) -> 400
        let bad_layout = json!({
            "widgets": [
                {"id": "dup", "size": "sm", "type": "system_status"},
                {"id": "dup", "size": "md", "type": "weather"}
            ]
        });
        let bad_put = app
            .clone()
            .oneshot(
                Request::put("/api/dashboard")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(bad_layout.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_put.status(), StatusCode::BAD_REQUEST);

        // Admin valid PUT persists to config/dashboard.toml and is readable by Member
        let ok_put = app
            .clone()
            .oneshot(
                Request::put("/api/dashboard")
                    .header("cookie", &admin_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(new_layout.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok_put.status(), StatusCode::OK);
        assert!(
            fixture
                .state
                .config_store
                .config_dir()
                .join("dashboard.toml")
                .is_file()
        );

        let get_saved = app
            .clone()
            .oneshot(
                Request::get("/api/dashboard")
                    .header("authorization", format!("Bearer {member_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_saved.status(), StatusCode::OK);
        let saved_json = json_body(get_saved).await;
        assert_eq!(saved_json["widgets"].as_array().unwrap().len(), 2);
        assert_eq!(saved_json["widgets"][0]["id"], "pm25-kpi");
        assert_eq!(saved_json["widgets"][1]["window_hours"], 24);
    }
}
