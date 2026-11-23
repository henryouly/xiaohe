use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::trace::TraceLayer;
use tracing::warn;

use crate::storage::HistoryStore;

#[derive(Clone)]
pub struct AppState {
    pub history: Arc<dyn HistoryStore>,
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

/// Build the `axum` HTTP router with health probes and structured request tracing.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/health/ready", get(health_ready))
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
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
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
        let app = build_router(AppState { history: store });

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
        let app = build_router(AppState {
            history: Arc::new(FailingStore),
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
}
