pub mod webhooks;

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tower_http::trace::TraceLayer;
use tracing::warn;

use crate::config::WebhookConfig;
use crate::http::webhooks::{MAX_WEBHOOK_BODY_BYTES, handle_webhook};
use crate::state::{EventBus, StateStore};
use crate::storage::HistoryStore;

#[derive(Clone)]
pub struct AppState {
    pub history: Arc<dyn HistoryStore>,
    pub state: StateStore,
    pub events: EventBus,
    pub webhooks: Arc<Vec<WebhookConfig>>,
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

/// Build the `axum` HTTP router with health probes, webhook ingestion, and request tracing.
pub fn build_router(state: AppState) -> Router {
    let webhook_route = Router::new()
        .route("/api/webhooks/{webhook_id}", post(handle_webhook))
        .layer(DefaultBodyLimit::max(MAX_WEBHOOK_BODY_BYTES));

    Router::new()
        .route("/health", get(health))
        .route("/health/ready", get(health_ready))
        .merge(webhook_route)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<LivenessResponse> {
    Json(LivenessResponse { status: "ok" })
}

async fn health_ready(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    match state.history.health_check().await {
        Ok(()) => (
            StatusCode::OK,
            Json(ReadinessResponse {
                status: "ok",
                storage: "ok",
                runtime: "disabled",
            }),
        ),
        Err(err) => {
            warn!(error = %err, "storage health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReadinessResponse {
                    status: "degraded",
                    storage: "error",
                    runtime: "disabled",
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

    #[tokio::test]
    async fn health_and_ready_return_200_with_healthy_store() {
        let store = Arc::new(SqliteStore::open(":memory:").await.expect("sqlite"));
        let events = EventBus::new();
        let state = StateStore::new(events.clone());
        let app = build_router(AppState {
            history: store,
            state,
            events,
            webhooks: Arc::new(vec![]),
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
        let app = build_router(AppState {
            history: Arc::new(FailingStore),
            state,
            events,
            webhooks: Arc::new(vec![]),
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
        let webhooks = Arc::new(vec![
            WebhookConfig {
                id: "ios_arriving".into(),
                name: Some("iOS Shortcuts".into()),
                secret_hash: hash_webhook_secret(secret),
                enabled: true,
            },
            WebhookConfig {
                id: "disabled_hook".into(),
                name: None,
                secret_hash: hash_webhook_secret(secret),
                enabled: false,
            },
        ]);

        let app = build_router(AppState {
            history: store,
            state,
            events,
            webhooks,
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
}
