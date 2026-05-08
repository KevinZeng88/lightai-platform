use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use lightai_server::{db, repository, routes};
use serde_json::{json, Value};
use tower::ServiceExt;

mod common;

async fn test_app() -> (axum::Router, sqlx::SqlitePool) {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    common::ensure_initial_admin(&pool, "admin", "admin-password-123").await;
    (routes::app(pool.clone()), pool)
}

async fn empty_app() -> (axum::Router, sqlx::SqlitePool) {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    (routes::app(pool.clone()), pool)
}

async fn request(
    app: axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, header::HeaderMap, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };

    let response = app.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let json = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (status, headers, json)
}

async fn login_cookie(app: axum::Router, username: &str, password: &str) -> String {
    let (status, headers, _json) = request(
        app,
        "POST",
        "/api/auth/login",
        None,
        Some(json!({
            "username": username,
            "password": password
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    session_cookie(&headers)
}

fn session_cookie(headers: &header::HeaderMap) -> String {
    headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn control_plane_requires_logged_in_user() {
    let (app, _pool) = test_app().await;

    let (status, _headers, json) = request(app, "GET", "/api/nodes", None, None).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["error"], "unauthorized");
}

#[tokio::test]
async fn login_session_allows_control_plane_and_logout_revokes_it() {
    let (app, _pool) = test_app().await;

    let (status, headers, json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({
            "username": "admin",
            "password": "admin-password-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["user"]["username"], "admin");
    assert_eq!(json["user"]["role"], "admin");
    let cookie = session_cookie(&headers);

    let (status, _headers, _json) =
        request(app.clone(), "GET", "/api/nodes", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _headers, json) =
        request(app.clone(), "GET", "/api/auth/me", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["user"]["username"], "admin");

    let (status, _headers, _json) =
        request(app.clone(), "POST", "/api/auth/logout", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _headers, _json) = request(app, "GET", "/api/nodes", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn setup_creates_first_admin_once_and_then_closes() {
    let (app, _pool) = empty_app().await;

    let (status, _headers, json) =
        request(app.clone(), "GET", "/api/setup/status", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["setup_required"], true);

    let (status, headers, json) = request(
        app.clone(),
        "POST",
        "/api/setup/admin",
        None,
        Some(json!({
            "username": "admin",
            "password": "admin-password-123A"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["user"]["role"], "admin");
    let cookie = session_cookie(&headers);

    let (status, _headers, json) =
        request(app.clone(), "GET", "/api/setup/status", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["setup_required"], false);

    let (status, _headers, _json) = request(
        app.clone(),
        "POST",
        "/api/setup/admin",
        None,
        Some(json!({
            "username": "second-admin",
            "password": "admin-password-456A"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _headers, _json) = request(app, "GET", "/api/users", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn agent_registration_does_not_require_user_session() {
    let (app, _pool) = test_app().await;

    let (status, _headers, json) = request(
        app,
        "POST",
        "/api/agent/register",
        None,
        Some(json!({
            "name": "node-a",
            "hostname": "gpu-node-a",
            "agent_version": "0.1.0",
            "os": "linux",
            "arch": "x86_64"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["agent_token"].as_str().unwrap().len() >= 32);
}

#[tokio::test]
async fn admin_can_create_and_disable_local_users() {
    let (app, _pool) = test_app().await;
    let (status, headers, _json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({
            "username": "admin",
            "password": "admin-password-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = session_cookie(&headers);

    let (status, _headers, user) = request(
        app.clone(),
        "POST",
        "/api/users",
        Some(&cookie),
        Some(json!({
            "username": "operator",
            "password": "operator-password-123",
            "role": "viewer"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["user"]["id"].as_str().unwrap();

    let (status, _headers, users) =
        request(app.clone(), "GET", "/api/users", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(users["users"].as_array().unwrap().len(), 2);

    let (status, _headers, _json) = request(
        app.clone(),
        "PUT",
        &format!("/api/users/{user_id}"),
        Some(&cookie),
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _headers, _json) = request(
        app,
        "POST",
        "/api/auth/login",
        None,
        Some(json!({
            "username": "operator",
            "password": "operator-password-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn group_role_grants_effective_admin_permission_until_group_disabled() {
    let (app, _pool) = test_app().await;
    let (status, headers, _json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({
            "username": "admin",
            "password": "admin-password-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let admin_cookie = session_cookie(&headers);

    let (status, _headers, group) = request(
        app.clone(),
        "POST",
        "/api/groups",
        Some(&admin_cookie),
        Some(json!({
            "name": "platform-admins",
            "role": "admin"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let group_id = group["group"]["id"].as_str().unwrap();

    let (status, _headers, user) = request(
        app.clone(),
        "POST",
        "/api/users",
        Some(&admin_cookie),
        Some(json!({
            "username": "operator",
            "password": "operator-password-123",
            "role": "viewer"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["user"]["id"].as_str().unwrap();

    let (status, _headers, _json) = request(
        app.clone(),
        "PUT",
        &format!("/api/groups/{group_id}/members"),
        Some(&admin_cookie),
        Some(json!({ "user_ids": [user_id] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, headers, json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({
            "username": "operator",
            "password": "operator-password-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["user"]["role"], "viewer");
    assert_eq!(json["user"]["effective_role"], "admin");
    let operator_cookie = session_cookie(&headers);

    let (status, _headers, _json) = request(
        app.clone(),
        "GET",
        "/api/users",
        Some(&operator_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _headers, _json) = request(
        app.clone(),
        "PUT",
        &format!("/api/groups/{group_id}"),
        Some(&admin_cookie),
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, headers, json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({
            "username": "operator",
            "password": "operator-password-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["user"]["effective_role"], "viewer");
    let operator_cookie = session_cookie(&headers);

    let (status, _headers, _json) =
        request(app, "GET", "/api/users", Some(&operator_cookie), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn group_delete_requires_empty_membership() {
    let (app, _pool) = test_app().await;
    let (status, headers, _json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({
            "username": "admin",
            "password": "admin-password-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = session_cookie(&headers);

    let (status, _headers, group) = request(
        app.clone(),
        "POST",
        "/api/groups",
        Some(&cookie),
        Some(json!({ "name": "ops", "role": "operator" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let group_id = group["group"]["id"].as_str().unwrap();

    let (status, _headers, user) = request(
        app.clone(),
        "POST",
        "/api/users",
        Some(&cookie),
        Some(json!({
            "username": "operator",
            "password": "operator-password-123",
            "role": "viewer"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["user"]["id"].as_str().unwrap();

    let (status, _headers, _json) = request(
        app.clone(),
        "PUT",
        &format!("/api/groups/{group_id}/members"),
        Some(&cookie),
        Some(json!({ "user_ids": [user_id] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _headers, json) = request(
        app.clone(),
        "DELETE",
        &format!("/api/groups/{group_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["error"], "conflict");

    let (status, _headers, _json) = request(
        app.clone(),
        "PUT",
        &format!("/api/groups/{group_id}/members"),
        Some(&cookie),
        Some(json!({ "user_ids": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _headers, _json) = request(
        app,
        "DELETE",
        &format!("/api/groups/{group_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn operator_can_manage_models_but_viewer_cannot_mutate() {
    let (app, _pool) = test_app().await;
    let (status, headers, _json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({ "username": "admin", "password": "admin-password-123" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let admin_cookie = session_cookie(&headers);

    let (status, _headers, _json) = request(
        app.clone(),
        "POST",
        "/api/users",
        Some(&admin_cookie),
        Some(json!({
            "username": "operator",
            "password": "operator-password-123A",
            "role": "operator"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _headers, _json) = request(
        app.clone(),
        "POST",
        "/api/users",
        Some(&admin_cookie),
        Some(json!({
            "username": "viewer",
            "password": "viewer-password-123A",
            "role": "viewer"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, headers, _json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({ "username": "operator", "password": "operator-password-123A" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let operator_cookie = session_cookie(&headers);
    let (status, _headers, _json) = request(
        app.clone(),
        "POST",
        "/api/models",
        Some(&operator_cookie),
        Some(json!({
            "name": "qwen",
            "model_type": "llm",
            "initial_file": { "node_id": "missing-node", "path": "/models/qwen.gguf" }
        })),
    )
    .await;
    assert_ne!(status, StatusCode::FORBIDDEN);

    let (status, headers, _json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({ "username": "viewer", "password": "viewer-password-123A" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer_cookie = session_cookie(&headers);
    let (status, _headers, _json) = request(
        app,
        "POST",
        "/api/models",
        Some(&viewer_cookie),
        Some(json!({ "name": "blocked", "model_type": "llm" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn password_reset_revokes_existing_sessions() {
    let (app, pool) = test_app().await;
    let (status, headers, _json) = request(
        app.clone(),
        "POST",
        "/api/auth/login",
        None,
        Some(json!({ "username": "admin", "password": "admin-password-123" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = session_cookie(&headers);

    repository::reset_user_password(
        &pool,
        "admin",
        "new-admin-password-123A",
        repository::PasswordPolicy::default(),
    )
    .await
    .unwrap();

    let (status, _headers, _json) =
        request(app.clone(), "GET", "/api/nodes", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _headers, _json) = request(
        app,
        "POST",
        "/api/auth/login",
        None,
        Some(json!({ "username": "admin", "password": "new-admin-password-123A" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn forced_password_change_limits_control_plane_until_user_changes_password() {
    let (app, pool) = test_app().await;
    let admin_cookie = login_cookie(app.clone(), "admin", "admin-password-123").await;

    let (status, _headers, user) = request(
        app.clone(),
        "POST",
        "/api/users",
        Some(&admin_cookie),
        Some(json!({
            "username": "viewer",
            "password": "viewer-password-123A",
            "role": "viewer"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["user"]["id"].as_str().unwrap();

    let (status, _headers, _json) = request(
        app.clone(),
        "PUT",
        &format!("/api/users/{user_id}"),
        Some(&admin_cookie),
        Some(json!({ "password": "viewer-temp-password-123A" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let viewer_cookie = login_cookie(app.clone(), "viewer", "viewer-temp-password-123A").await;
    let (status, _headers, json) = request(
        app.clone(),
        "GET",
        "/api/auth/me",
        Some(&viewer_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["user"]["must_change_password"], true);

    let (status, _headers, _json) =
        request(app.clone(), "GET", "/api/nodes", Some(&viewer_cookie), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _headers, _json) = request(
        app.clone(),
        "POST",
        "/api/auth/change-password",
        Some(&viewer_cookie),
        Some(json!({
            "current_password": "viewer-temp-password-123A",
            "new_password": "viewer-final-password-123A"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let changed = repository::list_users(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|user| user.username == "viewer")
        .unwrap();
    assert!(!changed.must_change_password);

    let (status, _headers, _json) =
        request(app.clone(), "GET", "/api/nodes", Some(&viewer_cookie), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let final_cookie = login_cookie(app.clone(), "viewer", "viewer-final-password-123A").await;
    let (status, _headers, _json) =
        request(app, "GET", "/api/nodes", Some(&final_cookie), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn role_permission_matrix_is_enforced_by_effective_role() {
    let (app, _pool) = test_app().await;
    let admin_cookie = login_cookie(app.clone(), "admin", "admin-password-123").await;

    for (username, role) in [("operator", "operator"), ("viewer", "viewer")] {
        let (status, _headers, _json) = request(
            app.clone(),
            "POST",
            "/api/users",
            Some(&admin_cookie),
            Some(json!({
                "username": username,
                "password": format!("{username}-password-123A"),
                "role": role
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let operator_cookie = login_cookie(app.clone(), "operator", "operator-password-123A").await;
    let viewer_cookie = login_cookie(app.clone(), "viewer", "viewer-password-123A").await;

    for cookie in [&admin_cookie, &operator_cookie, &viewer_cookie] {
        let (status, _headers, _json) =
            request(app.clone(), "GET", "/api/nodes", Some(cookie), None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _headers, _json) =
            request(app.clone(), "GET", "/api/models", Some(cookie), None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _headers, _json) = request(
            app.clone(),
            "GET",
            "/api/runtime-environments",
            Some(cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _headers, _json) = request(
            app.clone(),
            "GET",
            "/api/model-instances",
            Some(cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _headers, _json) =
            request(app.clone(), "GET", "/api/logs", Some(cookie), None).await;
        assert_eq!(status, StatusCode::OK);
    }

    for (cookie, expected) in [
        (&admin_cookie, StatusCode::OK),
        (&operator_cookie, StatusCode::FORBIDDEN),
        (&viewer_cookie, StatusCode::FORBIDDEN),
    ] {
        let (status, _headers, _json) =
            request(app.clone(), "GET", "/api/users", Some(cookie), None).await;
        assert_eq!(status, expected);
        let (status, _headers, _json) = request(
            app.clone(),
            "PUT",
            "/api/config/server-logs",
            Some(cookie),
            Some(json!({
                "log_dir": "logs",
                "log_level": "info",
                "log_max_file_bytes": 1048576,
                "log_retention_files": 5,
                "log_retention_days": 7
            })),
        )
        .await;
        assert_eq!(status, expected);
    }

    for cookie in [&admin_cookie, &operator_cookie] {
        let (status, _headers, _json) = request(
            app.clone(),
            "POST",
            "/api/models",
            Some(cookie),
            Some(json!({
                "name": "matrix-model",
                "model_type": "llm",
                "initial_file": { "node_id": "missing-node", "path": "/models/missing.gguf" }
            })),
        )
        .await;
        assert_ne!(status, StatusCode::FORBIDDEN);
    }
    let (status, _headers, _json) = request(
        app.clone(),
        "POST",
        "/api/models",
        Some(&viewer_cookie),
        Some(json!({
            "name": "matrix-model",
            "model_type": "llm",
            "initial_file": { "node_id": "missing-node", "path": "/models/missing.gguf" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    for (cookie, expected) in [
        (&admin_cookie, StatusCode::NOT_FOUND),
        (&operator_cookie, StatusCode::FORBIDDEN),
        (&viewer_cookie, StatusCode::FORBIDDEN),
    ] {
        let (status, _headers, _json) = request(
            app.clone(),
            "POST",
            "/api/model-file-trash/missing/cleanup",
            Some(cookie),
            None,
        )
        .await;
        assert_eq!(status, expected);
    }
}

#[tokio::test]
async fn control_plane_audit_records_logged_in_actor() {
    let (app, pool) = test_app().await;
    let admin_cookie = login_cookie(app.clone(), "admin", "admin-password-123").await;

    let (status, _headers, _json) = request(
        app.clone(),
        "POST",
        "/api/models",
        Some(&admin_cookie),
        Some(json!({
            "name": "audit-model",
            "model_type": "llm",
            "initial_file": { "node_id": "missing-node", "path": "/models/missing.gguf" }
        })),
    )
    .await;
    assert_ne!(status, StatusCode::FORBIDDEN);

    let audits = repository::list_audit_events(
        &pool,
        lightai_server::models::AuditQuery {
            operation_type: Some("model.create".to_string()),
            target_type: None,
            target_id: None,
            node_id: None,
            instance_id: None,
            actor_type: Some("user".to_string()),
            result: Some("failed".to_string()),
            from: None,
            limit: None,
            offset: None,
            to: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(audits.events.len(), 1);
    assert_ne!(audits.events[0].actor_id.as_deref(), Some("local"));
    assert!(audits.events[0]
        .detail_json
        .as_deref()
        .unwrap_or_default()
        .contains("\"effective_role\":\"admin\""));
}
