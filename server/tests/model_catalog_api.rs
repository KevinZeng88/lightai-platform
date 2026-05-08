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
            && event["actor_type"] == "system-emergency"
            && event["actor_id"] == "emergency"
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
