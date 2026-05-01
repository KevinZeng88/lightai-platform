use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{routing::delete, routing::get, routing::post, Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::models::{
    AgentConfigPolicy, AgentTaskPollRequest, AgentTaskResultRequest, GpuMetricsQuery,
    HeartbeatRequest, HeartbeatResponse, MetricsQuery, ModelFileRequest, ModelFileTrashRequest,
    ModelInstanceCreateRequest, ModelInstanceUpdateRequest, ModelRequest, RegisterRequest,
    RuntimeEnvironmentRequest,
};
use crate::repository;
use crate::stage3a;

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
        .route("/api/config/agent", get(list_agent_config_policies))
        .route(
            "/api/config/agent/global",
            get(get_global_agent_config_policy).put(update_global_agent_config_policy),
        )
        .route(
            "/api/nodes/{node_id}/config",
            get(get_node_agent_config_policy).put(update_node_agent_config_policy),
        )
        .route("/api/nodes/{node_id}/metrics", get(node_metrics))
        .route(
            "/api/nodes/{node_id}/gpus/{gpu_key}/metrics",
            get(gpu_metrics),
        )
        .route(
            "/api/nodes/{node_id}/gpu-metrics",
            get(gpu_metrics_by_query),
        )
        .route("/api/runtime-environments", get(list_runtime_environments))
        .route(
            "/api/nodes/{node_id}/runtime-environments",
            get(list_node_runtime_environments).post(create_runtime_environment),
        )
        .route(
            "/api/runtime-environments/{id}",
            get(get_runtime_environment)
                .put(update_runtime_environment)
                .delete(delete_runtime_environment),
        )
        .route(
            "/api/runtime-environments/{id}/check",
            post(check_runtime_environment),
        )
        .route("/api/models", get(list_models).post(create_model))
        .route(
            "/api/models/{id}",
            get(get_model).put(update_model).delete(delete_model),
        )
        .route(
            "/api/models/{id}/files",
            get(list_model_files).post(create_model_file),
        )
        .route(
            "/api/model-files/{id}",
            get(get_model_file)
                .put(update_model_file)
                .delete(delete_model_file),
        )
        .route("/api/model-files/{id}/verify", post(verify_model_file))
        .route("/api/model-files/{id}/trash", post(create_model_file_trash))
        .route(
            "/api/model-instances",
            get(list_model_instances).post(create_model_instance),
        )
        .route(
            "/api/model-instances/{id}",
            get(get_model_instance)
                .put(update_model_instance)
                .delete(delete_model_instance),
        )
        .route(
            "/api/model-instances/{id}/check",
            post(check_model_instance),
        )
        .route(
            "/api/model-instances/{id}/start",
            post(start_model_instance),
        )
        .route("/api/model-instances/{id}/stop", post(stop_model_instance))
        .route("/api/model-instances/{id}/test", post(test_model_instance))
        .route("/api/model-file-trash", get(list_model_file_trash))
        .route(
            "/api/model-file-trash/{id}/cleanup",
            post(cleanup_model_file_trash),
        )
        .route(
            "/api/model-file-trash/{id}",
            delete(delete_model_file_trash),
        )
        .route("/api/agent/tasks/poll", post(agent_task_poll))
        .route("/api/agent/tasks/{id}/result", post(agent_task_result))
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

    let node_id = request.node_id.clone();
    repository::record_heartbeat(&pool, request).await?;
    Ok(Json(HeartbeatResponse {
        status: "ok",
        agent_config: repository::effective_agent_config(&pool, &node_id).await?,
    }))
}

