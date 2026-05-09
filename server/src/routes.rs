use axum::body::Body;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{routing::delete, routing::get, routing::post, routing::put, Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;
use tower_http::services::ServeDir;

use crate::domain;
use crate::models::{
    AgentConfigPolicy, AgentTaskPollRequest, AgentTaskResultRequest, AuditQuery, AuthResponse,
    AuthUser, ChangePasswordRequest, CollectorRegistryEntry, FrontendErrorReport, GpuMetricsQuery,
    HeartbeatRequest, HeartbeatResponse, LogQuery, LoginRequest, MetricsQuery, ModelFileRequest,
    ModelFileTrashRequest, ModelInstanceCreateRequest, ModelInstanceUpdateRequest, ModelRequest,
    RegisterCollectorRequest, RegisterRequest, RuntimeEnvironmentRequest, SetupAdminRequest,
    SetupStatusResponse, UserCreateRequest, UserGroupCreateRequest, UserGroupListResponse,
    UserGroupMembersRequest, UserGroupResponse, UserGroupUpdateRequest, UserListResponse,
    UserUpdateRequest,
};
use crate::platform_log::LogPolicy;
use crate::repository;

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

pub fn app(pool: SqlitePool) -> Router {
    app_with_auth_policies(
        pool,
        repository::PasswordPolicy::default(),
        repository::SessionPolicy::default(),
    )
}

pub fn app_with_auth_policies(
    pool: SqlitePool,
    password_policy: repository::PasswordPolicy,
    session_policy: repository::SessionPolicy,
) -> Router {
    app_with_web(pool, password_policy, session_policy, None)
}

pub fn app_with_web(
    pool: SqlitePool,
    password_policy: repository::PasswordPolicy,
    session_policy: repository::SessionPolicy,
    web_dist_dir: Option<String>,
) -> Router {
    let auth_state = AuthState::new(pool.clone(), password_policy, session_policy);
    let mut router = Router::new()
        .route("/health", get(health))
        .route("/api/setup/status", get(setup_status))
        .route("/api/setup/admin", post(setup_admin))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(current_user))
        .route("/api/auth/change-password", post(change_password))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/{id}", put(update_user))
        .route("/api/groups", get(list_groups).post(create_group))
        .route("/api/groups/{id}", put(update_group).delete(delete_group))
        .route("/api/groups/{id}/members", put(update_group_members))
        .route("/api/agent/register", post(register_agent))
        .route("/api/agent/heartbeat", post(agent_heartbeat))
        .route(
            "/api/agent/collector-registry",
            get(agent_collector_registry),
        )
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
        .route(
            "/api/collector-registry",
            get(list_collector_registry_entries).post(register_collector_entry),
        )
        .route(
            "/api/collector-registry/{id}/{version}",
            get(get_collector_entry)
                .put(update_collector_entry)
                .delete(delete_collector_entry),
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
        .route(
            "/api/model-instances/{id}/logs",
            post(refresh_instance_logs),
        )
        .route("/api/model-file-trash", get(list_model_file_trash))
        .route("/api/logs", get(read_logs))
        .route("/api/frontend-errors", post(report_frontend_error))
        .route("/api/audit-events", get(list_audit_events))
        .route(
            "/api/config/server-logs",
            get(get_server_log_policy).put(update_server_log_policy),
        )
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
        .layer(axum::Extension(AuthPolicies {
            password_policy: auth_state.password_policy.clone(),
            session_policy: auth_state.session_policy.clone(),
        }))
        .route_layer(middleware::from_fn_with_state(
            auth_state,
            control_plane_auth,
        ))
        .route("/api/{*rest}", get(api_not_found).post(api_not_found).put(api_not_found).delete(api_not_found).patch(api_not_found))
        .with_state(pool);
    if let Some(dist_dir) = &web_dist_dir {
        // ServeDir handles static assets.  SPA deep routes that don't match a file
        // would return 404, so we use not_found_service to fall back to index.html.
        // Wrap with a status-correcting service that maps 404 -> 200 for SPA.
        let index_path = format!("{dist_dir}/index.html");
        let serve_dir = ServeDir::new(dist_dir).fallback(
            SpaFallback::new(index_path),
        );
        router = router.fallback_service(serve_dir);
        tracing::info!(dist_dir, "serving web static files with SPA fallback");
    }
    router
}

#[derive(Clone)]
struct AuthState {
    pool: SqlitePool,
    password_policy: repository::PasswordPolicy,
    session_policy: repository::SessionPolicy,
}

#[derive(Clone)]
struct AuthPolicies {
    password_policy: repository::PasswordPolicy,
    session_policy: repository::SessionPolicy,
}

impl AuthState {
    fn new(
        pool: SqlitePool,
        password_policy: repository::PasswordPolicy,
        session_policy: repository::SessionPolicy,
    ) -> Self {
        Self {
            pool,
            password_policy,
            session_policy,
        }
    }
}

async fn control_plane_auth(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    mut request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if !path.starts_with("/api/")
        || path.starts_with("/api/agent/")
        || path == "/api/auth/login"
        || path == "/api/setup/status"
        || path == "/api/setup/admin"
    {
        return next.run(request).await;
    }

    if let Some(session_token) = session_cookie(&headers) {
        match repository::authenticate_session(&auth.pool, session_token, &auth.session_policy)
            .await
        {
            Ok(Some(user)) => {
                if user.must_change_password && !is_password_change_allowed_path(path) {
                    return forbidden_response_with_message(
                        "Password change required before continuing".to_string(),
                    );
                }
                if !is_authorized_for_path(&user, request.method(), path) {
                    return forbidden_response();
                }
                request.extensions_mut().insert(user);
                return next.run(request).await;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(error = %error, "failed to authenticate user session");
            }
        }
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "unauthorized",
            message: Some("Please log in".to_string()),
        }),
    )
        .into_response()
}

