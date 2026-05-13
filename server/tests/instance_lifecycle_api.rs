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
            "instance started",
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
        report_instance_task_result(app, node_id, token, task_id, "stopped", "instance stopped")
            .await;
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
                "message": "process exited: main: exiting due to HTTP server error",
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
        "directory verified",
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
        let params_json: serde_json::Value = serde_json::from_str(
            task["task"]["payload"]["params_json"]
                .as_str()
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        assert_eq!(params_json["port"], 18081);
        assert_eq!(params_json["extra_args"][0], "--verbose");
        let task_id = task["task"]["id"].as_str().unwrap();
        report_instance_task_result_with_details(
            app.clone(),
            node_id,
            token,
            task_id,
            json!({
                "instance_status": "running",
                "message": "instance started",
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
                "message": "test succeeded: HTTP 200 OK",
                "response_summary": "{\"data\":[]}"
            }),
        )
        .await;
    };
    let ((status, tested), _) = tokio::join!(test_request, agent);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tested["status"], "running");
    assert!(tested["last_error"]
        .as_str()
        .unwrap_or("")
        .contains("test succeeded: HTTP 200 OK"));

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
