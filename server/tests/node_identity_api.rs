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
        let app = routes::app_with_emergency_token(pool.clone(), TEST_EMERGENCY_TOKEN.to_string());
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
    let app = routes::app_with_emergency_token(pool.clone(), TEST_EMERGENCY_TOKEN.to_string());

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
