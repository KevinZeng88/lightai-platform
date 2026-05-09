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
async fn model_files_are_registered_per_node_and_drive_model_file_status() {
    let app = test_app().await;
    let registered_a = register_node_json(app.clone()).await;
    let node_a = registered_a["node_id"].as_str().unwrap();
    let token_a = registered_a["agent_token"].as_str().unwrap();
    let registered_b = register_second_node_json(app.clone()).await;
    let node_b = registered_b["node_id"].as_str().unwrap();
    let token_b = registered_b["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_a, token_a).await;
    let model_id = model["id"].as_str().unwrap();

    let file_b =
        create_model_file(app.clone(), model_id, node_b, token_b, "/models/qwen.gguf").await;

    assert_eq!(file_b["node_id"], node_b);
    assert_eq!(file_b["status"], "verified");

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(files["files"].as_array().unwrap().len(), 2);

    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = &models["models"].as_array().unwrap()[0];
    assert_eq!(listed["file_status"], "all_files_verified");
    assert_eq!(listed["verified_file_count"], 2);
    assert_eq!(listed["total_file_count"], 2);
}

#[tokio::test]
async fn model_file_create_fails_when_agent_verification_fails_and_does_not_insert_path() {
    let app = test_app().await;
    let registered_a = register_node_json(app.clone()).await;
    let node_a = registered_a["node_id"].as_str().unwrap();
    let token_a = registered_a["agent_token"].as_str().unwrap();
    let registered_b = register_second_node_json(app.clone()).await;
    let node_b = registered_b["node_id"].as_str().unwrap();
    let token_b = registered_b["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_a, token_a).await;
    let model_id = model["id"].as_str().unwrap();

    let (status, json) = create_model_file_with_agent_result(
        app.clone(),
        model_id,
        node_b,
        token_b,
        "/models/missing.gguf",
        "missing",
        None,
        "file does not exist",
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["message"], "file does not exist");
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(files["files"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_last_model_file_moves_path_to_trash() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let file_id = files["files"].as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap();

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/model-files/{file_id}"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(files["files"].as_array().unwrap().len(), 0);

    let (status, trash_list) = request(app.clone(), "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["model_file_id"], file_id);
    assert_eq!(items[0]["model_id"], model_id);
    assert_eq!(items[0]["path"], "/models/qwen2-7b/model.gguf");

    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        models["models"].as_array().unwrap()[0]["file_status"],
        "no_files"
    );
}

#[tokio::test]
async fn deleting_one_of_multiple_model_files_is_allowed() {
    let app = test_app().await;
    let registered_a = register_node_json(app.clone()).await;
    let node_a = registered_a["node_id"].as_str().unwrap();
    let token_a = registered_a["agent_token"].as_str().unwrap();
    let registered_b = register_second_node_json(app.clone()).await;
    let node_b = registered_b["node_id"].as_str().unwrap();
    let token_b = registered_b["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_a, token_a).await;
    let model_id = model["id"].as_str().unwrap();
    let file_b = create_model_file(
        app.clone(),
        model_id,
        node_b,
        token_b,
        "/models/qwen-b.gguf",
    )
    .await;
    let file_b_id = file_b["id"].as_str().unwrap();

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/model-files/{file_b_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let files = files["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["node_id"], node_a);

    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["model_file_id"], file_b_id);
    assert_eq!(items[0]["path"], "/models/qwen-b.gguf");
}

#[tokio::test]
async fn agent_task_verifies_model_file_and_updates_model_file_status() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let file_id = files["files"].as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap();

    let (status, pending_file) = request(
        app.clone(),
        "POST",
        &format!("/api/model-files/{file_id}/verify"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pending_file["status"], "verify_pending");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/tasks/poll")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let task: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(task["task"]["kind"], "verify_model_file");
    assert_eq!(task["task"]["payload"]["model_file_id"], file_id);
    let task_id = task["task"]["id"].as_str().unwrap();

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(files["files"].as_array().unwrap()[0]["status"], "verifying");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": "succeeded",
                        "result": {
                            "file_status": "verified",
                            "size_bytes": 1234,
                            "message": "file verified"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let verified = &files["files"].as_array().unwrap()[0];
    assert_eq!(verified["status"], "verified");
    assert_eq!(verified["size_bytes"], 1234);
    assert_eq!(verified["last_error"], Value::Null);

    let (status, model) = request(app, "GET", &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(model["file_status"], "all_files_verified");
    assert_eq!(model["verified_file_count"], 1);
}

#[tokio::test]
async fn verification_task_timeout_updates_model_file_status_when_no_agent_reports_back() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let file_id = files["files"].as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap();

    let (status, pending_file) = request(
        app.clone(),
        "POST",
        &format!("/api/model-files/{file_id}/verify"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pending_file["status"], "verify_pending");

    sqlx::query("UPDATE agent_tasks SET created_at = 1, updated_at = 1")
        .execute(&pool)
        .await
        .unwrap();

    let (status, files) = request(app, "GET", &format!("/api/models/{model_id}/files"), None).await;
    assert_eq!(status, StatusCode::OK);
    let timed_out = &files["files"].as_array().unwrap()[0];
    assert_eq!(timed_out["status"], "verify_timeout");
    assert_eq!(timed_out["last_error"], "verification timed out");
}

#[tokio::test]
async fn failed_model_file_verification_keeps_error_and_marks_model_unverified() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let file_id = files["files"].as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap();

    let (status, _) = request(
        app.clone(),
        "POST",
        &format!("/api/model-files/{file_id}/verify"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/tasks/poll")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(json!({ "node_id": node_id }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let task: Value = serde_json::from_slice(&body).unwrap();
    let task_id = task["task"]["id"].as_str().unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/tasks/{task_id}/result"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "node_id": node_id,
                        "status": "failed",
                        "result": {
                            "file_status": "missing",
                            "message": "file does not exist"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let missing = &files["files"].as_array().unwrap()[0];
    assert_eq!(missing["status"], "missing");
    assert_eq!(missing["last_error"], "file does not exist");

    let (status, model) = request(app, "GET", &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(model["file_status"], "verification_failed");
}

#[tokio::test]
async fn deleting_model_moves_all_node_file_paths_to_trash() {
    let app = test_app().await;
    let registered_a = register_node_json(app.clone()).await;
    let node_a = registered_a["node_id"].as_str().unwrap();
    let token_a = registered_a["agent_token"].as_str().unwrap();
    let registered_b = register_second_node_json(app.clone()).await;
    let node_b = registered_b["node_id"].as_str().unwrap();
    let token_b = registered_b["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_a, token_a).await;
    let model_id = model["id"].as_str().unwrap();
    create_model_file(
        app.clone(),
        model_id,
        node_b,
        token_b,
        "/models/qwen-node-b.gguf",
    )
    .await;

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/models/{model_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, models) = request(app.clone(), "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["models"].as_array().unwrap().len(), 0);

    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let mut paths = items
        .iter()
        .map(|item| item["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    assert_eq!(
        paths,
        vec!["/models/qwen-node-b.gguf", "/models/qwen2-7b/model.gguf"]
    );
    assert!(items.iter().all(|item| item["status"] == "pending"));
    assert!(items
        .iter()
        .all(|item| item["file_deleted_at"] == Value::Null));
}

#[tokio::test]
async fn model_file_trash_records_specific_node_file_path() {
    let app = test_app().await;
    let model = create_model(app.clone()).await;
    let (status, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{}/files", model["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let model_file = &files["files"].as_array().unwrap()[0];

    let (status, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({
            "reason": "manual cleanup later",
            "note": "trash with manual cleanup"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trash["status"], "pending");

    let (status, trash_list) = request(app.clone(), "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = trash_list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["model_file_id"], model_file["id"]);
    assert_eq!(items[0]["model_name"], "qwen2-7b");
    assert_eq!(items[0]["node_id"], model_file["node_id"]);
    assert_eq!(items[0]["path"], "/models/qwen2-7b/model.gguf");
    assert_eq!(items[0]["file_deleted_at"], Value::Null);

    let (status, models) = request(app, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["models"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn trash_cleanup_file_is_executed_by_matching_agent_and_updates_status() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file = &files["files"].as_array().unwrap()[0];
    let (status, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({ "reason": "cleanup" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let trash_id = trash["id"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;

    let cleanup_uri = format!("/api/model-file-trash/{trash_id}/cleanup");
    let cleanup_request = request(app.clone(), "POST", &cleanup_uri, None);
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        assert_eq!(task["task"]["kind"], "cleanup_model_file");
        assert_eq!(task["task"]["node_id"], node_id);
        assert_eq!(task["task"]["payload"]["trash_id"], trash_id);
        assert_eq!(task["task"]["payload"]["path"], model_file["path"]);
        let task_id = task["task"]["id"].as_str().unwrap();
        report_cleanup_task_result(app, node_id, token, task_id, "deleted", "file cleaned up")
            .await;
    };
    let ((status, cleaned), _) = tokio::join!(cleanup_request, agent);

    assert_eq!(status, StatusCode::OK);
    assert_eq!(cleaned["status"], "cleaned");
    assert_ne!(cleaned["file_deleted_at"], Value::Null);
    assert_eq!(cleaned["last_error"], Value::Null);
}

#[tokio::test]
async fn trash_cleanup_file_fails_when_agent_is_offline_and_keeps_record() {
    let (app, pool) = test_app_with_pool().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file = &files["files"].as_array().unwrap()[0];
    let (_, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({ "reason": "cleanup" })),
    )
    .await;
    let trash_id = trash["id"].as_str().unwrap();
    sqlx::query("UPDATE nodes SET last_heartbeat_at = 1 WHERE id = ?")
        .bind(node_id)
        .execute(&pool)
        .await
        .unwrap();

    let (status, json) = request(
        app.clone(),
        "POST",
        &format!("/api/model-file-trash/{trash_id}/cleanup"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["message"], "Node Agent offline, cannot clean up file");
    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let item = &trash_list["items"].as_array().unwrap()[0];
    assert_eq!(item["status"], "cleanup_failed");
    assert_eq!(
        item["last_error"],
        "Node Agent offline, cannot clean up file"
    );
}

#[tokio::test]
async fn trash_cleanup_file_failure_keeps_record_and_error() {
    let app = test_app().await;
    let registered = register_node_json(app.clone()).await;
    let node_id = registered["node_id"].as_str().unwrap();
    let token = registered["agent_token"].as_str().unwrap();
    let model = create_model_for_node(app.clone(), node_id, token).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file = &files["files"].as_array().unwrap()[0];
    let (_, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({ "reason": "cleanup" })),
    )
    .await;
    let trash_id = trash["id"].as_str().unwrap();
    heartbeat_node(app.clone(), node_id, token).await;

    let cleanup_uri = format!("/api/model-file-trash/{trash_id}/cleanup");
    let cleanup_request = request(app.clone(), "POST", &cleanup_uri, None);
    let agent = async {
        let task = poll_agent_task(app.clone(), node_id, token).await;
        let task_id = task["task"]["id"].as_str().unwrap();
        report_cleanup_task_result(
            app.clone(),
            node_id,
            token,
            task_id,
            "failed",
            "file does not exist",
        )
        .await;
    };
    let ((status, json), _) = tokio::join!(cleanup_request, agent);

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["message"], "file does not exist");
    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    let item = &trash_list["items"].as_array().unwrap()[0];
    assert_eq!(item["status"], "cleanup_failed");
    assert_eq!(item["last_error"], "file does not exist");
}

#[tokio::test]
async fn trash_record_delete_removes_only_platform_record() {
    let app = test_app().await;
    let model = create_model(app.clone()).await;
    let model_id = model["id"].as_str().unwrap();
    let (_, files) = request(
        app.clone(),
        "GET",
        &format!("/api/models/{model_id}/files"),
        None,
    )
    .await;
    let model_file = &files["files"].as_array().unwrap()[0];
    let (_, trash) = request(
        app.clone(),
        "POST",
        &format!(
            "/api/model-files/{}/trash",
            model_file["id"].as_str().unwrap()
        ),
        Some(json!({ "reason": "remove record" })),
    )
    .await;
    let trash_id = trash["id"].as_str().unwrap();

    let (status, _) = request(
        app.clone(),
        "DELETE",
        &format!("/api/model-file-trash/{trash_id}"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let (status, trash_list) = request(app, "GET", "/api/model-file-trash", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trash_list["items"].as_array().unwrap().len(), 0);
}