fn is_authorized_for_path(user: &AuthUser, method: &axum::http::Method, path: &str) -> bool {
    let role = user.effective_role.as_str();
    if role == "admin" {
        return true;
    }
    if path.starts_with("/api/auth/") || path == "/api/frontend-errors" {
        return true;
    }
    if path.starts_with("/api/users")
        || path.starts_with("/api/groups")
        || path.starts_with("/api/config")
        || path.starts_with("/api/model-file-trash")
    {
        return false;
    }
    if role == "operator" {
        return true;
    }
    matches!(*method, axum::http::Method::GET)
}

fn is_password_change_allowed_path(path: &str) -> bool {
    matches!(
        path,
        "/api/auth/me" | "/api/auth/logout" | "/api/auth/change-password"
    ) || path == "/api/frontend-errors"
}

fn forbidden_response() -> Response {
    forbidden_response_with_message("Insufficient permissions for this operation".to_string())
}

fn forbidden_response_with_message(message: String) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "forbidden",
            message: Some(message),
        }),
    )
        .into_response()
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == "lightai_session" && !value.is_empty()).then_some(value)
    })
}

async fn login(
    State(pool): State<SqlitePool>,
    Extension(auth): Extension<AuthPolicies>,
    Json(request): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let Some((user, session_token)) = repository::login_user(
        &pool,
        &request.username,
        &request.password,
        &auth.session_policy,
        &auth.password_policy,
    )
    .await?
    else {
        return Err(ApiError::Unauthorized);
    };
    let cookie = session_cookie_header(
        &session_token,
        auth.session_policy.ttl_secs,
        auth.session_policy.secure_cookie,
    );
    audit_actor_success(&pool, &user, "auth.login", "user", Some(&user.id)).await;
    Ok(([(header::SET_COOKIE, cookie)], Json(AuthResponse { user })))
}

async fn logout(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(token) = session_cookie(&headers) {
        repository::revoke_session(&pool, token).await?;
    }
    audit_actor_success(&pool, &user, "auth.logout", "user", Some(&user.id)).await;
    Ok((
        [(
            header::SET_COOKIE,
            "lightai_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".to_string(),
        )],
        Json(serde_json::json!({ "status": "ok" })),
    ))
}

