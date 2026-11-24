pub mod auth;
pub mod users;
pub mod webhooks;

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Serialize;
use tower_http::trace::TraceLayer;
use tracing::warn;

use crate::auth::AuthService;
use crate::config::WebhookConfig;
use crate::http::webhooks::{MAX_WEBHOOK_BODY_BYTES, handle_webhook};
use crate::skills::SkillBridge;
use crate::state::{EventBus, StateStore};
use crate::storage::HistoryStore;

#[derive(Clone)]
pub struct AppState {
    pub history: Arc<dyn HistoryStore>,
    pub state: StateStore,
    pub events: EventBus,
    pub webhooks: Arc<Vec<WebhookConfig>>,
    pub skills: Option<SkillBridge>,
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

/// Build the `axum` HTTP router with health probes, auth/RBAC, user management, and webhooks.
pub fn build_router(state: AppState) -> Router {
    let webhook_route = Router::new()
        .route("/api/webhooks/{webhook_id}", post(handle_webhook))
        .layer(DefaultBodyLimit::max(MAX_WEBHOOK_BODY_BYTES));

    Router::new()
        .route("/health", get(health))
        .route("/health/ready", get(health_ready))
        .route("/api/auth/bootstrap", post(auth::bootstrap))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
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
        .merge(webhook_route)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
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

    async fn json_body(response: axum::response::Response) -> Value {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("parse json")
    }

    fn test_auth() -> (tempfile::TempDir, AuthService) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let auth = AuthService::new(
            crate::config::ConfigStore::new(dir.path()),
            crate::config::UsersConfig::default(),
        );
        (dir, auth)
    }

    #[tokio::test]
    async fn health_and_ready_return_200_with_healthy_store() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let events = EventBus::new();
        let state = StateStore::new(events.clone());
        let (_dir, auth) = test_auth();
        let app = build_router(AppState {
            history: store,
            state,
            events,
            webhooks: Arc::new(vec![]),
            skills: None,
            auth,
        });

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
        let events = EventBus::new();
        let state = StateStore::new(events.clone());
        let (_dir, auth) = test_auth();
        let app = build_router(AppState {
            history: Arc::new(FailingStore),
            state,
            events,
            webhooks: Arc::new(vec![]),
            skills: None,
            auth,
        });

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
        let events = EventBus::new();
        let mut rx = events.subscribe();
        let state = StateStore::new(events.clone());

        let secret = "my-super-secret-webhook-token";
        // Independently computed via `printf '%s' 'my-super-secret-webhook-token' | sha256sum`.
        let expected_hex = "bddbaa946a00f5f1c2add380d6eac35745b05f011dfd408f619d43c48ec1a97b";
        let expected_hash = format!("sha256:{expected_hex}");
        assert_eq!(hash_webhook_secret(secret), expected_hash);

        let webhooks = Arc::new(vec![
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
        ]);

        let (_dir, auth) = test_auth();
        let app = build_router(AppState {
            history: store,
            state,
            events,
            webhooks,
            skills: None,
            auth,
        });

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
        let events = EventBus::new();
        let state = StateStore::new(events.clone());
        let (_dir, auth) = test_auth();
        let app = build_router(AppState {
            history: store,
            state,
            events,
            webhooks: Arc::new(vec![]),
            skills: None,
            auth,
        });

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
}
