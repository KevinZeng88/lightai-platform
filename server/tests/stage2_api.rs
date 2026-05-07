use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use lightai_server::{db, routes};
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;

async fn test_app() -> (sqlx::SqlitePool, axum::Router) {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let app = routes::app(pool.clone());
    (pool, app)
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

    let global_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config/agent/global")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "heartbeat_interval_secs": 20,
                        "metrics_sample_interval_secs": 25,
                        "command_timeout_secs": 6,
                        "environment_check_timeout_secs": 7,
                        "allowed_model_dirs": ["/models"],
                        "nvidia_collector_enabled": true,
                        "collector_timeout_secs": 5,
                        "collector_max_output_bytes": 1048576
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(global_response.status(), StatusCode::OK);

    let node_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/nodes/{node_id}/config"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "metrics_sample_interval_secs": 5,
                        "allowed_model_dirs": ["/models", "/mnt/models"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(node_response.status(), StatusCode::OK);

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
                            "metrics_sample_interval_secs": 15,
                            "command_timeout_secs": 5,
                            "environment_check_timeout_secs": 5,
                            "allowed_model_dirs": [],
                            "nvidia_collector_enabled": true,
                            "custom_collector_script": null,
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

    let nodes_response = app
        .oneshot(
            Request::builder()
                .uri("/api/nodes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(nodes_response.into_body(), 4096).await.unwrap();
    let nodes_json: Value = serde_json::from_slice(&body).unwrap();
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
                            "cpu_usage_percent": 41.5,
                            "memory_total_bytes": 16000,
                            "memory_used_bytes": 8000,
                            "disk_total_bytes": 100000,
                            "disk_used_bytes": 25000
                        },
                        "gpus": [{
                            "gpu_key": "nvidia:GPU-abc",
                            "gpu_index": 0,
                            "vendor": "nvidia",
                            "name": "NVIDIA A10",
                            "uuid": "GPU-abc",
                            "driver_version": "550.1",
                            "memory_total_bytes": 24000,
                            "memory_used_bytes": 12000,
                            "utilization_percent": 88.0,
                            "temperature_celsius": 62.0,
                            "power_watts": 110.0,
                            "collector": "nvidia",
                            "raw_json": null
                        }],
                        "collector_errors": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let status_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_status")
        .fetch_one(&pool)
        .await
        .unwrap();
    let node_sample_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_metric_samples")
        .fetch_one(&pool)
        .await
        .unwrap();
    let gpu_status_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gpu_status")
        .fetch_one(&pool)
        .await
        .unwrap();
    let gpu_sample_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gpu_metric_samples")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(status_count, 1);
    assert_eq!(node_sample_count, 1);
    assert_eq!(gpu_status_count, 1);
    assert_eq!(gpu_sample_count, 1);
}

#[tokio::test]
async fn metrics_api_returns_raw_samples_in_time_window() {
    let (_pool, app) = test_app().await;
    let registered = register_node(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();

    for sampled_at in [100i64, 200i64] {
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
                            "sampled_at": sampled_at,
                            "metrics": {
                                "cpu_usage_percent": sampled_at as f64 / 10.0,
                                "memory_total_bytes": 100,
                                "memory_used_bytes": sampled_at,
                                "disk_total_bytes": 1000,
                                "disk_used_bytes": sampled_at
                            },
                            "gpus": [{
                                "gpu_key": "custom:0",
                                "gpu_index": 0,
                                "vendor": "custom",
                                "name": "Custom GPU",
                                "uuid": null,
                                "driver_version": null,
                                "memory_total_bytes": 1000,
                                "memory_used_bytes": sampled_at,
                                "utilization_percent": sampled_at as f64 / 2.0,
                                "temperature_celsius": null,
                                "power_watts": null,
                                "collector": "custom",
                                "raw_json": null
                            }],
                            "collector_errors": []
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/nodes/{node_id}/metrics?from=150&to=250"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["requested_from"], 150);
    assert_eq!(json["requested_to"], 250);
    assert_eq!(json["actual_from"], 200);
    assert_eq!(json["actual_to"], 200);
    assert_eq!(json["sample_count"], 1);
    assert_eq!(json["samples"].as_array().unwrap().len(), 1);
    assert_eq!(json["samples"][0]["sampled_at"], 200);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/nodes/{node_id}/gpus/{}/metrics?from=0&to=250",
                    "custom:0"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["requested_from"], 0);
    assert_eq!(json["requested_to"], 250);
    assert_eq!(json["actual_from"], 100);
    assert_eq!(json["actual_to"], 200);
    assert_eq!(json["sample_count"], 2);
    assert_eq!(json["samples"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn gpu_metrics_query_endpoint_handles_gpu_keys_with_path_separators() {
    let (_pool, app) = test_app().await;
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
                        "sampled_at": 300i64,
                        "metrics": {},
                        "gpus": [{
                            "gpu_key": "custom:slot/0",
                            "gpu_index": 0,
                            "vendor": "custom",
                            "name": "Custom GPU",
                            "uuid": null,
                            "driver_version": null,
                            "memory_total_bytes": 1000,
                            "memory_used_bytes": 500,
                            "utilization_percent": 72.0,
                            "temperature_celsius": null,
                            "power_watts": null,
                            "collector": "custom",
                            "raw_json": null
                        }],
                        "collector_errors": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/nodes/{node_id}/gpu-metrics?gpu_key=custom%3Aslot%2F0&from=0&to=400"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["gpu_key"], "custom:slot/0");
    assert_eq!(json["sample_count"], 1);
    assert_eq!(json["samples"][0]["utilization_percent"], 72.0);
}

#[tokio::test]
async fn metrics_api_returns_empty_metadata_when_no_samples_exist() {
    let (_pool, app) = test_app().await;
    let registered = register_node(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/nodes/{node_id}/metrics?from=0&to=250"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["requested_from"], 0);
    assert_eq!(json["requested_to"], 250);
    assert_eq!(json["actual_from"], Value::Null);
    assert_eq!(json["actual_to"], Value::Null);
    assert_eq!(json["sample_count"], 0);
    assert_eq!(json["samples"].as_array().unwrap().len(), 0);
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
                        "sampled_at": 100,
                        "metrics": {},
                        "gpus": [],
                        "collector_errors": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Collector registry tests ──

#[tokio::test]
async fn collector_registry_table_exists_after_migrate() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();

    // Verify the table exists by querying it.
    let rows: Vec<(String,)> = sqlx::query_as("SELECT id FROM collector_registry")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn collector_registry_empty_list_on_fresh_db() {
    let (_pool, app) = test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/collector-registry")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 10_000).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let collectors = payload["collectors"].as_array().unwrap();
    assert!(collectors.is_empty());
}

#[tokio::test]
async fn collector_registry_register_and_list() {
    let (_pool, app) = test_app().await;

    let inspect_json = json!({
        "id": "nvidia-r535",
        "vendor": "nvidia",
        "name": "NVIDIA R535 Collector",
        "version": "1.0.0",
        "description": "test",
        "discover_sha256": "abc123",
        "metrics_sha256": "def456"
    });

    // Register.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/collector-registry")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_string(&inspect_json).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 10_000).await.unwrap();
    let entry: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(entry["id"], "nvidia-r535");
    assert_eq!(entry["version"], "1.0.0");
    assert_eq!(entry["enabled"], true);

    // List.
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/collector-registry")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 10_000).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let collectors = payload["collectors"].as_array().unwrap();
    assert_eq!(collectors.len(), 1);
    assert_eq!(collectors[0]["id"], "nvidia-r535");
}

#[tokio::test]
async fn collector_registry_get_by_id_version() {
    let (_pool, app) = test_app().await;

    let inspect_json = json!({
        "id": "nvidia-r550",
        "vendor": "nvidia",
        "name": "NVIDIA R550 Collector",
        "version": "2.0.0",
        "description": "test",
        "discover_sha256": "abc123",
        "metrics_sha256": "def456"
    });
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/collector-registry")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_string(&inspect_json).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/collector-registry/nvidia-r550/2.0.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn collector_registry_update_enabled_by_upsert() {
    let (_pool, app) = test_app().await;

    let inspect_json = json!({
        "id": "nvidia-r535",
        "vendor": "nvidia",
        "name": "NVIDIA R535 Collector",
        "version": "1.0.0",
        "description": "test",
        "discover_sha256": "abc123",
        "metrics_sha256": "def456",
        "enabled": false
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/collector-registry")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_string(&inspect_json).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 10_000).await.unwrap();
    let entry: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(entry["enabled"], false);
}

#[tokio::test]
async fn collector_registry_repeat_init_idempotent() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    // Call migrate twice — second call should be idempotent.
    db::migrate(&pool).await.unwrap();
    db::migrate(&pool).await.unwrap();

    let rows: Vec<(String,)> = sqlx::query_as("SELECT id FROM collector_registry")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn collector_registry_heartbeat_includes_registry() {
    let (_pool, app) = test_app().await;
    let node = register_node(app.clone()).await;

    let heartbeat = json!({
        "node_id": node["node_id"],
        "sampled_at": 1700000000i64,
        "metrics": {
            "cpu_usage_percent": 10.0,
            "memory_total_bytes": 8000000000i64,
            "memory_used_bytes": 4000000000i64,
            "disk_total_bytes": 100000000000i64,
            "disk_used_bytes": 50000000000i64
        },
        "gpus": [],
        "collector_errors": [],
        "agent_config": {
            "config_version": 1,
            "heartbeat_interval_secs": 15,
            "metrics_sample_interval_secs": 15,
            "command_timeout_secs": 5,
            "environment_check_timeout_secs": 5
        },
        "managed_instances": []
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/heartbeat")
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", node["agent_token"].as_str().unwrap()),
                )
                .body(Body::from(serde_json::to_string(&heartbeat).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 10_000).await.unwrap();
    let resp: Value = serde_json::from_slice(&body).unwrap();
    // Heartbeat response must include collector_registry field (empty or populated).
    assert!(resp.get("collector_registry").is_some());
}
