use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use lightai_server::{db, routes};
use tower::ServiceExt;

#[tokio::test]
async fn health_returns_json() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let app = routes::app_with_web(
        pool,
        Default::default(),
        Default::default(),
        None, // web disabled
    );

    let response = app
        .oneshot(Request::get("/health").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "server");
}

#[tokio::test]
async fn api_routes_work_when_web_is_enabled() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();

    let tmp = std::env::temp_dir().join(format!("lightai-test-web-{}", std::process::id()));
    let web_dir = tmp.join("web").join("dist");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html>test</html>").unwrap();

    let app = routes::app_with_web(
        pool,
        Default::default(),
        Default::default(),
        Some(web_dir.to_string_lossy().to_string()),
    );

    // /api/setup/status should return JSON even with web enabled.
    let response = app
        .clone()
        .oneshot(Request::get("/api/setup/status").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["setup_required"], true);

    // /health returns JSON.
    let response = app
        .clone()
        .oneshot(Request::get("/health").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // / returns index.html.
    let response = app
        .clone()
        .oneshot(Request::get("/").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(std::str::from_utf8(&body).unwrap().contains("<html>test</html>"));

    // Unknown /api path returns JSON 404, not index.html.
    let response = app
        .clone()
        .oneshot(Request::get("/api/nonexistent-endpoint").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("not_found"), "API 404 should return JSON, got: {text}");

    // SPA deep route falls back to index.html.
    let response = app
        .clone()
        .oneshot(Request::get("/nodes").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "SPA deep route should return index.html");
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(std::str::from_utf8(&body).unwrap().contains("<html>test</html>"));

    // Clean up.
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn web_disabled_returns_404_on_root() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let app = routes::app_with_web(
        pool,
        Default::default(),
        Default::default(),
        None,
    );

    let response = app
        .oneshot(Request::get("/").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    // Without web, root returns 404 from axum's default fallback.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
