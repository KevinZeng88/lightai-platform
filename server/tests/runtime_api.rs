#![allow(unused_imports)]
#![allow(dead_code)]
use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use lightai_server::{db, routes};
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;

mod common;
use common::*;

#[tokio::test]
async fn runtime_environments_can_be_listed_by_node_and_globally() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let created = create_runtime_environment(app.clone(), node_id, token).await;

    assert_eq!(created["node_id"], node_id);
    assert_eq!(created["backend"], "llama_cpp");
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
async fn docker_or_script_runtime_environment_requires_online_agent() {
    let app = test_app().await;
    let node_id = register_node(app.clone()).await;

    let (status, json) = request(
        app,
        "POST",
        &format!("/api/nodes/{node_id}/runtime-environments"),
        Some(json!({
            "name": "Node Script",
            "backend": "custom",
            "deploy_type": "script",
            "binary_path": "/usr/local/bin/run-model",
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
async fn ollama_runtime_environment_saves_without_agent_check_or_binary_path() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();

    let (status, json) = request(
        app,
        "POST",
        &format!("/api/nodes/{node_id}/runtime-environments"),
        Some(json!({
            "name": "Node Ollama",
            "backend": "ollama",
            "deploy_type": "binary",
            "params_json": json!({
                "defaults": {
                    "host": "127.0.0.1",
                    "port": 11434
                }
            }).to_string(),
            "enabled": true
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["backend"], "ollama");
    assert_eq!(json["deploy_type"], "binary");
    assert_eq!(json["binary_path"], Value::Null);
    assert_eq!(json["check_status"], "available");
    assert!(json["check_message"]
        .as_str()
        .unwrap()
        .contains("Ollama runtime config saved"));
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
            "script not found or not accessible",
        )
        .await;
    };
    let ((status, json), _) = tokio::join!(create_request, agent);

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["message"], "script not found or not accessible");

    let (status, list) = request(app, "GET", "/api/runtime-environments", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["runtime_environments"].as_array().unwrap().len(), 0);
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