async fn change_password(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthPolicies>,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    repository::change_user_password(
        &pool,
        &user.id,
        &request.current_password,
        &request.new_password,
        &auth.password_policy,
    )
    .await
    .map_err(password_change_error)?;
    audit_actor_success(&pool, &user, "auth.password.change", "user", Some(&user.id)).await;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

async fn setup_status(
    State(pool): State<SqlitePool>,
) -> Result<Json<SetupStatusResponse>, ApiError> {
    Ok(Json(SetupStatusResponse {
        setup_required: repository::user_count(&pool).await? == 0,
    }))
}

async fn setup_admin(
    State(pool): State<SqlitePool>,
    Extension(auth): Extension<AuthPolicies>,
    Json(request): Json<SetupAdminRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = repository::setup_initial_admin(
        &pool,
        &request.username,
        &request.password,
        &auth.password_policy,
    )
    .await
    .map_err(setup_error)?;
    let (_, session_token) = repository::login_user(
        &pool,
        &request.username,
        &request.password,
        &auth.session_policy,
        &auth.password_policy,
    )
    .await?
    .ok_or(ApiError::Unauthorized)?;
    let cookie = session_cookie_header(
        &session_token,
        auth.session_policy.ttl_secs,
        auth.session_policy.secure_cookie,
    );
    audit_actor_success(&pool, &user, "auth.setup", "user", Some(&user.id)).await;
    Ok(([(header::SET_COOKIE, cookie)], Json(AuthResponse { user })))
}

fn session_cookie_header(token: &str, max_age_secs: i64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "lightai_session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure}"
    )
}

async fn current_user(
    Extension(user): Extension<AuthUser>,
) -> Result<Json<AuthResponse>, ApiError> {
    Ok(Json(AuthResponse { user }))
}

async fn list_users(
    Extension(user): Extension<AuthUser>,
    State(pool): State<SqlitePool>,
) -> Result<Json<UserListResponse>, ApiError> {
    require_admin(&user)?;
    Ok(Json(UserListResponse {
        users: repository::list_users(&pool).await?,
    }))
}

async fn create_user(
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthPolicies>,
    State(pool): State<SqlitePool>,
    Json(request): Json<UserCreateRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    require_admin(&user)?;
    let created = repository::create_user_with_policy(&pool, request, &auth.password_policy)
        .await
        .map_err(user_management_error)?;
    audit_actor_success(&pool, &user, "user.create", "user", Some(&created.id)).await;
    Ok(Json(AuthResponse { user: created }))
}

async fn update_user(
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthPolicies>,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    Json(request): Json<UserUpdateRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    require_admin(&user)?;
    let updated = repository::update_user_with_policy(&pool, &id, request, &auth.password_policy)
        .await
        .map_err(user_management_error)?;
    audit_actor_success(&pool, &user, "user.update", "user", Some(&id)).await;
    Ok(Json(AuthResponse { user: updated }))
}

async fn list_groups(
    Extension(user): Extension<AuthUser>,
    State(pool): State<SqlitePool>,
) -> Result<Json<UserGroupListResponse>, ApiError> {
    require_admin(&user)?;
    Ok(Json(UserGroupListResponse {
        groups: repository::list_user_groups(&pool).await?,
    }))
}

async fn create_group(
    Extension(user): Extension<AuthUser>,
    State(pool): State<SqlitePool>,
    Json(request): Json<UserGroupCreateRequest>,
) -> Result<Json<UserGroupResponse>, ApiError> {
    require_admin(&user)?;
    let group = repository::create_user_group(&pool, request)
        .await
        .map_err(group_management_error)?;
    audit_actor_success(&pool, &user, "group.create", "user_group", Some(&group.id)).await;
    Ok(Json(UserGroupResponse { group }))
}

async fn update_group(
    Extension(user): Extension<AuthUser>,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    Json(request): Json<UserGroupUpdateRequest>,
) -> Result<Json<UserGroupResponse>, ApiError> {
    require_admin(&user)?;
    let group = repository::update_user_group(&pool, &id, request)
        .await
        .map_err(group_management_error)?;
    audit_actor_success(&pool, &user, "group.update", "user_group", Some(&id)).await;
    Ok(Json(UserGroupResponse { group }))
}

async fn update_group_members(
    Extension(user): Extension<AuthUser>,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    Json(request): Json<UserGroupMembersRequest>,
) -> Result<Json<UserGroupResponse>, ApiError> {
    require_admin(&user)?;
    let group = repository::replace_user_group_members(&pool, &id, request)
        .await
        .map_err(group_management_error)?;
    audit_actor_success(
        &pool,
        &user,
        "group.members.update",
        "user_group",
        Some(&id),
    )
    .await;
    Ok(Json(UserGroupResponse { group }))
}

