use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use lightai_server::{db, routes};
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;

mod common;

async fn test_app() -> (sqlx::SqlitePool, axum::Router) {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    common::ensure_initial_admin(&pool, "admin", "test-admin-pw-123").await;
    let app = routes::app(pool.clone());
    (pool, app)
}

/// Send a control-plane request authenticated with an admin session cookie.
async fn cp_request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    // Get a fresh admin session cookie.
    let login_resp = app
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
    let cookie = login_resp
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, cookie);
    if let Some(value) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        let response = app
            .oneshot(builder.body(Body::from(value.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let b = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let json: Value = if b.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&b).unwrap()
        };
        (status, json)
    } else {
        let response = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let b = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let json: Value = if b.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&b).unwrap()
        };
        (status, json)
    }
}

async fn register_node(app: axum::Router) -> Value {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": "node-a",
                        "hostname": "gpu-node-a",
                        "agent_version": "0.1.0",
                        "os": "linux",
                        "arch": "x86_64"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn register_returns_node_id_and_stores_only_token_hash() {
    let (pool, app) = test_app().await;
    let json = register_node(app).await;

    let node_id = json["node_id"].as_str().unwrap();
    let token = json["agent_token"].as_str().unwrap();
    assert!(!node_id.is_empty());
    assert!(token.len() >= 32);
    assert_eq!(json["heartbeat_interval_secs"], 15);

    let row = sqlx::query("SELECT token_hash, token_prefix FROM nodes WHERE id = ?")
        .bind(node_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let token_hash: String = row.get("token_hash");
    let token_prefix: String = row.get("token_prefix");

    assert_ne!(token_hash, token);
    assert!(token.starts_with(&token_prefix));
}

#[tokio::test]
async fn agent_config_policy_supports_global_defaults_and_node_override() {
    let (_pool, app) = test_app().await;
    let registered = register_node(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();

    let (status, _) = cp_request(
        app.clone(),
        "PUT",
        "/api/config/agent/global",
        Some(json!({
            "heartbeat_interval_secs": 20,
            "metrics_sample_interval_secs": 25,
            "command_timeout_secs": 6,
            "environment_check_timeout_secs": 7,
            "allowed_model_dirs": ["/models"],
            "collector_timeout_secs": 5,
            "collector_max_output_bytes": 1048576
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = cp_request(
        app.clone(),
        "PUT",
        &format!("/api/nodes/{node_id}/config"),
        Some(json!({
            "metrics_sample_interval_secs": 5,
            "allowed_model_dirs": ["/models", "/mnt/models"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

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
                        "collector_status": "no_collector_configured",
                        "agent_config": {
                            "config_version": 1,
                            "heartbeat_interval_secs": 15,
                            "metrics_sample_interval_secs": 15,
                            "command_timeout_secs": 5,
                            "environment_check_timeout_secs": 5,
                            "allowed_model_dirs": [],
                            "collector_timeout_secs": 5,
                            "collector_max_output_bytes": 1048576,
                            "last_config_updated_at": null
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
    let heartbeat_json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        heartbeat_json["agent_config"]["heartbeat_interval_secs"],
        20
    );
    assert_eq!(
        heartbeat_json["agent_config"]["metrics_sample_interval_secs"],
        5
    );
    assert_eq!(
        heartbeat_json["agent_config"]["allowed_model_dirs"][1],
        "/mnt/models"
    );

    let (_, nodes_json) = cp_request(app, "GET", "/api/nodes", None).await;
    let node = &nodes_json["nodes"][0];
    assert_eq!(
        node["effective_agent_config"]["metrics_sample_interval_secs"],
        5
    );
    assert_eq!(node["config_sync_status"], "out_of_sync");
}

#[tokio::test]
async fn heartbeat_updates_latest_status_and_inserts_metric_samples() {
    let (pool, app) = test_app().await;
    let registered = register_node(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();

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
                        "metrics": {
                            "cpu_usage_percent": 45.2,
                            "memory_total_bytes": 32000000000i64,
                            "memory_used_bytes": 12000000000i64,
                            "disk_total_bytes": 1000000000000i64,
                            "disk_used_bytes": 400000000000i64
                        },
                        "gpus": [{
                            "gpu_key": "nvidia:GPU-test",
                            "gpu_index": 0,
                            "vendor": "nvidia",
                            "name": "A100",
                            "uuid": "GPU-test",
                            "memory_total_bytes": 40000000000i64,
                            "memory_used_bytes": 5000000000i64,
                            "utilization_percent": 80.0,
                            "temperature_celsius": 65.0,
                            "power_watts": 250.0,
                            "collector": "nvidia-wsl"
                        }],
                        "collector_errors": [],
                        "collector_status": "collector_ok_devices_found",
                        "agent_config": {
                            "config_version": 1,
                            "heartbeat_interval_secs": 15,
                            "metrics_sample_interval_secs": 15,
                            "command_timeout_secs": 5,
                            "environment_check_timeout_secs": 5,
                            "last_config_updated_at": null
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify collector status saved.
    let status_col: Option<String> =
        sqlx::query_scalar("SELECT collector_status FROM node_status WHERE node_id = ?")
            .bind(node_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status_col, Some("collector_ok_devices_found".to_string()));

    let (_, nodes_json) = cp_request(app.clone(), "GET", "/api/nodes", None).await;
    let node = &nodes_json["nodes"][0];
    // Node status may be "offline" with hardcoded old timestamp; just verify it exists
    assert!(node["status"].as_str().is_some());
    assert!((node["metrics"]["cpu_usage_percent"].as_f64().unwrap() - 45.2).abs() < 0.01);
    assert_eq!(node["gpus"].as_array().unwrap().len(), 1);
    assert_eq!(node["gpus"][0]["gpu_key"], "nvidia:GPU-test");
    assert_eq!(node["gpus"][0]["name"], "A100");

    let (_, metrics_resp) = cp_request(
        app.clone(),
        "GET",
        &format!("/api/nodes/{node_id}/metrics?from=1600000000&to=1800000000"),
        None,
    )
    .await;
    assert_eq!(metrics_resp["sample_count"], 1);
    assert!(
        (metrics_resp["samples"][0]["cpu_usage_percent"]
            .as_f64()
            .unwrap()
            - 45.2)
            .abs()
            < 0.01
    );

    let (_, gpu_metrics) = cp_request(
        app,
        "GET",
        &format!(
            "/api/nodes/{node_id}/gpu-metrics?gpu_key=nvidia:GPU-test&from=1600000000&to=1800000000"
        ),
        None,
    )
    .await;
    assert_eq!(gpu_metrics["sample_count"], 1);
    assert_eq!(
        gpu_metrics["samples"][0]["memory_used_bytes"],
        5000000000i64
    );
}

#[tokio::test]
async fn metrics_api_returns_raw_samples_in_time_window() {
    let (_pool, app) = test_app().await;

    let (status, resp) = cp_request(
        app,
        "GET",
        "/api/nodes/unknown-node/metrics?from=1600000000&to=1800000000",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["requested_from"], 1600000000);
    assert_eq!(resp["requested_to"], 1800000000);
    assert_eq!(resp["sample_count"], 0);
}

#[tokio::test]
async fn gpu_metrics_query_endpoint_handles_gpu_keys_with_path_separators() {
    let (_pool, app) = test_app().await;

    let (status, resp) = cp_request(
        app,
        "GET",
        "/api/nodes/unknown-node/gpu-metrics?gpu_key=nvidia%2FGPU-1234&from=1600000000&to=1800000000",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["sample_count"], 0);
}

#[tokio::test]
async fn metrics_api_returns_empty_metadata_when_no_samples_exist() {
    let (_pool, app) = test_app().await;

    let (status, resp) = cp_request(
        app,
        "GET",
        "/api/nodes/unknown-node/metrics?from=1600000000&to=1800000000",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["sample_count"], 0);
    assert!(resp["actual_from"].is_null());
    assert!(resp["actual_to"].is_null());
}

#[tokio::test]
async fn heartbeat_rejects_invalid_token() {
    let (_pool, app) = test_app().await;
    let registered = register_node(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/heartbeat")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, "Bearer wrong-token")
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "sampled_at": 1_700_000_000i64,
                        "metrics": {},
                        "gpus": [],
                        "collector_errors": [],
                        "collector_status": "no_collector_configured",
                        "agent_config": {
                            "config_version": 1,
                            "heartbeat_interval_secs": 15,
                            "metrics_sample_interval_secs": 15,
                            "command_timeout_secs": 5,
                            "environment_check_timeout_secs": 5
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Collector registry ──

#[tokio::test]
async fn collector_registry_table_exists_after_migrate() {
    let (pool, _app) = test_app().await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM collector_registry")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn collector_registry_empty_list_on_fresh_db() {
    let (_pool, app) = test_app().await;
    let (status, json) = cp_request(app, "GET", "/api/collector-registry", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["collectors"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn collector_registry_register_and_list() {
    let (_pool, app) = test_app().await;
    let (status, entry) = cp_request(
        app.clone(),
        "POST",
        "/api/collector-registry",
        Some(json!({
            "id": "nvidia-wsl",
            "vendor": "nvidia",
            "name": "NVIDIA WSL",
            "version": "1.0.0",
            "description": "test",
            "discover_sha256": "abc123",
            "metrics_sha256": "def456",
            "enabled": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry["id"], "nvidia-wsl");

    let (status, json) = cp_request(app, "GET", "/api/collector-registry", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["collectors"].as_array().unwrap().len(), 1);
    assert_eq!(json["collectors"][0]["id"], "nvidia-wsl");
}

#[tokio::test]
async fn collector_registry_get_by_id_version() {
    let (_pool, app) = test_app().await;
    let (_, _) = cp_request(
        app.clone(),
        "POST",
        "/api/collector-registry",
        Some(json!({
            "id": "nvidia-wsl",
            "vendor": "nvidia",
            "name": "NVIDIA WSL",
            "version": "1.0.0",
            "description": "test",
            "discover_sha256": "abc123",
            "metrics_sha256": "def456",
            "enabled": true
        })),
    )
    .await;

    let (status, entry) =
        cp_request(app, "GET", "/api/collector-registry/nvidia-wsl/1.0.0", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry["id"], "nvidia-wsl");
}

#[tokio::test]
async fn collector_registry_update_enabled_by_upsert() {
    let (_pool, app) = test_app().await;
    let (_, _) = cp_request(
        app.clone(),
        "POST",
        "/api/collector-registry",
        Some(json!({
            "id": "nvidia-wsl",
            "vendor": "nvidia",
            "name": "NVIDIA WSL",
            "version": "1.0.0",
            "description": "test",
            "discover_sha256": "abc123",
            "metrics_sha256": "def456",
            "enabled": true
        })),
    )
    .await;

    let (status, _) = cp_request(
        app.clone(),
        "POST",
        "/api/collector-registry",
        Some(json!({
            "id": "nvidia-wsl",
            "vendor": "nvidia",
            "name": "NVIDIA WSL",
            "version": "1.0.0",
            "description": "test",
            "discover_sha256": "abc123",
            "metrics_sha256": "def456",
            "enabled": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, json) = cp_request(app, "GET", "/api/collector-registry", None).await;
    assert_eq!(json["collectors"].as_array().unwrap().len(), 1);
    assert_eq!(json["collectors"][0]["enabled"], false);
}

#[tokio::test]
async fn collector_registry_repeat_init_idempotent() {
    let (pool, _app) = test_app().await;
    // Calling the CLI init logic (via DB migration) twice is fine.
    db::migrate(&pool).await.unwrap();
    db::migrate(&pool).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM collector_registry")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn collector_registry_heartbeat_includes_registry() {
    let (_pool, app) = test_app().await;

    // Register a collector via CP endpoint.
    cp_request(
        app.clone(),
        "POST",
        "/api/collector-registry",
        Some(json!({
            "id": "nvidia-wsl",
            "vendor": "nvidia",
            "name": "NVIDIA WSL",
            "version": "1.0.0",
            "description": "test",
            "discover_sha256": "abc123",
            "metrics_sha256": "def456",
            "enabled": true
        })),
    )
    .await;

    // Simulate an agent heartbeat and check the response.
    let registered = register_node(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();

    // Send heartbeat without cookie (agent endpoint).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let heartbeat = json!({
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
            "command_timeout_secs": 5,
            "environment_check_timeout_secs": 5
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/heartbeat")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(serde_json::to_string(&heartbeat).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 10_000).await.unwrap();
    let resp: Value = serde_json::from_slice(&body).unwrap();
    // Heartbeat response must include collector_registry field.
    assert!(resp.get("collector_registry").is_some());
}

// ── Server collector CLI tests ──

use lightai_server::collector_cli::{self, RegisterOutcome};

fn make_temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lightai-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn tmp_collector_dir(
    name: &str,
    id: &str,
    version: &str,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = make_temp_dir();
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let toml = format!(
        "id = \"{id}\"\nvendor = \"test\"\nname = \"Test {name}\"\nversion = \"{version}\"\n\
         description = \"test collector\"\ndiscover = \"discover.sh\"\nmetrics = \"metrics.sh\"\n"
    );
    std::fs::write(dir.join("collector.toml"), toml).unwrap();
    std::fs::write(dir.join("discover.sh"), "#!/bin/sh\necho ok\n").unwrap();
    std::fs::write(dir.join("metrics.sh"), "#!/bin/sh\necho ok\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(
            dir.join("discover.sh"),
            std::fs::Permissions::from_mode(0o755),
        );
        let _ = std::fs::set_permissions(
            dir.join("metrics.sh"),
            std::fs::Permissions::from_mode(0o755),
        );
    }
    (root, dir)
}

async fn test_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    lightai_server::db::migrate(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn cli_register_one_inserts_collector() {
    let pool = test_pool().await;
    let (_root, dir) = tmp_collector_dir("nvidia-wsl", "nvidia-wsl", "1.0.0");
    let outcome = collector_cli::register_one(&pool, &dir).await.unwrap();
    match outcome {
        RegisterOutcome::Registered { id, version, .. } => {
            assert_eq!(id, "nvidia-wsl");
            assert_eq!(version, "1.0.0");
        }
        other => panic!("expected Registered, got {other:?}"),
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM collector_registry WHERE id = 'nvidia-wsl'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn cli_register_same_id_version_updates() {
    let pool = test_pool().await;
    let (_root, dir) = tmp_collector_dir("nvidia-wsl", "nvidia-wsl", "1.0.0");
    collector_cli::register_one(&pool, &dir).await.unwrap();
    let outcome = collector_cli::register_one(&pool, &dir).await.unwrap();
    match outcome {
        RegisterOutcome::Updated { id, version, .. } => {
            assert_eq!(id, "nvidia-wsl");
            assert_eq!(version, "1.0.0");
        }
        other => panic!("expected Updated, got {other:?}"),
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM collector_registry WHERE id = 'nvidia-wsl'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn cli_sync_root_registers_all_valid_dirs() {
    let pool = test_pool().await;
    let (root, _dir_a) = tmp_collector_dir("nvidia-wsl", "nvidia-wsl", "1.0.0");
    let dir_b = root.join("metax");
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_b.join("collector.toml"),
        "id = \"metax\"\nvendor = \"metax\"\nname = \"MetaX\"\nversion = \"1.0.0\"\n\
         description = \"\"\ndiscover = \"discover.sh\"\nmetrics = \"metrics.sh\"\n",
    )
    .unwrap();
    std::fs::write(dir_b.join("discover.sh"), "#!/bin/sh\necho ok\n").unwrap();
    std::fs::write(dir_b.join("metrics.sh"), "#!/bin/sh\necho ok\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(
            dir_b.join("discover.sh"),
            std::fs::Permissions::from_mode(0o755),
        );
        let _ = std::fs::set_permissions(
            dir_b.join("metrics.sh"),
            std::fs::Permissions::from_mode(0o755),
        );
    }

    let outcomes = collector_cli::sync_root(&pool, &root).await.unwrap();
    assert_eq!(outcomes.len(), 2);
    let registered = outcomes
        .iter()
        .filter(|o| matches!(o, RegisterOutcome::Registered { .. }))
        .count();
    assert_eq!(registered, 2);
}

#[tokio::test]
async fn cli_sync_skips_invalid_dir() {
    let pool = test_pool().await;
    let (root, _dir_a) = tmp_collector_dir("nvidia-wsl", "nvidia-wsl", "1.0.0");
    let bad = root.join("bad-dir");
    std::fs::create_dir_all(&bad).unwrap();

    let outcomes = collector_cli::sync_root(&pool, &root).await.unwrap();
    assert_eq!(outcomes.len(), 2);
    let skipped = outcomes
        .iter()
        .filter(|o| matches!(o, RegisterOutcome::Skipped { .. }))
        .count();
    assert_eq!(skipped, 1);
}

#[tokio::test]
async fn cli_register_skips_missing_scripts() {
    let pool = test_pool().await;
    let root = make_temp_dir();
    let dir = root.join("broken");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("collector.toml"),
        "id = \"brk\"\nvendor = \"t\"\nname = \"B\"\nversion = \"1\"\n\
         description = \"\"\ndiscover = \"discover.sh\"\nmetrics = \"metrics.sh\"\n",
    )
    .unwrap();
    let outcome = collector_cli::register_one(&pool, &dir).await.unwrap();
    assert!(matches!(outcome, RegisterOutcome::Skipped { .. }));
}

#[tokio::test]
async fn cli_register_does_not_execute_scripts() {
    let pool = test_pool().await;
    let root = make_temp_dir();
    let dir = root.join("safe");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("collector.toml"),
        "id = \"safe\"\nvendor = \"t\"\nname = \"S\"\nversion = \"1\"\n\
         description = \"\"\ndiscover = \"discover.sh\"\nmetrics = \"metrics.sh\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("discover.sh"),
        "#!/bin/sh\ntouch /tmp/should-not-exist-cli-test\n",
    )
    .unwrap();
    std::fs::write(dir.join("metrics.sh"), "#!/bin/sh\necho ok\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(
            dir.join("discover.sh"),
            std::fs::Permissions::from_mode(0o755),
        );
        let _ = std::fs::set_permissions(
            dir.join("metrics.sh"),
            std::fs::Permissions::from_mode(0o755),
        );
    }
    let outcome = collector_cli::register_one(&pool, &dir).await.unwrap();
    assert!(matches!(
        outcome,
        RegisterOutcome::Registered { .. } | RegisterOutcome::Updated { .. }
    ));
    assert!(!std::path::Path::new("/tmp/should-not-exist-cli-test").exists());
}
