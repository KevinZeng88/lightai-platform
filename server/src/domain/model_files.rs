use sqlx::{Row, SqlitePool};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

use super::{
    ensure_model_exists, ensure_node_exists, node_online, now_unix_secs, validate_non_empty,
    validate_path, Stage3Error, MODEL_FILE_VERIFY_TIMEOUT_SECS,
};
use crate::agent_tasks;
use crate::models::{ModelFileListResponse, ModelFileRequest, ModelFileView};
use crate::repository;

pub(crate) struct VerifiedModelFile {
    pub(crate) size_bytes: Option<i64>,
    pub(crate) path_type: Option<String>,
    pub(crate) verified_at: i64,
}

pub async fn create_model_file(
    pool: &SqlitePool,
    model_id: &str,
    request: ModelFileRequest,
) -> Result<ModelFileView, Stage3Error> {
    validate_non_empty("path", &request.path)?;
    validate_path("path", &request.path)?;
    ensure_model_exists(pool, model_id).await?;
    ensure_node_exists(pool, &request.node_id).await?;
    let verified_file =
        verify_model_file_before_save(pool, &request.node_id, &request.path).await?;

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO model_files (
            id, model_id, node_id, path, status, size_bytes, last_verified_at,
            path_type, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, 'verified', ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(model_id)
    .bind(request.node_id)
    .bind(request.path)
    .bind(verified_file.size_bytes)
    .bind(verified_file.verified_at)
    .bind(verified_file.path_type)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    model_file(pool, &id).await
}

pub async fn list_model_files(
    pool: &SqlitePool,
    model_id: &str,
) -> Result<ModelFileListResponse, Stage3Error> {
    ensure_model_exists(pool, model_id).await?;
    let rows = model_file_rows(pool, Some(model_id), None).await?;
    Ok(ModelFileListResponse {
        files: rows.into_iter().map(model_file_from_row).collect(),
    })
}

pub async fn model_file(pool: &SqlitePool, id: &str) -> Result<ModelFileView, Stage3Error> {
    let rows = model_file_rows(pool, None, Some(id)).await?;
    rows.into_iter()
        .next()
        .map(model_file_from_row)
        .ok_or_else(|| Stage3Error::NotFound("model file not found".to_string()))
}

pub async fn update_model_file(
    pool: &SqlitePool,
    id: &str,
    request: ModelFileRequest,
) -> Result<ModelFileView, Stage3Error> {
    validate_non_empty("path", &request.path)?;
    validate_path("path", &request.path)?;
    ensure_node_exists(pool, &request.node_id).await?;
    let verified_file =
        verify_model_file_before_save(pool, &request.node_id, &request.path).await?;

    let now = now_unix_secs();
    let result = sqlx::query(
        r#"
        UPDATE model_files
        SET node_id = ?, path = ?, status = 'verified', size_bytes = ?,
            last_verified_at = ?, path_type = ?, last_error = NULL, verify_task_id = NULL,
            updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(request.node_id)
    .bind(request.path)
    .bind(verified_file.size_bytes)
    .bind(verified_file.verified_at)
    .bind(verified_file.path_type)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(Stage3Error::NotFound("model file not found".to_string()));
    }
    model_file(pool, id).await
}

pub async fn delete_model_file(pool: &SqlitePool, id: &str) -> Result<(), Stage3Error> {
    super::model_trash::ensure_model_file_trash(
        pool,
        id,
        Some("Remove node file path from model".to_string()),
        Some("Only removes the model-file association; actual files are not deleted.".to_string()),
    )
    .await?;
    let now = now_unix_secs();
    let result = sqlx::query("UPDATE model_files SET deleted_at = ?, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(Stage3Error::NotFound("model file not found".to_string()));
    }
    Ok(())
}

pub async fn queue_model_file_verification(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelFileView, Stage3Error> {
    let file = model_file(pool, id).await?;
    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let wait_message = if node_online(pool, &file.node_id).await? {
        "Waiting for node Agent to verify"
    } else {
        "Waiting for node Agent to come online for verification"
    };
    let payload = serde_json::json!({
        "model_file_id": file.id,
        "path": file.path,
    });
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO agent_tasks (
            id, node_id, kind, status, payload_json, attempt_count, created_at, updated_at
        )
        VALUES (?, ?, 'verify_model_file', 'queued', ?, 0, ?, ?)
        "#,
    )
    .bind(&task_id)
    .bind(&file.node_id)
    .bind(payload.to_string())
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        UPDATE model_files
        SET status = 'verify_pending', verify_task_id = ?, last_error = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(&task_id)
    .bind(wait_message)
    .bind(now)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    agent_tasks::notify_agent_tasks();
    model_file(pool, id).await
}

pub(crate) async fn verify_model_file_before_save(
    pool: &SqlitePool,
    node_id: &str,
    path: &str,
) -> Result<VerifiedModelFile, Stage3Error> {
    if !node_online(pool, node_id).await? {
        return Err(Stage3Error::Conflict(
            "Node Agent offline, cannot verify model file".to_string(),
        ));
    }

    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let payload = serde_json::json!({ "path": path });
    sqlx::query(
        r#"
        INSERT INTO agent_tasks (
            id, node_id, kind, status, payload_json, attempt_count, created_at, updated_at
        )
        VALUES (?, ?, 'verify_model_file', 'queued', ?, 0, ?, ?)
        "#,
    )
    .bind(&task_id)
    .bind(node_id)
    .bind(payload.to_string())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    agent_tasks::notify_agent_tasks();

    wait_for_model_file_verification(pool, &task_id).await
}

async fn wait_for_model_file_verification(
    pool: &SqlitePool,
    task_id: &str,
) -> Result<VerifiedModelFile, Stage3Error> {
    let deadline = Instant::now() + Duration::from_secs(MODEL_FILE_VERIFY_TIMEOUT_SECS);
    loop {
        let row =
            sqlx::query("SELECT status, result_json, error_message FROM agent_tasks WHERE id = ?")
                .bind(task_id)
                .fetch_one(pool)
                .await?;
        let status: String = row.get("status");
        match status.as_str() {
            "succeeded" => {
                let result_json: Option<String> = row.get("result_json");
                let result = result_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                    .unwrap_or_else(|| serde_json::json!({}));
                let file_status = result
                    .get("file_status")
                    .and_then(|value| value.as_str())
                    .unwrap_or("verified");
                if file_status != "verified" {
                    return Err(Stage3Error::BadRequest(verification_error_message(&result)));
                }
                return Ok(VerifiedModelFile {
                    size_bytes: result.get("size_bytes").and_then(|value| value.as_i64()),
                    path_type: result
                        .get("path_type")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                        .or_else(|| Some("file".to_string())),
                    verified_at: now_unix_secs(),
                });
            }
            "failed" => {
                let result_json: Option<String> = row.get("result_json");
                let message = result_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                    .map(|value| verification_error_message(&value))
                    .or_else(|| row.get::<Option<String>, _>("error_message"))
                    .unwrap_or_else(|| "Model file verification failed".to_string());
                return Err(Stage3Error::BadRequest(message));
            }
            "timed_out" => {
                return Err(Stage3Error::Conflict(
                    "Model file verification timed out; confirm Agent is online and retry"
                        .to_string(),
                ));
            }
            _ => {}
        }

        if Instant::now() >= deadline {
            agent_tasks::mark_task_timed_out(pool, task_id).await?;
            return Err(Stage3Error::Conflict(
                "Model file verification timed out; confirm Agent is online and retry".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn verification_error_message(result: &serde_json::Value) -> String {
    result
        .get("message")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Model file verification failed")
        .to_string()
}

pub(crate) struct ModelFileSummary {
    pub(crate) file_status: String,
    pub(crate) total_file_count: i64,
    pub(crate) verified_file_count: i64,
    pub(crate) available_node_count: i64,
    pub(crate) last_file_verified_at: Option<i64>,
}

pub(crate) async fn model_file_summary(
    pool: &SqlitePool,
    model_id: &str,
) -> Result<ModelFileSummary, Stage3Error> {
    agent_tasks::mark_timed_out_tasks(pool).await?;
    let rows = sqlx::query(
        r#"
        SELECT status, node_id, last_verified_at
        FROM model_files
        WHERE model_id = ? AND deleted_at IS NULL
        "#,
    )
    .bind(model_id)
    .fetch_all(pool)
    .await?;
    let total_file_count = rows.len() as i64;
    let verified_file_count = rows
        .iter()
        .filter(|row| row.get::<String, _>("status") == "verified")
        .count() as i64;
    let mut verified_nodes = std::collections::BTreeSet::new();
    let mut last_file_verified_at = None;
    let mut has_pending = false;
    let mut has_failed = false;
    for row in &rows {
        let status: String = row.get("status");
        match status.as_str() {
            "verified" => {
                verified_nodes.insert(row.get::<String, _>("node_id"));
            }
            "unverified" | "verify_pending" | "verifying" => has_pending = true,
            _ => has_failed = true,
        }
        if let Some(value) = row.get::<Option<i64>, _>("last_verified_at") {
            last_file_verified_at =
                Some(last_file_verified_at.map_or(value, |current: i64| current.max(value)));
        }
    }
    let file_status = if total_file_count == 0 {
        "no_files"
    } else if verified_file_count == total_file_count {
        "all_files_verified"
    } else if verified_file_count > 0 {
        "partially_verified"
    } else if has_pending && !has_failed {
        "pending_verification"
    } else {
        "verification_failed"
    }
    .to_string();
    Ok(ModelFileSummary {
        file_status,
        total_file_count,
        verified_file_count,
        available_node_count: verified_nodes.len() as i64,
        last_file_verified_at,
    })
}

async fn model_file_rows(
    pool: &SqlitePool,
    model_id: Option<&str>,
    file_id: Option<&str>,
) -> Result<Vec<sqlx::sqlite::SqliteRow>, Stage3Error> {
    agent_tasks::mark_timed_out_tasks(pool).await?;
    let now = now_unix_secs();
    let query = r#"
        SELECT mf.*, m.name AS model_name, n.name AS node_name, n.last_heartbeat_at,
               t.status AS verify_task_status
        FROM model_files mf
        LEFT JOIN models m ON m.id = mf.model_id
        LEFT JOIN nodes n ON n.id = mf.node_id
        LEFT JOIN agent_tasks t ON t.id = mf.verify_task_id
        WHERE (? IS NULL OR mf.model_id = ?)
          AND (? IS NULL OR mf.id = ?)
          AND mf.deleted_at IS NULL
        ORDER BY n.name, mf.path
        "#;
    let rows = sqlx::query(query)
        .bind(model_id)
        .bind(model_id)
        .bind(file_id)
        .bind(file_id)
        .fetch_all(pool)
        .await?;
    for row in &rows {
        let status: String = row.get("status");
        let task_status: Option<String> = row.get("verify_task_status");
        if status == "verifying" && task_status.as_deref() == Some("timed_out") {
            sqlx::query(
                "UPDATE model_files SET status = 'verify_timeout', last_error = 'verification timed out', updated_at = ? WHERE id = ?",
            )
            .bind(now)
            .bind(row.get::<String, _>("id"))
            .execute(pool)
            .await?;
        }
    }
    Ok(rows)
}

fn model_file_from_row(row: sqlx::sqlite::SqliteRow) -> ModelFileView {
    let now = now_unix_secs();
    let last_heartbeat_at: Option<i64> = row.get("last_heartbeat_at");
    let node_status = match last_heartbeat_at {
        Some(last_seen) if now - last_seen <= repository::ONLINE_THRESHOLD_SECS => "online",
        Some(_) => "offline",
        None => "registered",
    }
    .to_string();
    ModelFileView {
        id: row.get("id"),
        model_id: row.get("model_id"),
        model_name: row.get("model_name"),
        node_id: row.get("node_id"),
        node_name: row.get("node_name"),
        node_status,
        path: row.get("path"),
        path_type: row.get("path_type"),
        status: row.get("status"),
        size_bytes: row.get("size_bytes"),
        last_verified_at: row.get("last_verified_at"),
        last_error: row.get("last_error"),
        verify_task_id: row.get("verify_task_id"),
        verify_task_status: row.get("verify_task_status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
