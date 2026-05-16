use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use lightai_server::{models, repository};
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;

mod common;
use common::*;

#[tokio::test]
async fn gateway_start_requires_online_agent() {
    let app = test_app().await;
    let node_id = register_node(app.clone()).await;

    let (status, json) = request(
        app,
        "POST",
        &format!("/api/nodes/{node_id}/gateway/start"),
        Some(gateway_start_payload()),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["error"], "conflict");
    assert!(json["message"].as_str().unwrap().contains("Gateway task"));
}

#[tokio::test]
async fn gateway_start_creates_agent_task_without_touching_instances() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;

    let gateway_start_uri = format!("/api/nodes/{node_id}/gateway/start");
    let request_gateway_start = request(
        app.clone(),
        "POST",
        &gateway_start_uri,
        Some(gateway_start_payload()),
    );
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        assert_eq!(task["task"]["kind"], "start_gateway");
        assert_eq!(
            task["task"]["payload"]["binary_path"],
            "/opt/lightai/bin/lightai-gateway"
        );
        assert!(task["task"]["payload"].get("state_path").is_none());
        report_gateway_task_result(
            app.clone(),
            node_id,
            token,
            task_id,
            json!({
                "gateway_status": "running",
                "message": "Gateway started",
                "process_id": 1234,
                "process_ref": "1234",
                "health_url": "http://127.0.0.1:18082/health",
                "log_tail": null,
                "command": "/opt/lightai/bin/lightai-gateway --config gateway.toml"
            }),
        )
        .await;
    };

    let ((status, json), _) = tokio::join!(request_gateway_start, agent);

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["gateway_status"], "running");
    assert_eq!(json["process_id"], 1234);
    let instance_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM model_instances")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("count");
    assert_eq!(instance_count, 0);
}

#[tokio::test]
async fn gateway_log_task_uses_gateway_task_kind() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;

    let gateway_logs_uri = format!("/api/nodes/{node_id}/gateway/logs");
    let request_gateway_logs = request(
        app.clone(),
        "POST",
        &gateway_logs_uri,
        Some(json!({
            "state_path": "/tmp/untrusted-gateway-state.json",
            "log_path": "/tmp/untrusted-lightai-gateway.log",
            "max_bytes": 4096
        })),
    );
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        assert_eq!(task["task"]["kind"], "read_gateway_log");
        assert!(task["task"]["payload"].get("state_path").is_none());
        assert!(task["task"]["payload"].get("log_path").is_none());
        report_gateway_task_result(
            app.clone(),
            node_id,
            token,
            task_id,
            json!({
                "gateway_status": "log_available",
                "message": "Gateway log read succeeded",
                "process_id": null,
                "process_ref": null,
                "health_url": null,
                "log_tail": "gateway log",
                "command": null
            }),
        )
        .await;
    };

    let ((status, json), _) = tokio::join!(request_gateway_logs, agent);

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["gateway_status"], "log_available");
    assert_eq!(json["log_tail"], "gateway log");
}

#[tokio::test]
async fn gateway_start_rejects_non_loopback_health_url() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;
    let mut payload = gateway_start_payload();
    payload["health_url"] = json!("http://example.com/health");

    let (status, json) = request(
        app,
        "POST",
        &format!("/api/nodes/{node_id}/gateway/start"),
        Some(payload),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["message"]
        .as_str()
        .unwrap()
        .contains("localhost or loopback"));
}

#[tokio::test]
async fn gateway_start_rejects_non_health_path() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;
    let mut payload = gateway_start_payload();
    payload["health_url"] = json!("http://127.0.0.1:18082/admin");

    let (status, json) = request(
        app,
        "POST",
        &format!("/api/nodes/{node_id}/gateway/start"),
        Some(payload),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["message"].as_str().unwrap().contains("/health"));
}

#[tokio::test]
async fn viewer_cannot_execute_gateway_lifecycle_action() {
    let (app, pool) = test_app_with_pool().await;
    repository::create_user(
        &pool,
        models::UserCreateRequest {
            username: "viewer".to_string(),
            password: "viewer-password-123".to_string(),
            role: "viewer".to_string(),
        },
    )
    .await
    .unwrap();
    let node_id = register_node(app.clone()).await;
    let cookie = login_cookie(&app, "viewer", "viewer-password-123").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/nodes/{node_id}/gateway/check"))
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_can_execute_gateway_lifecycle_action() {
    let app = test_app().await;
    let node_id = register_node(app.clone()).await;

    let (status, _json) = request(
        app,
        "POST",
        &format!("/api/nodes/{node_id}/gateway/check"),
        Some(json!({})),
    )
    .await;

    assert_ne!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn operator_can_execute_gateway_lifecycle_action() {
    let (app, pool) = test_app_with_pool().await;
    repository::create_user(
        &pool,
        models::UserCreateRequest {
            username: "operator".to_string(),
            password: "operator-password-123".to_string(),
            role: "operator".to_string(),
        },
    )
    .await
    .unwrap();
    let node_id = register_node(app.clone()).await;
    let cookie = login_cookie(&app, "operator", "operator-password-123").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/nodes/{node_id}/gateway/check"))
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(response.status(), StatusCode::FORBIDDEN);
}

fn gateway_start_payload() -> Value {
    json!({
        "binary_path": "/opt/lightai/bin/lightai-gateway",
        "config_path": "gateway.toml",
        "work_dir": "/opt/lightai",
        "log_path": "logs/lightai-gateway.log",
        "state_path": "data/agent-state.toml.gateway.json",
        "health_url": "http://127.0.0.1:18082/health"
    })
}

async fn report_gateway_task_result(
    app: axum::Router,
    node_id: &str,
    token: &str,
    task_id: &str,
    result: Value,
) {
    let status = match result["gateway_status"].as_str().unwrap_or("failed") {
        "running" | "stopped" | "log_available" => "succeeded",
        _ => "failed",
    };
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": status,
                        "result": result
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "gateway task result failed: {}",
        String::from_utf8_lossy(&body)
    );
}

async fn login_cookie(app: &axum::Router, username: &str, password: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "username": username,
                        "password": password
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}
