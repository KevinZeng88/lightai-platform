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
