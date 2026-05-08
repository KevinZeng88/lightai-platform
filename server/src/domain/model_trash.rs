use sqlx::{Row, SqlitePool};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

use super::{model_file, node_online, now_unix_secs, Stage3Error, MODEL_FILE_CLEANUP_TIMEOUT_SECS};
use crate::agent_tasks;
use crate::models::{ModelFileTrashListResponse, ModelFileTrashRequest, ModelFileTrashView};

pub async fn create_model_file_trash(
    pool: &SqlitePool,
    model_file_id: &str,
    request: ModelFileTrashRequest,
) -> Result<ModelFileTrashView, Stage3Error> {
    ensure_model_file_trash(pool, model_file_id, request.reason, request.note).await
}

pub(super) async fn ensure_model_file_trash(
    pool: &SqlitePool,
    model_file_id: &str,
    reason: Option<String>,
    note: Option<String>,
) -> Result<ModelFileTrashView, Stage3Error> {
    if let Some(existing_id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM model_file_trash WHERE model_file_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(model_file_id)
    .fetch_optional(pool)
    .await?
    {
        return model_file_trash_item(pool, &existing_id).await;
    }
    let file = model_file(pool, model_file_id).await?;

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO model_file_trash (
            id, model_file_id, model_id, node_id, path, reason, status, note, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(model_file_id)
    .bind(file.model_id)
    .bind(file.node_id)
    .bind(file.path)
    .bind(reason)
    .bind(note)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    model_file_trash_item(pool, &id).await
}

pub async fn list_model_file_trash(
    pool: &SqlitePool,
) -> Result<ModelFileTrashListResponse, Stage3Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            t.*,
            CASE
                WHEN m.deleted_at IS NOT NULL AND instr(m.name, '__deleted__') > 0
                THEN substr(m.name, 1, instr(m.name, '__deleted__') - 1)
                ELSE m.name
            END AS model_name,
            n.name AS node_name
        FROM model_file_trash t
        LEFT JOIN models m ON m.id = t.model_id
        LEFT JOIN nodes n ON n.id = t.node_id
        ORDER BY t.created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(ModelFileTrashListResponse {
        items: rows.into_iter().map(model_file_trash_from_row).collect(),
    })
}

pub async fn cleanup_model_file_trash(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelFileTrashView, Stage3Error> {
    let item = model_file_trash_item(pool, id).await?;
    let node_id = item
        .node_id
        .clone()
        .ok_or_else(|| Stage3Error::BadRequest("trash item has no node".to_string()))?;
    if item.file_deleted_at.is_some() {
        return Ok(item);
    }
    if !node_online(pool, &node_id).await? {
        let message = "Node Agent offline, cannot clean up file";
        update_trash_failure(pool, id, "cleanup_failed", message).await?;
        return Err(Stage3Error::Conflict(message.to_string()));
    }

    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let payload = serde_json::json!({
        "trash_id": id,
        "path": item.path,
    });
    sqlx::query(
        r#"
        INSERT INTO agent_tasks (
            id, node_id, kind, status, payload_json, attempt_count, created_at, updated_at
        )
        VALUES (?, ?, 'cleanup_model_file', 'queued', ?, 0, ?, ?)
        "#,
    )
    .bind(&task_id)
    .bind(&node_id)
    .bind(payload.to_string())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    agent_tasks::notify_agent_tasks();
    sqlx::query(
        r#"
        UPDATE model_file_trash
        SET status = 'cleanup_pending', cleanup_task_id = ?, last_error = NULL, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(&task_id)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    wait_for_model_file_cleanup(pool, id, &task_id).await
}

pub async fn delete_model_file_trash(pool: &SqlitePool, id: &str) -> Result<(), Stage3Error> {
    let result = sqlx::query("DELETE FROM model_file_trash WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(Stage3Error::NotFound("trash item not found".to_string()));
    }
    Ok(())
}

async fn model_file_trash_item(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelFileTrashView, Stage3Error> {
    let row = sqlx::query(
        r#"
        SELECT
            t.*,
            CASE
                WHEN m.deleted_at IS NOT NULL AND instr(m.name, '__deleted__') > 0
                THEN substr(m.name, 1, instr(m.name, '__deleted__') - 1)
                ELSE m.name
            END AS model_name,
            n.name AS node_name
        FROM model_file_trash t
        LEFT JOIN models m ON m.id = t.model_id
        LEFT JOIN nodes n ON n.id = t.node_id
        WHERE t.id = ?
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| Stage3Error::NotFound("trash item not found".to_string()))?;
    Ok(model_file_trash_from_row(row))
}

async fn wait_for_model_file_cleanup(
    pool: &SqlitePool,
    trash_id: &str,
    task_id: &str,
) -> Result<ModelFileTrashView, Stage3Error> {
    let deadline = Instant::now() + Duration::from_secs(MODEL_FILE_CLEANUP_TIMEOUT_SECS);
    loop {
        let row =
            sqlx::query("SELECT status, result_json, error_message FROM agent_tasks WHERE id = ?")
                .bind(task_id)
                .fetch_one(pool)
                .await?;
        let status: String = row.get("status");
        match status.as_str() {
            "succeeded" => return model_file_trash_item(pool, trash_id).await,
            "failed" => {
                let result_json: Option<String> = row.get("result_json");
                let message = result_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                    .and_then(|value| {
                        value
                            .get("message")
                            .and_then(|message| message.as_str())
                            .map(str::to_string)
                    })
                    .or_else(|| row.get::<Option<String>, _>("error_message"))
                    .unwrap_or_else(|| "File cleanup failed".to_string());
                return Err(Stage3Error::Conflict(message));
            }
            "timed_out" => {
                return Err(Stage3Error::Conflict(
                    "file cleanup timed out; confirm Agent is online and retry".to_string(),
                ));
            }
            _ => {}
        }

        if Instant::now() >= deadline {
            agent_tasks::mark_task_timed_out(pool, task_id).await?;
            update_trash_failure(pool, trash_id, "cleanup_timeout", "file cleanup timed out")
                .await?;
            return Err(Stage3Error::Conflict(
                "file cleanup timed out; confirm Agent is online and retry".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn update_trash_failure(
    pool: &SqlitePool,
    trash_id: &str,
    status: &str,
    message: &str,
) -> Result<(), Stage3Error> {
    sqlx::query(
        "UPDATE model_file_trash SET status = ?, last_error = ?, updated_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(message)
    .bind(now_unix_secs())
    .bind(trash_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn model_file_trash_from_row(row: sqlx::sqlite::SqliteRow) -> ModelFileTrashView {
    ModelFileTrashView {
        id: row.get("id"),
        model_file_id: row.get("model_file_id"),
        model_id: row.get("model_id"),
        model_name: row.get("model_name"),
        node_id: row.get("node_id"),
        node_name: row.get("node_name"),
        path: row.get("path"),
        reason: row.get("reason"),
        status: row.get("status"),
        file_deleted_at: row.get("file_deleted_at"),
        cleanup_task_id: row.get("cleanup_task_id"),
        last_error: row.get("last_error"),
        note: row.get("note"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
