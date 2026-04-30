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