async fn delete_group(
    Extension(user): Extension<AuthUser>,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_admin(&user)?;
    repository::delete_user_group(&pool, &id)
        .await
        .map_err(group_management_error)?;
    audit_actor_success(&pool, &user, "group.delete", "user_group", Some(&id)).await;
    Ok(StatusCode::OK)
}

fn require_admin(user: &AuthUser) -> Result<(), ApiError> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn group_management_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    if message.contains("not found") {
        ApiError::NotFound(message)
    } else if message.contains("has members") {
        ApiError::Conflict(message)
    } else if message.contains("UNIQUE constraint")
        || message.contains("must be")
        || message.contains("may only contain")
    {
        ApiError::BadRequest(message)
    } else {
        ApiError::Internal(error)
    }
}

fn setup_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    if message.contains("already completed") {
        ApiError::Conflict(message)
    } else if message.contains("password") || message.contains("username") {
        ApiError::BadRequest(message)
    } else {
        ApiError::Internal(error)
    }
}

fn user_management_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    if message.contains("not found") {
        ApiError::NotFound(message)
    } else if message.contains("UNIQUE constraint")
        || message.contains("must be")
        || message.contains("cannot disable")
        || message.contains("may only contain")
    {
        ApiError::BadRequest(message)
    } else {
        ApiError::Internal(error)
    }
}

fn password_change_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    if message.contains("password") {
        ApiError::BadRequest(message)
    } else if message.contains("not found") {
        ApiError::NotFound(message)
    } else {
        ApiError::Internal(error)
    }
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "server",
    })
}

async fn api_not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "not_found",
            message: Some("API endpoint not found".to_string()),
        }),
    )
}

