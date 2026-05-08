#![allow(dead_code)]
use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use lightai_server::{db, models, repository, routes};
use serde_json::{json, Value};
use tower::ServiceExt;

/// Create an initial admin user for integration tests.
pub async fn ensure_initial_admin(pool: &sqlx::SqlitePool, username: &str, password: &str) {
    if repository::user_count(pool).await.unwrap() > 0 {
        return;
    }
    repository::create_user(
        pool,
        models::UserCreateRequest {
            username: username.to_string(),
            password: password.to_string(),
            role: "admin".to_string(),
        },
    )
    .await
    .unwrap();
}

/// Log in as admin and return a session cookie header value.
async fn admin_cookie(app: &axum::Router) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username": "admin", "password": "test-admin-pw-123"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

pub async fn test_app() -> axum::Router {
    let (app, _pool) = test_app_with_pool().await;
    app
}

pub async fn test_app_with_pool() -> (axum::Router, sqlx::SqlitePool) {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    ensure_initial_admin(&pool, "admin", "test-admin-pw-123").await;
    let app = routes::app(pool.clone());
    (app, pool)
}

pub async fn request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let cookie = admin_cookie(&app).await;
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, cookie);
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };

    let response = app.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let json = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (status, json)
}

pub async fn register_node(app: axum::Router) -> String {
    let (status, json) = request(
        app,
        "POST",
        "/api/agent/register",
        Some(json!({
            "name": "node-a",
            "hostname": "gpu-node-a",
            "agent_version": "0.1.0",
            "os": "linux",
            "arch": "x86_64"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    json["node_id"].as_str().unwrap().to_string()
}

pub async fn register_second_node_json(app: axum::Router) -> Value {
    let (status, json) = request(
        app,
        "POST",
        "/api/agent/register",
        Some(json!({
            "name": "node-b",
            "hostname": "gpu-node-b",
            "agent_version": "0.1.0",
            "os": "linux",
            "arch": "x86_64"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    json
}

pub async fn register_node_json(app: axum::Router) -> Value {
    let (status, json) = request(
        app,
        "POST",
        "/api/agent/register",
        Some(json!({
            "name": "node-a",
            "hostname": "gpu-node-a",
            "agent_version": "0.1.0",
            "os": "linux",
            "arch": "x86_64"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    json
}

pub async fn heartbeat_node(app: axum::Router, node_id: &str, token: &str) {
    heartbeat_node_with_managed_instances(app, node_id, token, json!([])).await;
}

pub async fn heartbeat_node_with_managed_instances(
    app: axum::Router,
    node_id: &str,
    token: &str,
    managed_instances: Value,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/heartbeat")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "sampled_at": now,
                        "metrics": {},
                        "gpus": [],
                        "collector_errors": [],
                        "collector_status": "no_collector_configured",
                        "agent_config": {
                            "config_version": 1,
                            "heartbeat_interval_secs": 15,
                            "metrics_sample_interval_secs": 30,
                            "task_poll_interval_secs": 20,
                            "config_refresh_interval_secs": 60,
                            "command_timeout_secs": 5,
                            "environment_check_timeout_secs": 5,
                            "last_config_updated_at": 1_700_000_000i64
                        },
                        "managed_instances": managed_instances
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "heartbeat failed: {}",
        String::from_utf8_lossy(&body)
    );
}

pub async fn poll_agent_task(app: axum::Router, node_id: &str, token: &str) -> Value {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/tasks/poll")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(json!({ "node_id": node_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

pub async fn report_agent_task_result(
    app: axum::Router,
    node_id: &str,
    token: &str,
    task_id: &str,
    file_status: &str,
    size_bytes: Option<i64>,
    message: &str,
) {
    let status = if file_status == "verified" {
        "succeeded"
    } else {
        "failed"
    };
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": status,
                        "result": {
                            "file_status": file_status,
                            "size_bytes": size_bytes,
                            "path_type": if size_bytes.is_some() { "file" } else { "directory" },
                            "message": message
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "task result failed: {}",
        String::from_utf8_lossy(&body)
    );
}

pub async fn report_cleanup_task_result(
    app: axum::Router,
    node_id: &str,
    token: &str,
    task_id: &str,
    cleanup_status: &str,
    message: &str,
) {
    let status = if cleanup_status == "deleted" {
        "succeeded"
    } else {
        "failed"
    };
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": status,
                        "result": {
                            "cleanup_status": cleanup_status,
                            "message": message
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "cleanup task result failed: {}",
        String::from_utf8_lossy(&body)
    );
}

pub async fn create_runtime_environment(app: axum::Router, node_id: &str, token: &str) -> Value {
    heartbeat_node(app.clone(), node_id, token).await;
    let uri = format!("/api/nodes/{node_id}/runtime-environments");
    let create_request = request(
        app.clone(),
        "POST",
        &uri,
        Some(json!({
            "name": "Ollama Local",
            "backend": "ollama",
            "deploy_type": "binary",
            "binary_path": "/usr/local/bin/ollama",
            "enabled": true
        })),
    );
    let agent_app = app.clone();
    let agent = async {
        let task = poll_agent_task(agent_app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        assert_eq!(task["task"]["kind"], "check_runtime_environment");
        report_runtime_check_result(
            agent_app,
            node_id,
            token,
            task_id,
            "available",
            Some("0.5.0"),
            "runtime environment available",
        )
        .await;
    };
    let ((status, json), _) = tokio::join!(create_request, agent);

    assert_eq!(status, StatusCode::OK);
    json
}

pub async fn report_runtime_check_result(
    app: axum::Router,
    node_id: &str,
    token: &str,
    task_id: &str,
    check_status: &str,
    version: Option<&str>,
    message: &str,
) {
    let status = if matches!(check_status, "available" | "version_unavailable") {
        "succeeded"
    } else {
        "failed"
    };
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": status,
                        "result": {
                            "check_status": check_status,
                            "version": version,
                            "message": message
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

pub async fn report_instance_task_result(
    app: axum::Router,
    node_id: &str,
    token: &str,
    task_id: &str,
    instance_status: &str,
    message: &str,
) {
    let status = if matches!(instance_status, "running" | "stopped") {
        "succeeded"
    } else {
        "failed"
    };
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": status,
                        "result": {
                            "instance_status": instance_status,
                            "message": message
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

pub async fn report_instance_task_result_with_details(
    app: axum::Router,
    node_id: &str,
    token: &str,
    task_id: &str,
    result: Value,
) {
    let instance_status = result["instance_status"].as_str().unwrap_or("failed");
    let status = if matches!(instance_status, "running" | "stopped") {
        "succeeded"
    } else {
        "failed"
    };
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": status,
                        "result": result
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

pub async fn create_model(app: axum::Router) -> Value {
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    create_model_for_node(app, node_id, token).await
}

pub async fn create_model_for_node(app: axum::Router, node_id: &str, token: &str) -> Value {
    let (status, json) = create_model_for_node_with_agent_result(
        app,
        node_id,
        token,
        "/models/qwen2-7b/model.gguf",
        "verified",
        Some(1234),
        "file verified",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    json
}

pub async fn create_model_for_node_with_agent_result(
    app: axum::Router,
    node_id: &str,
    token: &str,
    path: &str,
    file_status: &str,
    size_bytes: Option<i64>,
    message: &str,
) -> (StatusCode, Value) {
    heartbeat_node(app.clone(), node_id, token).await;
    let create_request = request(
        app.clone(),
        "POST",
        "/api/models",
        Some(json!({
            "name": "qwen2-7b",
            "display_name": "Qwen2 7B",
            "model_type": "llm",
            "description": "test model",
            "default_backend": "ollama",
            "initial_file": {
                "node_id": node_id,
                "path": path
            }
        })),
    );
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        report_agent_task_result(
            app,
            node_id,
            token,
            task_id,
            file_status,
            size_bytes,
            message,
        )
        .await;
    };
    let (created, _) = tokio::join!(create_request, agent);
    created
}

pub async fn create_model_file(
    app: axum::Router,
    model_id: &str,
    node_id: &str,
    token: &str,
    path: &str,
) -> Value {
    let (status, json) = create_model_file_with_agent_result(
        app,
        model_id,
        node_id,
        token,
        path,
        "verified",
        Some(4321),
        "file verified",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    json
}

#[allow(clippy::too_many_arguments)]
pub async fn create_model_file_with_agent_result(
    app: axum::Router,
    model_id: &str,
    node_id: &str,
    token: &str,
    path: &str,
    file_status: &str,
    size_bytes: Option<i64>,
    message: &str,
) -> (StatusCode, Value) {
    heartbeat_node(app.clone(), node_id, token).await;
    let uri = format!("/api/models/{model_id}/files");
    let create_request = request(
        app.clone(),
        "POST",
        &uri,
        Some(json!({
            "node_id": node_id,
            "path": path
        })),
    );
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        report_agent_task_result(
            app,
            node_id,
            token,
            task_id,
            file_status,
            size_bytes,
            message,
        )
        .await;
    };
    let (created, _) = tokio::join!(create_request, agent);
    created
}

pub async fn create_model_with_path(app: axum::Router, model_path: Option<&str>) -> Value {
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let mut payload = json!({
        "name": format!("model-{}", model_path.unwrap_or("without-path").replace('/', "-")),
        "display_name": "Model",
        "model_type": "llm",
        "description": "test model",
        "initial_file": {
            "node_id": node_id,
            "path": "/models/test/model.gguf"
        }
    });
    if let Some(model_path) = model_path {
        payload["model_path"] = json!(model_path);
    }

    heartbeat_node(app.clone(), node_id, token).await;
    let create_request = request(app.clone(), "POST", "/api/models", Some(payload));
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        report_agent_task_result(
            app,
            node_id,
            token,
            task_id,
            "verified",
            Some(1234),
            "file verified",
        )
        .await;
    };
    let ((status, json), _) = tokio::join!(create_request, agent);
    assert_eq!(status, StatusCode::OK);
    json
}

pub async fn register_with_name_hostname(
    app: axum::Router,
    name: &str,
    hostname: &str,
) -> (StatusCode, Value) {
    request(
        app,
        "POST",
        "/api/agent/register",
        Some(json!({
            "name": name,
            "hostname": hostname,
            "agent_version": "0.1.0",
            "os": "linux",
            "arch": "x86_64"
        })),
    )
    .await
}

pub async fn stage2_test_app() -> (sqlx::SqlitePool, axum::Router) {
    let (app, pool) = test_app_with_pool().await;
    (pool, app)
}
