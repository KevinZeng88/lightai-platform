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
    let (app, pool) = test_app_with_pool().await;
    let first = register_node_json(app.clone()).await;
    let first_id = first["node_id"].as_str().unwrap();
    let first_token = first["agent_token"].as_str().unwrap();

    let second = register_node_json(app.clone()).await;
    assert_eq!(second["node_id"].as_str().unwrap(), first_id);
    assert_ne!(second["agent_token"].as_str().unwrap(), first_token);

    // Ensure DB has exactly one record
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
    let (app, _pool) = test_app_with_pool().await;
    // Register node-a @ gpu-node-a
    let (status1, _) = register_with_name_hostname(app.clone(), "node-a", "gpu-node-a").await;
    assert_eq!(status1, StatusCode::OK);
    // Same name different hostname → rejected
    let (status2, body) = register_with_name_hostname(app.clone(), "node-a", "gpu-node-b").await;
    assert_eq!(status2, StatusCode::BAD_REQUEST);
    assert!(body["message"]
        .as_str()
        .unwrap_or("")
        .contains("same name cannot be used for different hosts"));
}

#[tokio::test]
async fn register_node_rejects_different_name_same_hostname() {
    let (app, _pool) = test_app_with_pool().await;
    // Register node-a @ gpu-node-a
    let (status1, _) = register_with_name_hostname(app.clone(), "node-a", "gpu-node-a").await;
    assert_eq!(status1, StatusCode::OK);
    // Different name same hostname → rejected
    let (status2, body) = register_with_name_hostname(app.clone(), "node-b", "gpu-node-a").await;
    assert_eq!(status2, StatusCode::BAD_REQUEST);
    assert!(body["message"]
        .as_str()
        .unwrap_or("")
        .contains("same host cannot be used for different names"));
}

#[tokio::test]
async fn register_node_creates_new_node_for_different_name() {
    let (app, _pool) = test_app_with_pool().await;
    let first = register_node_json(app.clone()).await;
    let second = register_second_node_json(app.clone()).await;
    assert_ne!(second["node_id"], first["node_id"]);
}

// ── check_model_instance returns last_error when Agent is offline ──

#[tokio::test]
async fn nodes_name_unique_constraint_rejects_duplicate_name() {
    let (app, _pool) = test_app_with_pool().await;
    let (s1, _) = register_with_name_hostname(app.clone(), "agent-1", "host-a").await;
    assert_eq!(s1, StatusCode::OK);
    // Same name different hostname → rejected by UNIQUE(name) constraint
    let (s2, body) = register_with_name_hostname(app, "agent-1", "host-b").await;
    assert_eq!(s2, StatusCode::BAD_REQUEST);
    assert!(body["message"]
        .as_str()
        .unwrap_or("")
        .contains("same name cannot"));
}

#[tokio::test]
async fn nodes_hostname_unique_constraint_rejects_duplicate_hostname() {
    let (app, _pool) = test_app_with_pool().await;
    let (s1, _) = register_with_name_hostname(app.clone(), "agent-1", "host-a").await;
    assert_eq!(s1, StatusCode::OK);
    // Same hostname different name → rejected by UNIQUE(hostname) constraint
    let (s2, body) = register_with_name_hostname(app, "agent-2", "host-a").await;
    assert_eq!(s2, StatusCode::BAD_REQUEST);
    assert!(body["message"]
        .as_str()
        .unwrap_or("")
        .contains("same host cannot"));
}

#[tokio::test]
async fn register_node_idempotent_same_name_and_hostname() {
    let (app, _pool) = test_app_with_pool().await;
    let (s1, first) = register_with_name_hostname(app.clone(), "agent-1", "host-a").await;
    assert_eq!(s1, StatusCode::OK);
    // Re-registration → reuse node_id, update token
    let (s2, second) = register_with_name_hostname(app.clone(), "agent-1", "host-a").await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(second["node_id"], first["node_id"]);
    assert_ne!(second["agent_token"], first["agent_token"]);
    // Idempotent across three registrations
    let (s3, third) = register_with_name_hostname(app, "agent-1", "host-a").await;
    assert_eq!(s3, StatusCode::OK);
    assert_eq!(third["node_id"], first["node_id"]);
}

#[tokio::test]
async fn register_node_recovers_from_unique_constraint_on_concurrent_same_node() {
    let (app, pool) = test_app_with_pool().await;
    // Simulate concurrency: insert row occupying name+hostname, then register_node should reuse that node_id
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

    // DB still has exactly one record
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nodes WHERE name = 'agent-c' AND hostname = 'host-c'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

// ── Disk SQLite persistence: state recovery and reconcile after restart ──

#[tokio::test]
async fn disk_sqlite_persistence_survives_restart_and_reconciles() {
    let dir = std::env::temp_dir().join(format!("lightai_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");

    // Phase 1: create node and instance, write to DB
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
        common::ensure_initial_admin(&pool, "admin", "test-admin-pw-123").await;
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

    // Phase 2: reopen DB (simulate Server restart), must handle create_if_missing
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
    common::ensure_initial_admin(&pool, "admin", "test-admin-pw-123").await;
    let app = routes::app(pool.clone());

    // Verify node and instance still exist
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

    // Simulate Agent heartbeat reconcile: report same instance still alive
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

// ── Running instances on offline Agent stay running, but node_online=false ──