async fn register_agent(
    State(pool): State<SqlitePool>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<crate::models::RegisterResponse>, ApiError> {
    let response = match repository::register_node(&pool, request).await {
        Ok(response) => response,
        Err(error) => {
            let msg = error.to_string();
            if msg.contains("same name cannot") || msg.contains("same host cannot") {
                return Err(ApiError::BadRequest(msg));
            }
            return Err(ApiError::Internal(error));
        }
    };
    let _ = crate::platform_log::append(
        &repository::server_log_policy(&pool)
            .await
            .unwrap_or_default(),
        "server.log",
        "info",
        &format!("Agent registered successfully node_id={}", response.node_id),
    )
    .await;
    Ok(Json(response))
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
    let collector_registry = repository::list_collector_registry(&pool)
        .await
        .unwrap_or_default();
    Ok(Json(HeartbeatResponse {
        status: "ok",
        agent_config: repository::effective_agent_config(&pool, &node_id).await?,
        collector_registry,
    }))
}

async fn agent_collector_registry(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    // Authenticate via any registered node's token — the registry is global,
    // so we verify the token is valid for any node.
    let valid = repository::any_node_with_token(&pool, token).await?;
    if !valid {
        return Err(ApiError::Unauthorized);
    }
    let entries = repository::list_collector_registry(&pool)
        .await
        .unwrap_or_default();
    Ok(Json(serde_json::json!({ "collectors": entries })))
}

async fn list_nodes(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::NodeListResponse>, ApiError> {
    Ok(Json(repository::list_nodes(&pool).await?))
}

async fn read_logs(
    State(pool): State<SqlitePool>,
    Query(query): Query<LogQuery>,
) -> Result<Json<crate::models::LogResponse>, ApiError> {
    let source_type = query.source_type.as_deref().unwrap_or("server");
    let max_bytes = query.max_bytes.unwrap_or(64 * 1024).min(512 * 1024);
    match source_type {
        "server" => Ok(Json(crate::models::LogResponse {
            source_type: "server".to_string(),
            node_id: None,
            instance_id: None,
            content: crate::platform_log::read_tail(
                &repository::server_log_policy(&pool).await?,
                "server.log",
                max_bytes,
            )
            .await?,
            message: Some("Server log loaded".to_string()),
        })),
        "agent" => {
            let node_id = query.node_id.as_deref().ok_or_else(|| {
                ApiError::BadRequest("Node selection required to view Agent log".to_string())
            })?;
            Ok(Json(crate::models::LogResponse {
                source_type: "agent".to_string(),
                node_id: Some(node_id.to_string()),
                instance_id: None,
                content: domain::read_agent_log(&pool, node_id, max_bytes).await?,
                message: Some("Agent log loaded".to_string()),
            }))
        }
        "instance" => {
            let instance_id = query.instance_id.as_deref().ok_or_else(|| {
                ApiError::BadRequest("Instance selection required to view instance log".to_string())
            })?;
            let instance = domain::model_instance(&pool, instance_id).await?;
            Ok(Json(crate::models::LogResponse {
                source_type: "instance".to_string(),
                node_id: instance.node_id,
                instance_id: Some(instance_id.to_string()),
                content: instance
                    .log_tail
                    .or(instance.last_error)
                    .unwrap_or_else(|| "No instance log available".to_string()),
                message: Some("Instance log loaded".to_string()),
            }))
        }
        "errors" => Ok(Json(crate::models::LogResponse {
            source_type: "errors".to_string(),
            node_id: None,
            instance_id: None,
            content: domain::recent_error_summary(&pool).await?,
            message: Some("Recent error summary loaded".to_string()),
        })),
        "frontend" => Ok(Json(crate::models::LogResponse {
            source_type: "frontend".to_string(),
            node_id: None,
            instance_id: None,
            content: domain::frontend_error_summary(&pool).await?,
            message: Some("Frontend error log loaded".to_string()),
        })),
        _ => Err(ApiError::BadRequest(
            "unsupported log source type".to_string(),
        )),
    }
}

async fn get_server_log_policy(
    State(pool): State<SqlitePool>,
) -> Result<Json<LogPolicy>, ApiError> {
    Ok(Json(repository::server_log_policy(&pool).await?))
}

async fn update_server_log_policy(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<LogPolicy>,
) -> Result<Json<LogPolicy>, ApiError> {
    let policy = match repository::update_server_log_policy(&pool, request).await {
        Ok(policy) => policy,
        Err(error) => {
            let message = format!("{error:?}");
            audit_actor_result(
                &pool,
                &user,
                "config.update",
                "server_log_policy",
                Some("server"),
                None,
                None,
                "failed",
                Some(&message),
            )
            .await;
            return Err(ApiError::Internal(error));
        }
    };
    audit_actor_result(
        &pool,
        &user,
        "config.update",
        "server_log_policy",
        Some("server"),
        None,
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(policy))
}

async fn list_audit_events(
    State(pool): State<SqlitePool>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<crate::models::AuditListResponse>, ApiError> {
    Ok(Json(repository::list_audit_events(&pool, query).await?))
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
    Extension(user): Extension<AuthUser>,
    Json(request): Json<AgentConfigPolicy>,
) -> Result<Json<crate::models::AgentConfigPolicyView>, ApiError> {
    let view = repository::update_global_agent_config_policy(&pool, request).await?;
    audit_actor_result(
        &pool,
        &user,
        "config.update",
        "agent_config",
        Some("global"),
        None,
        None,
        "success",
        None,
    )
    .await;
    domain::notify_agent_tasks();
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
    Extension(user): Extension<AuthUser>,
    Path(node_id): Path<String>,
    Json(request): Json<AgentConfigPolicy>,
) -> Result<Json<crate::models::AgentConfigPolicyView>, ApiError> {
    let view = repository::update_node_agent_config_policy(&pool, &node_id, request).await?;
    audit_actor_result(
        &pool,
        &user,
        "config.update",
        "agent_config",
        Some(&node_id),
        Some(&node_id),
        None,
        "success",
        None,
    )
    .await;
    domain::notify_agent_tasks();
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
    Ok(Json(domain::list_runtime_environments(&pool, None).await?))
}

async fn list_node_runtime_environments(
    State(pool): State<SqlitePool>,
    Path(node_id): Path<String>,
) -> Result<Json<crate::models::RuntimeEnvironmentListResponse>, ApiError> {
    Ok(Json(
        domain::list_runtime_environments(&pool, Some(&node_id)).await?,
    ))
}

async fn create_runtime_environment(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(node_id): Path<String>,
    Json(request): Json<RuntimeEnvironmentRequest>,
) -> Result<Json<crate::models::RuntimeEnvironmentView>, ApiError> {
    let view = domain::create_runtime_environment(&pool, &node_id, request).await?;
    audit_actor_result(
        &pool,
        &user,
        "runtime.create",
        "runtime_environment",
        Some(&view.id),
        Some(&node_id),
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn get_runtime_environment(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::RuntimeEnvironmentView>, ApiError> {
    Ok(Json(domain::runtime_environment(&pool, &id).await?))
}

async fn update_runtime_environment(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<RuntimeEnvironmentRequest>,
) -> Result<Json<crate::models::RuntimeEnvironmentView>, ApiError> {
    let view = domain::update_runtime_environment(&pool, &id, request).await?;
    audit_actor_result(
        &pool,
        &user,
        "runtime.update",
        "runtime_environment",
        Some(&id),
        view.node_id.as_deref(),
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn delete_runtime_environment(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    domain::delete_runtime_environment(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "runtime.delete",
        "runtime_environment",
        Some(&id),
        None,
        None,
        "success",
        None,
    )
    .await;
    Ok(StatusCode::OK)
}

async fn check_runtime_environment(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::RuntimeEnvironmentView>, ApiError> {
    let view = domain::check_runtime_environment(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "runtime.check",
        "runtime_environment",
        Some(&id),
        view.node_id.as_deref(),
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn list_models(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::ModelListResponse>, ApiError> {
    Ok(Json(domain::list_models(&pool).await?))
}

async fn create_model(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<ModelRequest>,
) -> Result<Json<crate::models::ModelView>, ApiError> {
    let view = match domain::create_model(&pool, request).await {
        Ok(view) => view,
        Err(error) => {
            let message = format!("{error:?}");
            audit_actor_result(
                &pool,
                &user,
                "model.create",
                "model",
                None,
                None,
                None,
                "failed",
                Some(&message),
            )
            .await;
            return Err(error.into());
        }
    };
    audit_actor_success(&pool, &user, "model.create", "model", Some(&view.id)).await;
    Ok(Json(view))
}

async fn get_model(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelView>, ApiError> {
    Ok(Json(domain::model(&pool, &id).await?))
}

async fn update_model(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<ModelRequest>,
) -> Result<Json<crate::models::ModelView>, ApiError> {
    let view = domain::update_model(&pool, &id, request).await?;
    audit_actor_success(&pool, &user, "model.update", "model", Some(&id)).await;
    Ok(Json(view))
}

async fn delete_model(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    domain::delete_model(&pool, &id).await?;
    audit_actor_success(&pool, &user, "model.delete", "model", Some(&id)).await;
    Ok(StatusCode::OK)
}

async fn list_model_files(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelFileListResponse>, ApiError> {
    Ok(Json(domain::list_model_files(&pool, &id).await?))
}

async fn create_model_file(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<ModelFileRequest>,
) -> Result<Json<crate::models::ModelFileView>, ApiError> {
    let view = domain::create_model_file(&pool, &id, request).await?;
    audit_actor_result(
        &pool,
        &user,
        "model_file.create",
        "model_file",
        Some(&view.id),
        Some(&view.node_id),
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn get_model_file(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelFileView>, ApiError> {
    Ok(Json(domain::model_file(&pool, &id).await?))
}

async fn update_model_file(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<ModelFileRequest>,
) -> Result<Json<crate::models::ModelFileView>, ApiError> {
    let view = domain::update_model_file(&pool, &id, request).await?;
    audit_actor_result(
        &pool,
        &user,
        "model_file.update",
        "model_file",
        Some(&id),
        Some(&view.node_id),
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn delete_model_file(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    domain::delete_model_file(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "model_file.delete",
        "model_file",
        Some(&id),
        None,
        None,
        "success",
        None,
    )
    .await;
    Ok(StatusCode::OK)
}

async fn verify_model_file(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelFileView>, ApiError> {
    let view = domain::queue_model_file_verification(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "model_file.verify",
        "model_file",
        Some(&id),
        Some(&view.node_id),
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn list_model_instances(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::ModelInstanceListResponse>, ApiError> {
    Ok(Json(domain::list_model_instances(&pool).await?))
}

async fn create_model_instance(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<ModelInstanceCreateRequest>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    let view = domain::create_model_instance(&pool, request).await?;
    audit_actor_result(
        &pool,
        &user,
        "instance.create",
        "model_instance",
        Some(&view.id),
        view.node_id.as_deref(),
        Some(&view.id),
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn get_model_instance(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    Ok(Json(domain::model_instance(&pool, &id).await?))
}

async fn update_model_instance(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<ModelInstanceUpdateRequest>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    let view = domain::update_model_instance(&pool, &id, request).await?;
    audit_actor_result(
        &pool,
        &user,
        "instance.update",
        "model_instance",
        Some(&id),
        view.node_id.as_deref(),
        Some(&id),
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn delete_model_instance(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    domain::delete_model_instance(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "instance.delete",
        "model_instance",
        Some(&id),
        None,
        Some(&id),
        "success",
        None,
    )
    .await;
    Ok(StatusCode::OK)
}

async fn check_model_instance(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    let view = domain::check_model_instance(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "instance.check",
        "model_instance",
        Some(&id),
        view.node_id.as_deref(),
        Some(&id),
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn start_model_instance(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    let view = domain::start_model_instance(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "instance.start",
        "model_instance",
        Some(&id),
        view.node_id.as_deref(),
        Some(&id),
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn stop_model_instance(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    let view = domain::stop_model_instance(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "instance.stop",
        "model_instance",
        Some(&id),
        view.node_id.as_deref(),
        Some(&id),
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn test_model_instance(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelInstanceView>, ApiError> {
    let view = domain::test_model_instance(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "instance.test",
        "model_instance",
        Some(&id),
        view.node_id.as_deref(),
        Some(&id),
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn refresh_instance_logs(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::LogResponse>, ApiError> {
    let content = domain::refresh_instance_logs(&pool, &id).await?;
    let instance = domain::model_instance(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "instance.logs.refresh",
        "model_instance",
        Some(&id),
        instance.node_id.as_deref(),
        Some(&id),
        "success",
        None,
    )
    .await;
    Ok(Json(crate::models::LogResponse {
        source_type: "instance".to_string(),
        node_id: instance.node_id.clone(),
        instance_id: Some(id),
        content,
        message: Some("Instance log refreshed".to_string()),
    }))
}

async fn report_frontend_error(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Json(report): Json<FrontendErrorReport>,
) -> Result<StatusCode, ApiError> {
    let message = report.message.chars().take(1024).collect::<String>();
    let stack: Option<String> = report.stack.map(|s| s.chars().take(2048).collect());
    let url = report.url.map(|u| u.chars().take(512).collect::<String>());
    let occurred_at = report.occurred_at.unwrap_or_else(current_unix_secs);
    repository::record_frontend_error(
        &pool,
        &message,
        stack.as_deref(),
        url.as_deref(),
        occurred_at,
        Some(&user),
    )
    .await?;
    let _ = crate::platform_log::append(
        &repository::server_log_policy(&pool)
            .await
            .unwrap_or_default(),
        "server.log",
        "warn",
        &format!("Frontend error: {message}"),
    )
    .await;
    Ok(StatusCode::OK)
}

async fn list_model_file_trash(
    State(pool): State<SqlitePool>,
) -> Result<Json<crate::models::ModelFileTrashListResponse>, ApiError> {
    Ok(Json(domain::list_model_file_trash(&pool).await?))
}

async fn create_model_file_trash(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<ModelFileTrashRequest>,
) -> Result<Json<crate::models::ModelFileTrashView>, ApiError> {
    let view = domain::create_model_file_trash(&pool, &id, request).await?;
    audit_actor_result(
        &pool,
        &user,
        "trash.create",
        "model_file_trash",
        Some(&view.id),
        view.node_id.as_deref(),
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn cleanup_model_file_trash(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::ModelFileTrashView>, ApiError> {
    let view = domain::cleanup_model_file_trash(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "trash.cleanup",
        "model_file_trash",
        Some(&id),
        view.node_id.as_deref(),
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(view))
}

async fn delete_model_file_trash(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    domain::delete_model_file_trash(&pool, &id).await?;
    audit_actor_result(
        &pool,
        &user,
        "trash.delete_record",
        "model_file_trash",
        Some(&id),
        None,
        None,
        "success",
        None,
    )
    .await;
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
        domain::poll_agent_task(&pool, &request.node_id, request.current_config_version).await?,
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
    domain::record_agent_task_result(&pool, &id, request).await?;
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

async fn audit_actor_success(
    pool: &SqlitePool,
    actor: &AuthUser,
    operation_type: &str,
    target_type: &str,
    target_id: Option<&str>,
) {
    audit_actor_result(
        pool,
        actor,
        operation_type,
        target_type,
        target_id,
        None,
        None,
        "success",
        None,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn audit_actor_result(
    pool: &SqlitePool,
    actor: &AuthUser,
    operation_type: &str,
    target_type: &str,
    target_id: Option<&str>,
    node_id: Option<&str>,
    instance_id: Option<&str>,
    result: &str,
    error_message: Option<&str>,
) {
    let actor_type = "user";
    let detail_json = serde_json::json!({
        "actor_username": actor.username,
        "effective_role": actor.effective_role,
    })
    .to_string();
    let _ = repository::record_audit(
        pool,
        repository::AuditRecord {
            operation_type,
            target_type,
            target_id,
            node_id,
            instance_id,
            result,
            error_message,
            detail_json: Some(detail_json),
            actor_type: Some(actor_type),
            actor_id: Some(&actor.id),
            source: Some("web"),
        },
    )
    .await;
}

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    Forbidden,
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

impl From<domain::DomainError> for ApiError {
    fn from(error: domain::DomainError) -> Self {
        match error {
            domain::DomainError::BadRequest(message) => Self::BadRequest(message),
            domain::DomainError::NotFound(message) => Self::NotFound(message),
            domain::DomainError::Conflict(message) => Self::Conflict(message),
            domain::DomainError::Internal(error) => Self::Internal(error),
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
                    message: Some("Please log in or check credentials".to_string()),
                }),
            )
                .into_response(),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "forbidden",
                    message: Some("Insufficient permissions for this operation".to_string()),
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
                // fire-and-forget: into_response is not async, cannot await
                std::mem::drop(tokio::spawn(async move {
                    let _ = crate::platform_log::append(
                        &crate::platform_log::global(),
                        "server.log",
                        "error",
                        &format!("API internal error: {error}"),
                    )
                    .await;
                }));
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

// ── Collector registry ──

async fn list_collector_registry_entries(
    State(pool): State<SqlitePool>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let entries = repository::list_collector_registry(&pool).await?;
    Ok(Json(serde_json::json!({ "collectors": entries })))
}

async fn register_collector_entry(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<RegisterCollectorRequest>,
) -> Result<Json<CollectorRegistryEntry>, ApiError> {
    require_admin(&user)?;
    let entry = repository::register_collector(&pool, &request).await?;
    audit_actor_result(
        &pool,
        &user,
        "collector.register",
        "collector_registry",
        Some(&entry.id),
        None,
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(entry))
}

async fn get_collector_entry(
    State(pool): State<SqlitePool>,
    Path((id, version)): Path<(String, String)>,
) -> Result<Json<CollectorRegistryEntry>, ApiError> {
    let entries = repository::list_collector_registry(&pool).await?;
    let entry = entries
        .into_iter()
        .find(|e| e.id == id && e.version == version)
        .ok_or_else(|| ApiError::NotFound("collector registry entry not found".to_string()))?;
    Ok(Json(entry))
}

async fn delete_collector_entry(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path((id, version)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&user)?;
    repository::delete_collector_registry_entry(&pool, &id, &version).await?;
    audit_actor_result(
        &pool,
        &user,
        "collector.delete",
        "collector_registry",
        Some(&id),
        None,
        None,
        "success",
        None,
    )
    .await;
    Ok(StatusCode::OK)
}

async fn update_collector_entry(
    State(pool): State<SqlitePool>,
    Extension(user): Extension<AuthUser>,
    Path((_id, _version)): Path<(String, String)>,
    Json(request): Json<RegisterCollectorRequest>,
) -> Result<Json<CollectorRegistryEntry>, ApiError> {
    require_admin(&user)?;
    let entry = repository::register_collector(&pool, &request).await?;
    audit_actor_result(
        &pool,
        &user,
        "collector.update",
        "collector_registry",
        Some(&entry.id),
        None,
        None,
        "success",
        None,
    )
    .await;
    Ok(Json(entry))
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// Fallback service for SPA: serves index.html with status 200 for any request.
#[derive(Clone)]
struct SpaFallback {
    index_bytes: Vec<u8>,
}

impl SpaFallback {
    fn new(index_path: String) -> Self {
        Self {
            index_bytes: std::fs::read(&index_path).unwrap_or_default(),
        }
    }
}

impl tower::Service<axum::http::Request<Body>> for SpaFallback {
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future = std::future::Ready<Result<Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: axum::http::Request<Body>) -> Self::Future {
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(self.index_bytes.clone()))
            .unwrap();
        std::future::ready(Ok(response))
    }
}
