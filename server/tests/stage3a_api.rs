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

async fn create_runtime_environment(app: axum::Router, node_id: &str) -> Value {
    let (status, json) = request(
        app,
        "POST",
        &format!("/api/nodes/{node_id}/runtime-environments"),
        Some(json!({
            "name": "External Ollama",
            "backend": "ollama",
            "deploy_type": "external",
            "version": "0.5.0",
            "base_url": "http://127.0.0.1:11434",
            "health_url": "http://127.0.0.1:11434/api/tags",
            "enabled": true
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    json
}

async fn create_model(app: axum::Router) -> Value {
    let (status, json) = request(
        app,
        "POST",
        "/api/models",
        Some(json!({
            "name": "qwen2-7b",
            "display_name": "Qwen2 7B",
            "model_type": "llm",
            "model_path": "/models/qwen2-7b",
            "description": "test model",
            "default_backend": "ollama"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    json
}

#[tokio::test]
async fn runtime_environments_can_be_listed_by_node_and_globally() {
    let app = test_app().await;
    let node_id = register_node(app.clone()).await;
    let created = create_runtime_environment(app.clone(), &node_id).await;

    assert_eq!(created["node_id"], node_id);
    assert_eq!(created["backend"], "ollama");
    assert_eq!(created["deploy_type"], "external");

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
async fn model_file_trash_is_independent_from_model_delete() {
    let app = test_app().await;
    let node_id = register_node(app.clone()).await;
    let model = create_model(app.clone()).await;

    let (status, trash) = request(
        app.clone(),
        "POST",
        &format!("/api/models/{}/file-trash", model["id"].as_str().unwrap()),
        Some(json!({
            "node_id": node_id,
            "path": "/models/qwen2-7b",
            "reason": "manual cleanup later",
            "note": "do not physically delete in Stage 3A"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trash["status"], "pending");

    let (status, trash_list) = request(app.clone(), "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trash_list["items"].as_array().unwrap().len(), 1);

    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["models"].as_array().unwrap().len(), 1);
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
async fn external_instance_requires_at_least_one_check_url() {
    let app = test_app().await;
    let model = create_model(app.clone()).await;
    let (status, instance) = request(
        app,
        "POST",
        "/api/model-instances",
        Some(json!({
            "model_id": model["id"],
            "name": "qwen2 external",
            "backend": "custom"
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