async fn list_nodes(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::NodeListResponse>, ApiError> {
    Ok(Json(repository::list_nodes(&pool).await?))
}

async fn list_agent_config_policies(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::AgentConfigPoliciesResponse>, ApiError> {
    Ok(Json(repository::list_agent_config_policies(&pool).await?))
}

async fn get_global_agent_config_policy(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::AgentConfigPolicyView>, ApiError> {
    Ok(Json(repository::global_agent_config_policy(&pool).await?))
}

async fn update_global_agent_config_policy(
    State(pool): State<SqlitePool>,
    Json(request): Json<AgentConfigPolicy>,
) -> Result<Json<crate::models::AgentConfigPolicyView>, ApiError> {
    let view = repository::update_global_agent_config_policy(&pool, request).await?;
    stage3a::notify_agent_tasks();
    Ok(Json(view))
}

async fn get_node_agent_config_policy(
    State(pool): State<SqlitePool>,
    Path(node_id): Path<String>,
) -> Result<Json<crate::models::AgentConfigPolicyView>, ApiError> {
    Ok(Json(
        repository::node_agent_config_policy(&pool, &node_id).await?,
    ))
}

async fn update_node_agent_config_policy(
    State(pool): State<SqlitePool>,
    Path(node_id): Path<String>,
    Json(request): Json<AgentConfigPolicy>,
) -> Result<Json<crate::models::AgentConfigPolicyView>, ApiError> {
    let view = repository::update_node_agent_config_policy(&pool, &node_id, request).await?;
    stage3a::notify_agent_tasks();
    Ok(Json(view))
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

async fn gpu_metrics_by_query(
    State(pool): State<SqlitePool>,
    Path(node_id): Path<String>,
    Query(query): Query<GpuMetricsQuery>,
) -> Result<Json<crate::models::GpuMetricSamplesResponse>, ApiError> {
    let (from, to) = time_window(MetricsQuery {
        from: query.from,
        to: query.to,
    });
    Ok(Json(
        repository::gpu_metric_samples(&pool, &node_id, &query.gpu_key, from, to).await?,
    ))
}

async fn list_runtime_environments(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::RuntimeEnvironmentListResponse>, ApiError> {
    Ok(Json(stage3a::list_runtime_environments(&pool, None).await?))
}

async fn list_node_runtime_environments(
    State(pool): State<SqlitePool>,
    Path(node_id): Path<String>,
) -> Result<Json<crate::models::RuntimeEnvironmentListResponse>, ApiError> {
    Ok(Json(
        stage3a::list_runtime_environments(&pool, Some(&node_id)).await?,
    ))
}

async fn create_runtime_environment(
    State(pool): State<SqlitePool>,
    Path(node_id): Path<String>,
    Json(request): Json<RuntimeEnvironmentRequest>,
) -> Result<Json<crate::models::RuntimeEnvironmentView>, ApiError> {
    Ok(Json(
        stage3a::create_runtime_environment(&pool, &node_id, request).await?,
    ))
}

async fn get_runtime_environment(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::RuntimeEnvironmentView>, ApiError> {
    Ok(Json(stage3a::runtime_environment(&pool, &id).await?))
}

async fn update_runtime_environment(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    Json(request): Json<RuntimeEnvironmentRequest>,
) -> Result<Json<crate::models::RuntimeEnvironmentView>, ApiError> {
    Ok(Json(
        stage3a::update_runtime_environment(&pool, &id, request).await?,
    ))
}

async fn delete_runtime_environment(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    stage3a::delete_runtime_environment(&pool, &id).await?;
    Ok(StatusCode::OK)
}

async fn check_runtime_environment(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::RuntimeEnvironmentView>, ApiError> {
    Ok(Json(stage3a::check_runtime_environment(&pool, &id).await?))
}

async fn list_models(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::ModelListResponse>, ApiError> {
    Ok(Json(stage3a::list_models(&pool).await?))
}

async fn create_model(
    State(pool): State<SqlitePool>,
    Json(request): Json<ModelRequest>,
) -> Result<Json<crate::models::ModelView>, ApiError> {
    Ok(Json(stage3a::create_model(&pool, request).await?))
}

async fn get_model(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelView>, ApiError> {
    Ok(Json(stage3a::model(&pool, &id).await?))
}

async fn update_model(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    Json(request): Json<ModelRequest>,
) -> Result<Json<crate::models::ModelView>, ApiError> {
    Ok(Json(stage3a::update_model(&pool, &id, request).await?))
}

async fn delete_model(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    stage3a::delete_model(&pool, &id).await?;
    Ok(StatusCode::OK)
}

async fn list_model_files(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelFileListResponse>, ApiError> {
    Ok(Json(stage3a::list_model_files(&pool, &id).await?))
}

async fn create_model_file(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    Json(request): Json<ModelFileRequest>,
) -> Result<Json<crate::models::ModelFileView>, ApiError> {
    Ok(Json(stage3a::create_model_file(&pool, &id, request).await?))
}

async fn get_model_file(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelFileView>, ApiError> {
    Ok(Json(stage3a::model_file(&pool, &id).await?))
}

async fn update_model_file(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    Json(request): Json<ModelFileRequest>,
) -> Result<Json<crate::models::ModelFileView>, ApiError> {
    Ok(Json(stage3a::update_model_file(&pool, &id, request).await?))
}

async fn delete_model_file(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    stage3a::delete_model_file(&pool, &id).await?;
    Ok(StatusCode::OK)
}

async fn verify_model_file(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelFileView>, ApiError> {
    Ok(Json(
        stage3a::queue_model_file_verification(&pool, &id).await?,
    ))
}

async fn list_model_instances(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::ModelInstanceListResponse>, ApiError> {
    Ok(Json(stage3a::list_model_instances(&pool).await?))
}

async fn create_model_instance(
    State(pool): State<SqlitePool>,
    Json(request): Json<ModelInstanceCreateRequest>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    Ok(Json(stage3a::create_model_instance(&pool, request).await?))
}

async fn get_model_instance(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    Ok(Json(stage3a::model_instance(&pool, &id).await?))
}

async fn update_model_instance(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    Json(request): Json<ModelInstanceUpdateRequest>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    Ok(Json(
        stage3a::update_model_instance(&pool, &id, request).await?,
    ))
}

async fn delete_model_instance(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    stage3a::delete_model_instance(&pool, &id).await?;
    Ok(StatusCode::OK)
}

async fn check_model_instance(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    Ok(Json(stage3a::check_model_instance(&pool, &id).await?))
}

async fn start_model_instance(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    Ok(Json(stage3a::start_model_instance(&pool, &id).await?))
}

async fn stop_model_instance(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    Ok(Json(stage3a::stop_model_instance(&pool, &id).await?))
}

async fn test_model_instance(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    Ok(Json(stage3a::test_model_instance(&pool, &id).await?))
}

async fn list_model_file_trash(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::ModelFileTrashListResponse>, ApiError> {
    Ok(Json(stage3a::list_model_file_trash(&pool).await?))
}

async fn create_model_file_trash(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    Json(request): Json<ModelFileTrashRequest>,
) -> Result<Json<crate::models::ModelFileTrashView>, ApiError> {
    Ok(Json(
        stage3a::create_model_file_trash(&pool, &id, request).await?,
    ))
}

async fn cleanup_model_file_trash(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelFileTrashView>, ApiError> {
    Ok(Json(stage3a::cleanup_model_file_trash(&pool, &id).await?))
}

async fn delete_model_file_trash(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    stage3a::delete_model_file_trash(&pool, &id).await?;
    Ok(StatusCode::OK)
}

async fn agent_task_poll(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
    Json(request): Json<AgentTaskPollRequest>,
) -> Result<Json<crate::models::AgentTaskPollResponse>, ApiError> {
    let token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    if !repository::authenticate_node(&pool, &request.node_id, token).await? {
        return Err(ApiError::Unauthorized);
    }
    Ok(Json(
        stage3a::poll_agent_task(&pool, &request.node_id, request.current_config_version).await?,
    ))
}

async fn agent_task_result(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<AgentTaskResultRequest>,
) -> Result<StatusCode, ApiError> {
    let token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    if !repository::authenticate_node(&pool, &request.node_id, token).await? {
        return Err(ApiError::Unauthorized);
    }
    stage3a::record_agent_task_result(&pool, &id, request).await?;
    Ok(StatusCode::OK)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn time_window(query: MetricsQuery) -> (i64, i64) {
    let now = current_unix_secs();
    let to = query.to.unwrap_or(now);
    let from = query.from.unwrap_or(to - 3600);
    (from, to)
}

fn current_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

impl From<stage3a::Stage3Error> for ApiError {
    fn from(error: stage3a::Stage3Error) -> Self {
        match error {
            stage3a::Stage3Error::BadRequest(message) => Self::BadRequest(message),
            stage3a::Stage3Error::NotFound(message) => Self::NotFound(message),
            stage3a::Stage3Error::Conflict(message) => Self::Conflict(message),
            stage3a::Stage3Error::Internal(error) => Self::Internal(error),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "unauthorized",
                    message: None,
                }),
            )
                .into_response(),
            Self::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "bad_request",
                    message: Some(message),
                }),
            )
                .into_response(),
            Self::NotFound(message) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "not_found",
                    message: Some(message),
                }),
            )
                .into_response(),
            Self::Conflict(message) => (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "conflict",
                    message: Some(message),
                }),
            )
                .into_response(),
            Self::Internal(error) => {
                tracing::error!(%error, "api request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "internal_error",
                        message: None,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}
