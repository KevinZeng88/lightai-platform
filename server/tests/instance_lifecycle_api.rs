use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use lightai_server::{db, routes};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn test_app() -> axum::Router {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    routes::app(pool)
}

async fn test_app_with_pool() -> (axum::Router, sqlx::SqlitePool) {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    (routes::app(pool.clone()), pool)
}

async fn request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
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

async fn register_node(app: axum::Router) -> String {
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

async fn register_second_node_json(app: axum::Router) -> Value {
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

async fn register_node_json(app: axum::Router) -> Value {
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

async fn heartbeat_node(app: axum::Router, node_id: &str, token: &str) {
    heartbeat_node_with_managed_instances(app, node_id, token, json!([])).await;
}

async fn heartbeat_node_with_managed_instances(
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

async fn poll_agent_task(app: axum::Router, node_id: &str, token: &str) -> Value {
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

async fn report_agent_task_result(
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

async fn report_cleanup_task_result(
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

async fn create_runtime_environment(app: axum::Router, node_id: &str, token: &str) -> Value {
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
            "运行环境可用",
        )
        .await;
    };
    let ((status, json), _) = tokio::join!(create_request, agent);

    assert_eq!(status, StatusCode::OK);
    json
}

async fn report_runtime_check_result(
    app: axum::Router,
    node_id: &str,
    token: &str,
    task_id: &str,
    check_status: &str,
    version: Option<&str>,
    message: &str,
) {
    let status = if check_status == "available" {
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

async fn report_instance_task_result(
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

async fn report_instance_task_result_with_details(
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

async fn create_model(app: axum::Router) -> Value {
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    create_model_for_node(app, node_id, token).await
}

async fn create_model_for_node(app: axum::Router, node_id: &str, token: &str) -> Value {
    let (status, json) = create_model_for_node_with_agent_result(
        app,
        node_id,
        token,
        "/models/qwen2-7b/model.gguf",
        "verified",
        Some(1234),
        "文件已验证",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    json
}

async fn create_model_for_node_with_agent_result(
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

async fn create_model_file(
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
        "文件已验证",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    json
}

#[allow(clippy::too_many_arguments)]
async fn create_model_file_with_agent_result(
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

async fn create_model_with_path(app: axum::Router, model_path: Option<&str>) -> Value {
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
            "path": "/models/legacy/model.gguf"
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
            "文件已验证",
        )
        .await;
    };
    let ((status, json), _) = tokio::join!(create_request, agent);
    assert_eq!(status, StatusCode::OK);
    json
}

#[tokio::test]
async fn model_create_requires_initial_node_file_path() {
    let app = test_app().await;

    let (status, json) = request(
        app,
        "POST",
        "/api/models",
        Some(json!({
            "name": "qwen-without-file",
            "model_type": "llm"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "bad_request");
    assert!(json["message"]
        .as_str()
        .unwrap()
        .contains("initial_file is required"));
}

#[tokio::test]
async fn model_create_also_creates_first_node_file_path() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();

    let model = create_model_for_node(app.clone(), node_id, token).await;
    assert_eq!(model["file_status"], "all_files_verified");
    assert_eq!(model["total_file_count"], 1);

    let (status, files) = request(
        app,
        "GET",
        &format!("/api/models/{}/files", model["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let files = files["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["node_id"], node_id);
    assert_eq!(files[0]["path"], "/models/qwen2-7b/model.gguf");
    assert_eq!(files[0]["status"], "verified");
    assert_eq!(files[0]["size_bytes"], 1234);
}

#[tokio::test]
async fn key_operations_create_audit_events() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();

    let model = create_model_for_node(app.clone(), node_id, token).await;

    let (status, audit) = request(app, "GET", "/api/audit-events?target_type=model", None).await;
    assert_eq!(status, StatusCode::OK);
    let events = audit["events"].as_array().unwrap();
    assert!(events.iter().any(|event| {
        event["operation_type"] == "model.create"
            && event["target_id"] == model["id"]
            && event["actor_type"] == "system"
            && event["actor_id"] == "local"
            && event["result"] == "success"
    }));
}

#[tokio::test]
async fn model_create_fails_when_agent_reports_missing_file_and_does_not_insert_model() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();

    let (status, json) = create_model_for_node_with_agent_result(
        app.clone(),
        node_id,
        token,
        "/models/missing.gguf",
        "missing",
        None,
        "文件不存在",
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["message"], "文件不存在");
    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["models"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn model_create_fails_immediately_when_agent_is_offline() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();

    let (status, json) = request(
        app.clone(),
        "POST",
        "/api/models",
        Some(json!({
            "name": "qwen2-7b",
            "display_name": "Qwen2 7B",
            "model_type": "llm",
            "initial_file": {
                "node_id": node_id,
                "path": "/models/qwen2-7b/model.gguf"
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["message"], "节点 Agent 离线，无法验证模型文件");
    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["models"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn model_create_fails_when_agent_does_not_return_verification_before_timeout() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;

    let (status, json) = request(
        app.clone(),
        "POST",
        "/api/models",
        Some(json!({
            "name": "qwen2-7b",
            "display_name": "Qwen2 7B",
            "model_type": "llm",
            "initial_file": {
                "node_id": node_id,
                "path": "/models/qwen2-7b/model.gguf"
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["message"], "模型文件验证超时，请确认 Agent 在线并重试");
    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["models"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn soft_deleted_model_name_does_not_surface_raw_unique_constraint() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/models/{model_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let recreated = create_model_for_node(app.clone(), node_id, token).await;
    assert_eq!(recreated["name"], "qwen2-7b");

    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    let models = models["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], recreated["id"]);
}

#[tokio::test]
async fn runtime_environments_can_be_listed_by_node_and_globally() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let created = create_runtime_environment(app.clone(), node_id, token).await;

    assert_eq!(created["node_id"], node_id);
    assert_eq!(created["backend"], "ollama");
    assert_eq!(created["deploy_type"], "binary");
    assert_eq!(created["version"], "0.5.0");
    assert_eq!(created["check_status"], "available");

    let (status, by_node) = request(
        app.clone(),
        "GET",
        &format!("/api/nodes/{node_id}/runtime-environments"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(by_node["runtime_environments"].as_array().unwrap().len(), 1);

    let (status, global) = request(app, "GET", "/api/runtime-environments", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(global["runtime_environments"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn model_files_are_registered_per_node_and_drive_model_file_status() {
    let app = test_app().await;
    let registered_a = register_node_json(app.clone()).await;
    let node_a = registered_a["node_id"].as_str().unwrap();
    let token_a = registered_a["agent_token"].as_str().unwrap();
    let registered_b = register_second_node_json(app.clone()).await;
    let node_b = registered_b["node_id"].as_str().unwrap();
    let token_b = registered_b["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_a, token_a).await;
    let model_id = model["id"].as_str().unwrap();

    let file_b =
        create_model_file(app.clone(), model_id, node_b, token_b, "/models/qwen.gguf").await;

    assert_eq!(file_b["node_id"], node_b);
    assert_eq!(file_b["status"], "verified");

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(files["files"].as_array().unwrap().len(), 2);

    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = &models["models"].as_array().unwrap()[0];
    assert_eq!(listed["file_status"], "all_files_verified");
    assert_eq!(listed["verified_file_count"], 2);
    assert_eq!(listed["total_file_count"], 2);
}

#[tokio::test]
async fn model_file_create_fails_when_agent_verification_fails_and_does_not_insert_path() {
    let app = test_app().await;
    let registered_a = register_node_json(app.clone()).await;
    let node_a = registered_a["node_id"].as_str().unwrap();
    let token_a = registered_a["agent_token"].as_str().unwrap();
    let registered_b = register_second_node_json(app.clone()).await;
    let node_b = registered_b["node_id"].as_str().unwrap();
    let token_b = registered_b["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_a, token_a).await;
    let model_id = model["id"].as_str().unwrap();

    let (status, json) = create_model_file_with_agent_result(
        app.clone(),
        model_id,
        node_b,
        token_b,
        "/models/missing.gguf",
        "missing",
        None,
        "文件不存在",
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["message"], "文件不存在");
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(files["files"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_last_model_file_moves_path_to_trash() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let file_id = files["files"].as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap();

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/model-files/{file_id}"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(files["files"].as_array().unwrap().len(), 0);

    let (status, trash_list) = request(app.clone(), "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["model_file_id"], file_id);
    assert_eq!(items[0]["model_id"], model_id);
    assert_eq!(items[0]["path"], "/models/qwen2-7b/model.gguf");

    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        models["models"].as_array().unwrap()[0]["file_status"],
        "no_files"
    );
}

#[tokio::test]
async fn deleting_one_of_multiple_model_files_is_allowed() {
    let app = test_app().await;
    let registered_a = register_node_json(app.clone()).await;
    let node_a = registered_a["node_id"].as_str().unwrap();
    let token_a = registered_a["agent_token"].as_str().unwrap();
    let registered_b = register_second_node_json(app.clone()).await;
    let node_b = registered_b["node_id"].as_str().unwrap();
    let token_b = registered_b["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_a, token_a).await;
    let model_id = model["id"].as_str().unwrap();
    let file_b = create_model_file(
        app.clone(),
        model_id,
        node_b,
        token_b,
        "/models/qwen-b.gguf",
    )
    .await;
    let file_b_id = file_b["id"].as_str().unwrap();

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/model-files/{file_b_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let files = files["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["node_id"], node_a);

    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["model_file_id"], file_b_id);
    assert_eq!(items[0]["path"], "/models/qwen-b.gguf");
}

#[tokio::test]
async fn agent_task_verifies_model_file_and_updates_model_file_status() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let file_id = files["files"].as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap();

    let (status, pending_file) = request(
        app.clone(),
        "POST",
        &format!("/api/model-files/{file_id}/verify"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pending_file["status"], "verify_pending");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/tasks/poll")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let task: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(task["task"]["kind"], "verify_model_file");
    assert_eq!(task["task"]["payload"]["model_file_id"], file_id);
    let task_id = task["task"]["id"].as_str().unwrap();

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(files["files"].as_array().unwrap()[0]["status"], "verifying");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": "succeeded",
                        "result": {
                            "file_status": "verified",
                            "size_bytes": 1234,
                            "message": "文件已验证"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let verified = &files["files"].as_array().unwrap()[0];
    assert_eq!(verified["status"], "verified");
    assert_eq!(verified["size_bytes"], 1234);
    assert_eq!(verified["last_error"], Value::Null);

    let (status, model) = request(app, "GET", &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(model["file_status"], "all_files_verified");
    assert_eq!(model["verified_file_count"], 1);
}

#[tokio::test]
async fn verification_task_timeout_updates_model_file_status_when_no_agent_reports_back() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let file_id = files["files"].as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap();

    let (status, pending_file) = request(
        app.clone(),
        "POST",
        &format!("/api/model-files/{file_id}/verify"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pending_file["status"], "verify_pending");

    sqlx::query("UPDATE agent_tasks SET created_at = 1, updated_at = 1")
        .execute(&pool)
        .await
        .unwrap();

    let (status, files) = request(app, "GET", &format!("/api/models/{model_id}/files"), None).await;
    assert_eq!(status, StatusCode::OK);
    let timed_out = &files["files"].as_array().unwrap()[0];
    assert_eq!(timed_out["status"], "verify_timeout");
    assert_eq!(timed_out["last_error"], "验证超时");
}

#[tokio::test]
async fn failed_model_file_verification_keeps_error_and_marks_model_unverified() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let file_id = files["files"].as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap();

    let (status, _) = request(
        app.clone(),
        "POST",
        &format!("/api/model-files/{file_id}/verify"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let response = app
        .clone()
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
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let task: Value = serde_json::from_slice(&body).unwrap();
    let task_id = task["task"]["id"].as_str().unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": "failed",
                        "result": {
                            "file_status": "missing",
                            "message": "文件不存在"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let missing = &files["files"].as_array().unwrap()[0];
    assert_eq!(missing["status"], "missing");
    assert_eq!(missing["last_error"], "文件不存在");

    let (status, model) = request(app, "GET", &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(model["file_status"], "verification_failed");
}

#[tokio::test]
async fn external_instance_delete_does_not_create_trash_record() {
    let app = test_app().await;
    let (status, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "name": "external service",
            "model_name": "served-model",
            "base_url": "http://127.0.0.1:8088"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/model-instances/{}", instance["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trash_list["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn model_delete_is_soft_and_rejects_running_instances() {
    let app = test_app().await;
    let model = create_model(app.clone()).await;

    let (status, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "model_id": model["id"],
            "name": "qwen2 external",
            "backend": "ollama",
            "base_url": "http://127.0.0.1:11434",
            "endpoint_url": "http://127.0.0.1:11434/v1",
            "health_url": "http://127.0.0.1:11434/api/tags",
            "model_name": "qwen2",
            "status": "running"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(instance["status"], "running");

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/models/{}", model["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = request(
        app.clone(),
        "PUT",
        &format!("/api/model-instances/{}", instance["id"].as_str().unwrap()),
        Some(json!({
            "status": "stopped"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/models/{}", model["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, models) = request(app.clone(), "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["models"].as_array().unwrap().len(), 0);

    let (status, instances) = request(app, "GET", "/api/model-instances", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(instances["model_instances"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_model_moves_all_node_file_paths_to_trash() {
    let app = test_app().await;
    let registered_a = register_node_json(app.clone()).await;
    let node_a = registered_a["node_id"].as_str().unwrap();
    let token_a = registered_a["agent_token"].as_str().unwrap();
    let registered_b = register_second_node_json(app.clone()).await;
    let node_b = registered_b["node_id"].as_str().unwrap();
    let token_b = registered_b["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_a, token_a).await;
    let model_id = model["id"].as_str().unwrap();
    create_model_file(
        app.clone(),
        model_id,
        node_b,
        token_b,
        "/models/qwen-node-b.gguf",
    )
    .await;

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/models/{model_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, models) = request(app.clone(), "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["models"].as_array().unwrap().len(), 0);

    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let mut paths = items
        .iter()
        .map(|item| item["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    assert_eq!(
        paths,
        vec!["/models/qwen-node-b.gguf", "/models/qwen2-7b/model.gguf"]
    );
    assert!(items.iter().all(|item| item["status"] == "pending"));
    assert!(items
        .iter()
        .all(|item| item["file_deleted_at"] == Value::Null));
}

#[tokio::test]
async fn model_file_trash_records_specific_node_file_path() {
    let app = test_app().await;
    let model = create_model(app.clone()).await;
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{}/files", model["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let model_file = &files["files"].as_array().unwrap()[0];

    let (status, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({
            "reason": "manual cleanup later",
            "note": "do not physically delete in Stage 3A"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trash["status"], "pending");

    let (status, trash_list) = request(app.clone(), "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["model_file_id"], model_file["id"]);
    assert_eq!(items[0]["model_name"], "qwen2-7b");
    assert_eq!(items[0]["node_id"], model_file["node_id"]);
    assert_eq!(items[0]["path"], "/models/qwen2-7b/model.gguf");
    assert_eq!(items[0]["file_deleted_at"], Value::Null);

    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["models"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn trash_cleanup_file_is_executed_by_matching_agent_and_updates_status() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file = &files["files"].as_array().unwrap()[0];
    let (status, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({ "reason": "cleanup" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let trash_id = trash["id"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;

    let cleanup_uri = format!("/api/model-file-trash/{trash_id}/cleanup");
    let cleanup_request = request(app.clone(), "POST", &cleanup_uri, None);
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        assert_eq!(task["task"]["kind"], "cleanup_model_file");
        assert_eq!(task["task"]["node_id"], node_id);
        assert_eq!(task["task"]["payload"]["trash_id"], trash_id);
        assert_eq!(task["task"]["payload"]["path"], model_file["path"]);
        let task_id = task["task"]["id"].as_str().unwrap();
        report_cleanup_task_result(app, node_id, token, task_id, "deleted", "文件已清理").await;
    };
    let ((status, cleaned), _) = tokio::join!(cleanup_request, agent);

    assert_eq!(status, StatusCode::OK);
    assert_eq!(cleaned["status"], "cleaned");
    assert_ne!(cleaned["file_deleted_at"], Value::Null);
    assert_eq!(cleaned["last_error"], Value::Null);
}

#[tokio::test]
async fn trash_cleanup_file_fails_when_agent_is_offline_and_keeps_record() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file = &files["files"].as_array().unwrap()[0];
    let (_, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({ "reason": "cleanup" })),
    )
    .await;
    let trash_id = trash["id"].as_str().unwrap();
    sqlx::query("UPDATE nodes SET last_heartbeat_at = 1 WHERE id = ?")
        .bind(node_id)
        .execute(&pool)
        .await
        .unwrap();

    let (status, json) = request(
        app.clone(),
        "POST",
        &format!("/api/model-file-trash/{trash_id}/cleanup"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["message"], "节点 Agent 离线，无法清理文件");
    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let item = &trash_list["items"].as_array().unwrap()[0];
    assert_eq!(item["status"], "cleanup_failed");
    assert_eq!(item["last_error"], "节点 Agent 离线，无法清理文件");
}

#[tokio::test]
async fn trash_cleanup_file_failure_keeps_record_and_error() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file = &files["files"].as_array().unwrap()[0];
    let (_, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({ "reason": "cleanup" })),
    )
    .await;
    let trash_id = trash["id"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;

    let cleanup_uri = format!("/api/model-file-trash/{trash_id}/cleanup");
    let cleanup_request = request(app.clone(), "POST", &cleanup_uri, None);
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        report_cleanup_task_result(app.clone(), node_id, token, task_id, "failed", "文件不存在")
            .await;
    };
    let ((status, json), _) = tokio::join!(cleanup_request, agent);

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["message"], "文件不存在");
    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let item = &trash_list["items"].as_array().unwrap()[0];
    assert_eq!(item["status"], "cleanup_failed");
    assert_eq!(item["last_error"], "文件不存在");
}

#[tokio::test]
async fn trash_record_delete_removes_only_platform_record() {
    let app = test_app().await;
    let model = create_model(app.clone()).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file = &files["files"].as_array().unwrap()[0];
    let (_, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({ "reason": "remove record" })),
    )
    .await;
    let trash_id = trash["id"].as_str().unwrap();

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/model-file-trash/{trash_id}"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trash_list["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn external_instance_does_not_require_node_or_runtime_environment() {
    let app = test_app().await;
    let model = create_model(app.clone()).await;

    let (status, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "model_id": model["id"],
            "name": "llama.cpp local",
            "backend": "llama_cpp",
            "runtime_version": "b4000",
            "base_url": "http://127.0.0.1:8088",
            "endpoint_url": "http://127.0.0.1:8088/v1",
            "health_url": "http://127.0.0.1:8088/v1/models",
            "model_name": "local-gguf",
            "description": "existing llama-server"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(instance["node_id"], Value::Null);
    assert_eq!(instance["runtime_environment_id"], Value::Null);
    assert_eq!(instance["backend"], "llama_cpp");
    assert_eq!(instance["deploy_type"], "external");
    assert_eq!(instance["model_name"], "local-gguf");
}

#[tokio::test]
async fn external_instance_can_be_created_minimally_without_model_or_backend() {
    let app = test_app().await;

    let (status, instance) = request(
        app,
        "POST",
        "/api/model-instances",
        Some(json!({
            "name": "llama.cpp local",
            "model_name": "local-gguf",
            "base_url": "http://127.0.0.1:8088"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(instance["model_id"], Value::Null);
    assert_eq!(instance["node_id"], Value::Null);
    assert_eq!(instance["runtime_environment_id"], Value::Null);
    assert_eq!(instance["backend"], "custom");
    assert_eq!(instance["deploy_type"], "external");
    assert_eq!(instance["status"], "unknown");
}

#[tokio::test]
async fn external_instance_requires_base_url() {
    let app = test_app().await;
    let (status, instance) = request(
        app,
        "POST",
        "/api/model-instances",
        Some(json!({
            "name": "qwen2 external",
            "model_name": "qwen2"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(instance["error"], "bad_request");
}

#[tokio::test]
async fn docker_or_script_runtime_environment_requires_online_agent() {
    let app = test_app().await;
    let node_id = register_node(app.clone()).await;

    let (status, json) = request(
        app,
        "POST",
        &format!("/api/nodes/{node_id}/runtime-environments"),
        Some(json!({
            "name": "Node Ollama",
            "backend": "ollama",
            "deploy_type": "script",
            "binary_path": "/usr/local/bin/ollama",
            "enabled": true
        })),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["error"], "conflict");
    assert!(json["message"]
        .as_str()
        .unwrap()
        .contains("Agent is offline"));
}

#[tokio::test]
async fn runtime_environment_create_requires_agent_check_success() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;

    let uri = format!("/api/nodes/{node_id}/runtime-environments");
    let create_request = request(
        app.clone(),
        "POST",
        &uri,
        Some(json!({
            "name": "Bad Script",
            "backend": "custom",
            "deploy_type": "script",
            "binary_path": "/opt/lightai/run-model",
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
            "unavailable",
            None,
            "脚本不存在或不可访问",
        )
        .await;
    };
    let ((status, json), _) = tokio::join!(create_request, agent);

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["message"], "脚本不存在或不可访问");

    let (status, list) = request(app, "GET", "/api/runtime-environments", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["runtime_environments"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn local_instance_uses_verified_model_file_and_agent_start_stop_tasks() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let model_file_id = files["files"][0]["id"].as_str().unwrap();

    let (status, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local",
            "name": "qwen local",
            "node_id": node_id,
            "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(instance["deploy_type"], "local");
    assert_eq!(instance["status"], "stopped");
    assert_eq!(instance["model_file_id"], model_file_id);

    let instance_id = instance["id"].as_str().unwrap();
    let start_uri = format!("/api/model-instances/{instance_id}/start");
    let start_request = request(app.clone(), "POST", &start_uri, None);
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        assert_eq!(task["task"]["kind"], "start_model_instance");
        report_instance_task_result(
            app.clone(),
            node_id,
            token,
            task_id,
            "running",
            "实例已启动",
        )
        .await;
    };
    let ((status, started), _) = tokio::join!(start_request, agent);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(started["status"], "running");

    let stop_uri = format!("/api/model-instances/{instance_id}/stop");
    let stop_request = request(app.clone(), "POST", &stop_uri, None);
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        assert_eq!(task["task"]["kind"], "stop_model_instance");
        report_instance_task_result(app, node_id, token, task_id, "stopped", "实例已停止").await;
    };
    let ((status, stopped), _) = tokio::join!(stop_request, agent);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stopped["status"], "stopped");
}

#[tokio::test]
async fn local_instance_failure_persists_log_tail_and_command_summary() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let model_file_id = files["files"][0]["id"].as_str().unwrap();

    let (status, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local",
            "name": "qwen local failure",
            "node_id": node_id,
            "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id,
            "params_json": "{\"host\":\"127.0.0.1\",\"port\":18083}"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let instance_id = instance["id"].as_str().unwrap();

    let start_uri = format!("/api/model-instances/{instance_id}/start");
    let start_request = request(app.clone(), "POST", &start_uri, None);
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        assert_eq!(task["task"]["kind"], "start_model_instance");
        report_instance_task_result_with_details(
            app.clone(),
            node_id,
            token,
            task_id,
            json!({
                "instance_status": "failed",
                "message": "启动进程已退出：main: exiting due to HTTP server error",
                "log_tail": "stderr:\nmain: exiting due to HTTP server error",
                "command": "[\"/usr/local/bin/llama-server\",\"-m\",\"/models/qwen2-7b/model.gguf\"]"
            }),
        )
        .await;
    };
    let ((status, json), _) = tokio::join!(start_request, agent);
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(json["message"]
        .as_str()
        .unwrap()
        .contains("HTTP server error"));

    let (status, fetched) = request(
        app,
        "GET",
        &format!("/api/model-instances/{instance_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["status"], "failed");
    assert!(fetched["last_error"]
        .as_str()
        .unwrap()
        .contains("HTTP server error"));
    assert!(fetched["log_tail"]
        .as_str()
        .unwrap()
        .contains("HTTP server error"));
    assert!(fetched["command"]
        .as_str()
        .unwrap()
        .contains("llama-server"));
}

#[tokio::test]
async fn model_directory_can_be_registered_and_used_by_local_instance_params() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let (status, model) = create_model_for_node_with_agent_result(
        app.clone(),
        node_id,
        token,
        "/models/qwen2-7b",
        "verified",
        None,
        "目录已验证",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let model_id = model["id"].as_str().unwrap();
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(files["files"][0]["path_type"], "directory");
    let model_file_id = files["files"][0]["id"].as_str().unwrap();

    let (status, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local",
            "name": "qwen local params",
            "node_id": node_id,
            "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id,
            "params_json": json!({
                "host": "127.0.0.1",
                "port": 18081,
                "ctx_size": 4096,
                "gpu_layers": 20,
                "threads": 8,
                "extra_args": ["--verbose"]
            }).to_string()
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let instance_id = instance["id"].as_str().unwrap();

    let start_uri = format!("/api/model-instances/{instance_id}/start");
    let start_request = request(app.clone(), "POST", &start_uri, None);
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        assert_eq!(task["task"]["kind"], "start_model_instance");
        assert_eq!(task["task"]["payload"]["model_path"], "/models/qwen2-7b");
        assert_eq!(task["task"]["payload"]["model_path_type"], "directory");
        assert_eq!(task["task"]["payload"]["params"]["port"], 18081);
        assert_eq!(
            task["task"]["payload"]["params"]["extra_args"][0],
            "--verbose"
        );
        let task_id = task["task"]["id"].as_str().unwrap();
        report_instance_task_result_with_details(
            app.clone(),
            node_id,
            token,
            task_id,
            json!({
                "instance_status": "running",
                "message": "实例已启动",
                "base_url": "http://127.0.0.1:18081",
                "endpoint_url": "http://127.0.0.1:18081",
                "process_id": 12345,
                "process_ref": instance_id
            }),
        )
        .await;
    };
    let ((status, started), _) = tokio::join!(start_request, agent);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(started["status"], "running");
    assert_eq!(started["base_url"], "http://127.0.0.1:18081");
    assert_eq!(started["endpoint_url"], "http://127.0.0.1:18081");
    assert_eq!(started["process_id"], 12345);
}

#[tokio::test]
async fn local_instance_test_uses_agent_task_and_preserves_external_check() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let model_file_id = files["files"][0]["id"].as_str().unwrap();
    let (status, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local",
            "name": "qwen local test",
            "node_id": node_id,
            "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let instance_id = instance["id"].as_str().unwrap();
    sqlx::query("UPDATE model_instances SET status = 'running', base_url = 'http://127.0.0.1:18082', endpoint_url = 'http://127.0.0.1:18082' WHERE id = ?")
        .bind(instance_id)
        .execute(&pool)
        .await
        .unwrap();

    let test_uri = format!("/api/model-instances/{instance_id}/test");
    let test_request = request(app.clone(), "POST", &test_uri, None);
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        assert_eq!(task["task"]["kind"], "test_model_instance");
        assert_eq!(
            task["task"]["payload"]["endpoint_url"],
            "http://127.0.0.1:18082"
        );
        let task_id = task["task"]["id"].as_str().unwrap();
        report_instance_task_result_with_details(
            app.clone(),
            node_id,
            token,
            task_id,
            json!({
                "instance_status": "running",
                "message": "测试成功：HTTP 200 OK",
                "response_summary": "{\"data\":[]}"
            }),
        )
        .await;
    };
    let ((status, tested), _) = tokio::join!(test_request, agent);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tested["status"], "running");
    assert_eq!(tested["last_error"], "测试成功：HTTP 200 OK：{\"data\":[]}");

    let (status, external) = request(
        app,
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "external",
            "name": "external",
            "model_name": "external-model",
            "base_url": "http://127.0.0.1:1"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(external["deploy_type"], "external");
}

#[tokio::test]
async fn heartbeat_reconciles_running_local_instance_missing_from_agent_store() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let model_file_id = files["files"][0]["id"].as_str().unwrap();
    let (status, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local",
            "name": "qwen local restart recovery",
            "node_id": node_id,
            "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let instance_id = instance["id"].as_str().unwrap();
    sqlx::query(
        r#"
        UPDATE model_instances
        SET status = 'running', process_id = 12345, process_ref = ?, base_url = 'http://127.0.0.1:18084'
        WHERE id = ?
        "#,
    )
    .bind(instance_id)
    .bind(instance_id)
    .execute(&pool)
    .await
    .unwrap();

    heartbeat_node_with_managed_instances(app.clone(), node_id, token, json!([])).await;

    let (status, fetched) = request(
        app,
        "GET",
        &format!("/api/model-instances/{instance_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["status"], "failed");
    assert!(fetched["last_error"]
        .as_str()
        .unwrap()
        .contains("Agent 未上报该实例受管进程状态"));
    assert_eq!(fetched["process_id"], Value::Null);
    assert_eq!(fetched["process_ref"], Value::Null);
}

#[tokio::test]
async fn heartbeat_updates_running_local_instance_from_agent_managed_report() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let model_file_id = files["files"][0]["id"].as_str().unwrap();
    let (status, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local",
            "name": "qwen local restart restored",
            "node_id": node_id,
            "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let instance_id = instance["id"].as_str().unwrap();
    sqlx::query("UPDATE model_instances SET status = 'running' WHERE id = ?")
        .bind(instance_id)
        .execute(&pool)
        .await
        .unwrap();

    heartbeat_node_with_managed_instances(
        app.clone(),
        node_id,
        token,
        json!([{
            "instance_id": instance_id,
            "status": "running",
            "message": "Agent 重启后已恢复受管进程状态：受管进程仍在运行",
            "process_id": 23456,
            "process_ref": instance_id,
            "base_url": "http://127.0.0.1:18085",
            "endpoint_url": "http://127.0.0.1:18085",
            "command": "[\"/usr/local/bin/ollama\",\"--model\",\"/models/qwen2-7b/model.gguf\"]",
            "log_path": "/tmp/lightai/instance.log"
        }]),
    )
    .await;

    let (status, fetched) = request(
        app,
        "GET",
        &format!("/api/model-instances/{instance_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["status"], "running");
    assert_eq!(fetched["process_id"], 23456);
    // running 实例不应有 last_error（恢复信息写入 Agent 日志，不入库）
    assert!(fetched["last_error"].as_str().is_none_or(str::is_empty));
    assert!(fetched["log_tail"]
        .as_str()
        .unwrap()
        .contains("/tmp/lightai/instance.log"));
}

#[tokio::test]
async fn runtime_environment_delete_reports_conflict_when_used_by_instance() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let environment = create_runtime_environment(app.clone(), node_id, token).await;

    let (status, _) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local",
            "node_id": node_id,
            "runtime_environment_id": environment["id"],
            "model_file_id": "will-be-rejected-without-file",
            "name": "external via registered environment",
            "model_name": "qwen2"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    sqlx::query(
        r#"
        INSERT INTO model_instances (
            id, model_id, node_id, runtime_environment_id, name, backend,
            deploy_type, status, created_at, updated_at
        )
        VALUES ('inst-1', NULL, ?, ?, 'local instance', 'ollama', 'local', 'stopped', 1, 1)
        "#,
    )
    .bind(node_id)
    .bind(environment["id"].as_str().unwrap())
    .execute(&pool)
    .await
    .unwrap();

    let (status, json) = request(
        app,
        "DELETE",
        &format!(
            "/api/runtime-environments/{}",
            environment["id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["error"], "conflict");
    assert!(json["message"]
        .as_str()
        .unwrap()
        .contains("runtime environment is used by model instances"));
}

#[tokio::test]
async fn deleting_model_with_legacy_model_path_trashes_only_node_file_path() {
    let app = test_app().await;
    let model = create_model_with_path(app.clone(), Some("/models/qwen2-7b")).await;

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/models/{}", model["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["path"], "/models/legacy/model.gguf");
}

#[tokio::test]
async fn deleting_model_without_legacy_model_path_trashes_node_file_path() {
    let app = test_app().await;
    let model = create_model_with_path(app.clone(), None).await;

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/models/{}", model["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["path"], "/models/legacy/model.gguf");
}

#[tokio::test]
async fn agent_register_and_heartbeat_exchange_config_and_report_effective_config() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();

    assert_eq!(registered["agent_config"]["config_version"], 1);
    assert_eq!(registered["agent_config"]["heartbeat_interval_secs"], 15);
    assert_eq!(
        registered["agent_config"]["metrics_sample_interval_secs"],
        15
    );
    assert_eq!(registered["agent_config"]["task_poll_interval_secs"], 15);

    let (status, _) = request(
        app.clone(),
        "POST",
        "/api/agent/heartbeat",
        Some(json!({
            "node_id": node_id,
            "sampled_at": 1_700_000_000i64,
            "metrics": {},
            "gpus": [],
            "collector_errors": [],
            "agent_config": {
                "config_version": 1,
                "heartbeat_interval_secs": 15,
                "metrics_sample_interval_secs": 30,
                "task_poll_interval_secs": 20,
                "config_refresh_interval_secs": 60,
                "command_timeout_secs": 5,
                "environment_check_timeout_secs": 5,
                "last_config_updated_at": 1_700_000_000i64
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/heartbeat")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "sampled_at": 1_700_000_000i64,
                        "metrics": {},
                        "gpus": [],
                        "collector_errors": [],
                        "agent_config": {
                            "config_version": 1,
                            "heartbeat_interval_secs": 15,
                            "metrics_sample_interval_secs": 30,
                            "task_poll_interval_secs": 20,
                            "config_refresh_interval_secs": 60,
                            "command_timeout_secs": 5,
                            "environment_check_timeout_secs": 5,
                            "last_config_updated_at": 1_700_000_000i64
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let heartbeat: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(heartbeat["agent_config"]["config_version"], 1);

    let (status, nodes) = request(app, "GET", "/api/nodes", None).await;
    assert_eq!(status, StatusCode::OK);
    let node = &nodes["nodes"].as_array().unwrap()[0];
    assert_eq!(node["agent_config"]["metrics_sample_interval_secs"], 30);
    assert_eq!(node["agent_config"]["task_poll_interval_secs"], 20);
}

// ── register_node 按 name + hostname 复用 node_id ──

#[tokio::test]
async fn register_node_reuses_node_id_for_same_name_and_hostname() {
    let (pool, app) = stage2_test_app().await;
    let first = register_node_json(app.clone()).await;
    let first_id = first["node_id"].as_str().unwrap();
    let first_token = first["agent_token"].as_str().unwrap();

    let second = register_node_json(app.clone()).await;
    assert_eq!(second["node_id"].as_str().unwrap(), first_id);
    assert_ne!(second["agent_token"].as_str().unwrap(), first_token);

    // 确保 DB 只有一条记录
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE id = ? AND name = 'node-a'")
            .bind(first_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

async fn register_with_name_hostname(
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

#[tokio::test]
async fn register_node_rejects_same_name_different_hostname() {
    let (_pool, app) = stage2_test_app().await;
    // 先注册 node-a @ gpu-node-a
    let (status1, _) = register_with_name_hostname(app.clone(), "node-a", "gpu-node-a").await;
    assert_eq!(status1, StatusCode::OK);
    // 相同 name 不同 hostname → 拒绝
    let (status2, body) = register_with_name_hostname(app.clone(), "node-a", "gpu-node-b").await;
    assert_eq!(status2, StatusCode::BAD_REQUEST);
    assert!(body["message"]
        .as_str()
        .unwrap_or("")
        .contains("相同名称不允许用于不同主机"));
}

#[tokio::test]
async fn register_node_rejects_different_name_same_hostname() {
    let (_pool, app) = stage2_test_app().await;
    // 先注册 node-a @ gpu-node-a
    let (status1, _) = register_with_name_hostname(app.clone(), "node-a", "gpu-node-a").await;
    assert_eq!(status1, StatusCode::OK);
    // 不同 name 相同 hostname → 拒绝
    let (status2, body) = register_with_name_hostname(app.clone(), "node-b", "gpu-node-a").await;
    assert_eq!(status2, StatusCode::BAD_REQUEST);
    assert!(body["message"]
        .as_str()
        .unwrap_or("")
        .contains("相同主机不允许用于不同名称"));
}

#[tokio::test]
async fn register_node_creates_new_node_for_different_name() {
    let (_pool, app) = stage2_test_app().await;
    let first = register_node_json(app.clone()).await;
    let second = register_second_node_json(app.clone()).await;
    assert_ne!(second["node_id"], first["node_id"]);
}

// ── check_model_instance 在 Agent 离线时返回 last_error ──

#[tokio::test]
async fn check_model_instance_returns_last_error_when_agent_offline() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    // 先发一次心跳让节点上线，否则无法创建运行环境和模型文件
    heartbeat_node_with_managed_instances(app.clone(), node_id, token, json!([])).await;
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file_id = files["files"][0]["id"].as_str().unwrap();
    let (_, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local",
            "name": "offline check test",
            "node_id": node_id,
            "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id
        })),
    )
    .await;
    let instance_id = instance["id"].as_str().unwrap();

    // 设为 running 后清除心跳，模拟 Agent 离线
    sqlx::query("UPDATE model_instances SET status = 'running' WHERE id = ?")
        .bind(instance_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE nodes SET last_heartbeat_at = NULL WHERE id = ?")
        .bind(node_id)
        .execute(&pool)
        .await
        .unwrap();

    let (status, checked) = request(
        app.clone(),
        "POST",
        &format!("/api/model-instances/{instance_id}/check"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(checked["status"], "running");
    assert!(checked["last_error"]
        .as_str()
        .unwrap_or("")
        .contains("离线"));
    assert!(checked["last_checked_at"].is_number());
}

// ── heartbeat 恢复：reports 中有存活实例 → 保持 running ──

#[tokio::test]
async fn heartbeat_reports_keep_running_instances_alive() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    // 先发一次心跳让节点上线
    heartbeat_node_with_managed_instances(app.clone(), node_id, token, json!([])).await;
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file_id = files["files"][0]["id"].as_str().unwrap();
    let (_, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local",
            "name": "recovery keep alive",
            "node_id": node_id,
            "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id
        })),
    )
    .await;
    let instance_id = instance["id"].as_str().unwrap();

    sqlx::query("UPDATE model_instances SET status = 'running', process_id = 99991 WHERE id = ?")
        .bind(instance_id)
        .execute(&pool)
        .await
        .unwrap();

    // 模拟 Agent 重启后 heartbeat 报告该实例存活
    heartbeat_node_with_managed_instances(
        app.clone(),
        node_id,
        token,
        json!([{
            "instance_id": instance_id,
            "status": "running",
            "message": "Agent 重启后已恢复受管进程状态",
            "process_id": 99991,
            "process_ref": instance_id,
            "base_url": "http://127.0.0.1:18100"
        }]),
    )
    .await;

    let (status, fetched) = request(
        app.clone(),
        "GET",
        &format!("/api/model-instances/{instance_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["status"], "running");
    assert_eq!(fetched["process_id"], 99991);
    // running 实例不应有 last_error（恢复信息写入 Agent 日志，不入库）
    assert!(
        fetched["last_error"].as_str().unwrap_or("").is_empty()
            || fetched["last_error"] == Value::Null
    );
}

// ── 数据库唯一约束：name 和 hostname 独立唯一 ──

#[tokio::test]
async fn nodes_name_unique_constraint_rejects_duplicate_name() {
    let (_pool, app) = stage2_test_app().await;
    let (s1, _) = register_with_name_hostname(app.clone(), "agent-1", "host-a").await;
    assert_eq!(s1, StatusCode::OK);
    // 相同 name 不同 hostname → 被 UNIQUE(name) 约束拒绝
    let (s2, body) = register_with_name_hostname(app, "agent-1", "host-b").await;
    assert_eq!(s2, StatusCode::BAD_REQUEST);
    assert!(body["message"]
        .as_str()
        .unwrap_or("")
        .contains("相同名称不允许"));
}

#[tokio::test]
async fn nodes_hostname_unique_constraint_rejects_duplicate_hostname() {
    let (_pool, app) = stage2_test_app().await;
    let (s1, _) = register_with_name_hostname(app.clone(), "agent-1", "host-a").await;
    assert_eq!(s1, StatusCode::OK);
    // 相同 hostname 不同 name → 被 UNIQUE(hostname) 约束拒绝
    let (s2, body) = register_with_name_hostname(app, "agent-2", "host-a").await;
    assert_eq!(s2, StatusCode::BAD_REQUEST);
    assert!(body["message"]
        .as_str()
        .unwrap_or("")
        .contains("相同主机不允许"));
}

#[tokio::test]
async fn register_node_idempotent_same_name_and_hostname() {
    let (_pool, app) = stage2_test_app().await;
    let (s1, first) = register_with_name_hostname(app.clone(), "agent-1", "host-a").await;
    assert_eq!(s1, StatusCode::OK);
    // 重复注册 → 复用 node_id，更新 token
    let (s2, second) = register_with_name_hostname(app.clone(), "agent-1", "host-a").await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(second["node_id"], first["node_id"]);
    assert_ne!(second["agent_token"], first["agent_token"]);
    // 三次注册也幂等
    let (s3, third) = register_with_name_hostname(app, "agent-1", "host-a").await;
    assert_eq!(s3, StatusCode::OK);
    assert_eq!(third["node_id"], first["node_id"]);
}

#[tokio::test]
async fn register_node_recovers_from_unique_constraint_on_concurrent_same_node() {
    let (pool, app) = stage2_test_app().await;
    // 模拟并发：直接插入一行占用 name+hostname，然后 register_node 应复用该 node_id
    let preexisting_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO nodes (id, name, hostname, token_hash, token_prefix, registered_at, updated_at) VALUES (?, 'agent-c', 'host-c', 'old_hash', 'old_pre', 1, 1)",
    )
    .bind(&preexisting_id)
    .execute(&pool)
    .await
    .unwrap();

    let (status, response) = register_with_name_hostname(app, "agent-c", "host-c").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["node_id"], preexisting_id);

    // DB 中仍只有一条记录
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nodes WHERE name = 'agent-c' AND hostname = 'host-c'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

// ── 磁盘 SQLite 持久化：重启后状态恢复与 reconcile ──

#[tokio::test]
async fn disk_sqlite_persistence_survives_restart_and_reconciles() {
    let dir = std::env::temp_dir().join(format!("lightai_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");

    // 第一段：创建 node 和 instance，写入 DB
    let (node_id, token, instance_id) = {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        db::migrate(&pool).await.unwrap();
        let app = routes::app(pool.clone());
        let registered = register_node_json(app.clone()).await;
        let nid = registered["node_id"].as_str().unwrap().to_string();
        let tok = registered["agent_token"].as_str().unwrap().to_string();
        heartbeat_node_with_managed_instances(app.clone(), &nid, &tok, json!([])).await;
        let env = create_runtime_environment(app.clone(), &nid, &tok).await;
        let model = create_model_for_node(app.clone(), &nid, &tok).await;
        let mid = model["id"].as_str().unwrap();
        let (_, files) = request(
            app.clone(),
            "GET",
            &format!("/api/models/{mid}/files"),
            None,
        )
        .await;
        let mfid = files["files"][0]["id"].as_str().unwrap();
        let (_, inst) = request(
            app.clone(),
            "POST",
            "/api/model-instances",
            Some(json!({
                "deploy_type": "local",
                "name": "disk persist test",
                "node_id": &nid,
                "runtime_environment_id": env["id"],
                "model_file_id": mfid
            })),
        )
        .await;
        let iid = inst["id"].as_str().unwrap().to_string();
        sqlx::query(
            "UPDATE model_instances SET status = 'running', process_id = 77777 WHERE id = ?",
        )
        .bind(&iid)
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        (nid, tok, iid)
    };

    // 第二段：重新打开 DB（模拟 Server 重启），需显式处理 create_if_missing
    let pool = {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap()
    };
    db::migrate(&pool).await.unwrap();
    let app = routes::app(pool.clone());

    // 确认 node 和 instance 仍存在
    let (status, fetched) = request(
        app.clone(),
        "GET",
        &format!("/api/model-instances/{instance_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["status"], "running");
    assert_eq!(fetched["process_id"], 77777);

    // 模拟 Agent 心跳 reconcile：上报同一实例仍存活
    heartbeat_node_with_managed_instances(
        app.clone(),
        &node_id,
        &token,
        json!([{
            "instance_id": instance_id,
            "status": "running",
            "message": "still running after restart",
            "process_id": 77777,
            "process_ref": instance_id,
            "base_url": "http://127.0.0.1:18100"
        }]),
    )
    .await;

    let (status2, fetched2) = request(
        app,
        "GET",
        &format!("/api/model-instances/{instance_id}"),
        None,
    )
    .await;
    assert_eq!(status2, StatusCode::OK);
    assert_eq!(fetched2["status"], "running");
    assert!(fetched2["last_error"].as_str().is_none_or(str::is_empty));

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 离线 Agent 上 running 实例保持 running，但 node_online=false ──

#[tokio::test]
async fn running_instance_on_offline_node_shows_node_online_false() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    heartbeat_node_with_managed_instances(app.clone(), node_id, token, json!([])).await;
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file_id = files["files"][0]["id"].as_str().unwrap();
    let (_, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local", "name": "offline-node-instance",
            "node_id": node_id, "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id
        })),
    )
    .await;
    let instance_id = instance["id"].as_str().unwrap();
    sqlx::query("UPDATE model_instances SET status = 'running' WHERE id = ?")
        .bind(instance_id)
        .execute(&pool)
        .await
        .unwrap();
    // 设置心跳时间为过期，模拟 Agent 离线
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 120;
    sqlx::query("UPDATE nodes SET last_heartbeat_at = ? WHERE id = ?")
        .bind(cutoff)
        .bind(node_id)
        .execute(&pool)
        .await
        .unwrap();

    let (status, fetched) = request(
        app,
        "GET",
        &format!("/api/model-instances/{instance_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["status"], "running");
    assert_eq!(fetched["node_online"], false);
    assert!(fetched["last_error"].as_str().is_none_or(|s| s.is_empty()));
    assert!(fetched["last_heartbeat_at"].is_number());
}

#[tokio::test]
async fn instance_list_includes_node_online_when_agent_offline() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    heartbeat_node_with_managed_instances(app.clone(), node_id, token, json!([])).await;
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file_id = files["files"][0]["id"].as_str().unwrap();
    let (_, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local", "name": "list-offline-test",
            "node_id": node_id, "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id
        })),
    )
    .await;
    let instance_id = instance["id"].as_str().unwrap();
    sqlx::query("UPDATE model_instances SET status = 'running' WHERE id = ?")
        .bind(instance_id)
        .execute(&pool)
        .await
        .unwrap();
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 120;
    sqlx::query("UPDATE nodes SET last_heartbeat_at = ? WHERE id = ?")
        .bind(cutoff)
        .bind(node_id)
        .execute(&pool)
        .await
        .unwrap();

    let (status, list) = request(app, "GET", "/api/model-instances", None).await;
    assert_eq!(status, StatusCode::OK);
    let instances = list["model_instances"].as_array().unwrap();
    let instance = instances.iter().find(|i| i["id"] == instance_id).unwrap();
    assert_eq!(instance["status"], "running");
    assert_eq!(instance["node_online"], false);
    assert!(instance["last_error"].as_str().is_none_or(|s| s.is_empty()));
}

// ── Agent 在线时 node_online=true ──

#[tokio::test]
async fn running_instance_on_online_node_shows_node_online_true() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    heartbeat_node_with_managed_instances(app.clone(), node_id, token, json!([])).await;
    let environment = create_runtime_environment(app.clone(), node_id, token).await;
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file_id = files["files"][0]["id"].as_str().unwrap();
    let (_, instance) = request(
        app.clone(),
        "POST",
        "/api/model-instances",
        Some(json!({
            "deploy_type": "local", "name": "online-node-instance",
            "node_id": node_id, "runtime_environment_id": environment["id"],
            "model_file_id": model_file_id
        })),
    )
    .await;
    let instance_id = instance["id"].as_str().unwrap();
    sqlx::query("UPDATE model_instances SET status = 'running' WHERE id = ?")
        .bind(instance_id)
        .execute(&pool)
        .await
        .unwrap();

    let (status, fetched) = request(
        app,
        "GET",
        &format!("/api/model-instances/{instance_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["status"], "running");
    assert_eq!(fetched["node_online"], true);
}

async fn stage2_test_app() -> (sqlx::SqlitePool, axum::Router) {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let app = routes::app(pool.clone());
    (pool, app)
}
