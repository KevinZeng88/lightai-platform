use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{routing::get, routing::post, Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::models::{HeartbeatRequest, HeartbeatResponse, MetricsQuery, RegisterRequest};
use crate::repository;

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

pub fn app(pool: SqlitePool) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/agent/register", post(register_agent))
        .route("/api/agent/heartbeat", post(agent_heartbeat))
        .route("/api/nodes", get(list_nodes))
        .route("/api/nodes/{node_id}/metrics", get(node_metrics))
        .route(
            "/api/nodes/{node_id}/gpus/{gpu_key}/metrics",
            get(gpu_metrics),
        )
        .with_state(pool)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "server",
    })
}

async fn register_agent(
    State(pool): State<SqlitePool>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<crate::models::RegisterResponse>, ApiError> {
    Ok(Json(repository::register_node(&pool, request).await?))
}

async fn agent_heartbeat(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
    Json(request): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, ApiError> {
    let token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    if !repository::authenticate_node(&pool, &request.node_id, token).await? {
        return Err(ApiError::Unauthorized);
    }

    repository::record_heartbeat(&pool, request).await?;
    Ok(Json(HeartbeatResponse { status: "ok" }))
}

async fn list_nodes(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::NodeListResponse>, ApiError> {
    Ok(Json(repository::list_nodes(&pool).await?))
}

async fn node_metrics(
    State(pool): State<SqlitePool>,
    Path(node_id): Path<String>,
    Query(query): Query<MetricsQuery>,
) -> Result<Json<crate::models::NodeMetricSamplesResponse>, ApiError> {
    let (from, to) = time_window(query);
    Ok(Json(
        repository::node_metric_samples(&pool, &node_id, from, to).await?,
    ))
}

async fn gpu_metrics(
    State(pool): State<SqlitePool>,
    Path((node_id, gpu_key)): Path<(String, String)>,
    Query(query): Query<MetricsQuery>,
) -> Result<Json<crate::models::GpuMetricSamplesResponse>, ApiError> {
    let (from, to) = time_window(query);
    Ok(Json(
        repository::gpu_metric_samples(&pool, &node_id, &gpu_key, from, to).await?,
    ))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn time_window(query: MetricsQuery) -> (i64, i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let to = query.to.unwrap_or(now);
    let from = query.from.unwrap_or(to - 3600);
    (from, to)
}

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "unauthorized",
                }),
            )
                .into_response(),
            Self::Internal(error) => {
                tracing::error!(%error, "api request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "internal_error",
                    }),
                )
                    .into_response()
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: &'static str,
}
