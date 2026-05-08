use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use lightai_server::{db, routes};
use serde_json::{json, Value};
use tower::ServiceExt;

#[tokio::test]
async fn control_plane_api_requires_user_session_or_emergency_token() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let app = routes::app_with_emergency_token(pool, "test-emergency-token".to_string());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/nodes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "unauthorized");
    assert_eq!(json["message"], "请先登录");
}

#[tokio::test]
async fn control_plane_api_accepts_emergency_token() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let app = routes::app_with_emergency_token(pool, "test-emergency-token".to_string());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/nodes")
                .header(header::AUTHORIZATION, "Bearer test-emergency-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn agent_registration_is_not_control_plane_protected() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let app = routes::app_with_emergency_token(pool, "test-emergency-token".to_string());

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
}
