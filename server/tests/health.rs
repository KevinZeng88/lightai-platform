use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use lightai_server::db;
use lightai_server::routes;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn health_returns_ok_for_server() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let response = routes::app_with_emergency_token(pool, "test-emergency-token".to_string())
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "server");
}
