use sqlx::{Row, SqlitePool};
use std::sync::{Arc, OnceLock};

use tokio::sync::Notify;
use tokio::time::{timeout, Duration, Instant};

use crate::domain::Stage3Error;
use crate::models::{AgentTaskPollResponse, AgentTaskResultRequest, AgentTaskView};
use crate::repository;

const AGENT_TASK_LEASE_SECS: i64 = 30;
const AGENT_TASK_QUEUE_TIMEOUT_SECS: i64 = 300;
const AGENT_TASK_LONG_POLL_SECS: u64 = 25;
static TASK_NOTIFY: OnceLock<Arc<Notify>> = OnceLock::new();

pub fn task_notify() -> Arc<Notify> {
    TASK_NOTIFY.get_or_init(|| Arc::new(Notify::new())).clone()
}

pub fn notify_agent_tasks() {
    task_notify().notify_waiters();
}

pub async fn poll_agent_task(
    pool: &SqlitePool,
    node_id: &str,
    current_config_version: Option<i64>,
) -> Result<AgentTaskPollResponse, Stage3Error> {
    let deadline = Instant::now() + Duration::from_secs(AGENT_TASK_LONG_POLL_SECS);
    let row = loop {
        mark_timed_out_tasks(pool).await?;
        if let Some(row) = next_queued_agent_task(pool, node_id).await? {
            break row;
        }
        if Instant::now() >= deadline {
            return Ok(AgentTaskPollResponse {
                task: None,
                agent_config: repository::effective_agent_config(pool, node_id).await?,
            });
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_secs(AGENT_TASK_LONG_POLL_SECS));
        if timeout(wait, task_notify().notified()).await.is_ok() {
            let effective_config = repository::effective_agent_config(pool, node_id).await?;
            if next_queued_agent_task(pool, node_id).await?.is_none()
                && current_config_version
                    .is_some_and(|version| version != effective_config.config_version)
            {
                return Ok(AgentTaskPollResponse {
                    task: None,
                    agent_config: effective_config,
                });
            }
        }
    };
    let task_id: String = row.get("id");
    let now = crate::util::now_unix_secs();
    let lease_until = now + AGENT_TASK_LEASE_SECS;
    sqlx::query(
        r#"
        UPDATE agent_tasks
        SET status = 'running', started_at = COALESCE(started_at, ?),
            lease_until = ?, attempt_count = attempt_count + 1, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(now)
    .bind(lease_until)
    .bind(now)
    .bind(&task_id)
    .execute(pool)
    .await?;
    if row.get::<String, _>("kind") == "verify_model_file" {
        if let Ok(payload) =
            serde_json::from_str::<serde_json::Value>(&row.get::<String, _>("payload_json"))
        {
            if let Some(model_file_id) = payload
                .get("model_file_id")
                .and_then(|value| value.as_str())
            {
                sqlx::query(
                    "UPDATE model_files SET status = 'verifying', updated_at = ? WHERE id = ?",
                )
                .bind(now)
                .bind(model_file_id)
                .execute(pool)
                .await?;
            }
        }
    } else if row.get::<String, _>("kind") == "cleanup_model_file" {
        if let Ok(payload) =
            serde_json::from_str::<serde_json::Value>(&row.get::<String, _>("payload_json"))
        {
            if let Some(trash_id) = payload.get("trash_id").and_then(|value| value.as_str()) {
                sqlx::query(
                    "UPDATE model_file_trash SET status = 'cleanup_running', updated_at = ? WHERE id = ?",
                )
                .bind(now)
                .bind(trash_id)
                .execute(pool)
                .await?;
            }
        }
    }
    let row = sqlx::query("SELECT * FROM agent_tasks WHERE id = ?")
        .bind(&task_id)
        .fetch_one(pool)
        .await?;
    Ok(AgentTaskPollResponse {
        task: Some(agent_task_from_row(row)?),
        agent_config: repository::effective_agent_config(pool, node_id).await?,
    })
}

async fn next_queued_agent_task(
    pool: &SqlitePool,
    node_id: &str,
) -> Result<Option<sqlx::sqlite::SqliteRow>, Stage3Error> {
    Ok(sqlx::query(
        r#"
        SELECT * FROM agent_tasks
        WHERE node_id = ? AND status = 'queued'
        ORDER BY created_at
        LIMIT 1
        "#,
    )
    .bind(node_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn record_agent_task_result(
    pool: &SqlitePool,
    task_id: &str,
    request: AgentTaskResultRequest,
) -> Result<(), Stage3Error> {
    let task = sqlx::query("SELECT * FROM agent_tasks WHERE id = ? AND node_id = ?")
        .bind(task_id)
        .bind(&request.node_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Stage3Error::NotFound("agent task not found".to_string()))?;
    if !["succeeded", "failed"].contains(&request.status.as_str()) {
        return Err(Stage3Error::BadRequest("status is invalid".to_string()));
    }

    let now = crate::util::now_unix_secs();
    let result_json = request.result.to_string();
    let error_message = request
        .result
        .get("message")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE agent_tasks
        SET status = ?, result_json = ?, error_message = ?, completed_at = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(&request.status)
    .bind(result_json)
    .bind(if request.status == "failed" {
        error_message.as_deref()
    } else {
        None
    })
    .bind(now)
    .bind(now)
    .bind(task_id)
    .execute(&mut *tx)
    .await?;

    if task.get::<String, _>("kind") == "verify_model_file" {
        let payload: serde_json::Value =
            serde_json::from_str(&task.get::<String, _>("payload_json"))
                .map_err(|error| Stage3Error::Internal(error.into()))?;
        let model_file_id = payload
            .get("model_file_id")
            .and_then(|value| value.as_str());
        let file_status = request
            .result
            .get("file_status")
            .and_then(|value| value.as_str())
            .unwrap_or(if request.status == "succeeded" {
                "verified"
            } else {
                "failed"
            });
        let size_bytes = request
            .result
            .get("size_bytes")
            .and_then(|value| value.as_i64());
        let path_type = request
            .result
            .get("path_type")
            .and_then(|value| value.as_str())
            .unwrap_or("file");
        let last_error = if file_status == "verified" {
            None
        } else {
            error_message
                .as_deref()
                .or(Some("File verification failed"))
        };
        if let Some(model_file_id) = model_file_id {
            sqlx::query(
                r#"
                UPDATE model_files
                SET status = ?, size_bytes = ?, path_type = ?, last_verified_at = ?, last_error = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(file_status)
            .bind(size_bytes)
            .bind(path_type)
            .bind(now)
            .bind(last_error)
            .bind(now)
            .bind(model_file_id)
            .execute(&mut *tx)
            .await?;
        }
    } else if task.get::<String, _>("kind") == "cleanup_model_file" {
        let payload: serde_json::Value =
            serde_json::from_str(&task.get::<String, _>("payload_json"))
                .map_err(|error| Stage3Error::Internal(error.into()))?;
        let trash_id = payload
            .get("trash_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| Stage3Error::BadRequest("task payload is invalid".to_string()))?;
        let message = error_message
            .as_deref()
            .unwrap_or(if request.status == "succeeded" {
                "file cleaned up"
            } else {
                "File cleanup failed"
            });
        if request.status == "succeeded" {
            sqlx::query(
                r#"
                UPDATE model_file_trash
                SET status = 'cleaned', file_deleted_at = ?, last_error = NULL, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(now)
            .bind(now)
            .bind(trash_id)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                r#"
                UPDATE model_file_trash
                SET status = 'cleanup_failed', last_error = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(message)
            .bind(now)
            .bind(trash_id)
            .execute(&mut *tx)
            .await?;
        }
    } else if task.get::<String, _>("kind") == "read_instance_log" {
        // handled by refresh_instance_logs polling; no extra state update needed
    } else if matches!(
        task.get::<String, _>("kind").as_str(),
        "start_model_instance" | "stop_model_instance" | "test_model_instance"
    ) {
        let payload: serde_json::Value =
            serde_json::from_str(&task.get::<String, _>("payload_json"))
                .map_err(|error| Stage3Error::Internal(error.into()))?;
        let instance_id = payload
            .get("instance_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| Stage3Error::BadRequest("task payload is invalid".to_string()))?;
        let kind = task.get::<String, _>("kind");
        let next_status = request
            .result
            .get("instance_status")
            .and_then(|value| value.as_str())
            .unwrap_or(if request.status == "succeeded" {
                if kind == "stop_model_instance" {
                    "stopped"
                } else {
                    "running"
                }
            } else {
                "failed"
            });
        let mut last_error = if request.status == "succeeded" {
            None
        } else {
            error_message
                .clone()
                .or(Some("Instance task execution failed".to_string()))
        };
        if kind == "stop_model_instance" && request.status == "succeeded" {
            last_error = request
                .result
                .get("message")
                .and_then(|value| value.as_str())
                .map(str::to_string);
        }
        if kind == "start_model_instance" && request.status == "succeeded" {
            last_error = request
                .result
                .get("message")
                .and_then(|value| value.as_str())
                .map(str::to_string);
        }
        if kind == "test_model_instance" && request.status == "succeeded" {
            let message = request
                .result
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("Test succeeded");
            let summary = request
                .result
                .get("response_summary")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            last_error = Some(if summary.trim().is_empty() {
                message.to_string()
            } else {
                format!("{message}：{summary}")
            });
        }
        let base_url = request
            .result
            .get("base_url")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let endpoint_url = request
            .result
            .get("endpoint_url")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let process_id = request
            .result
            .get("process_id")
            .and_then(|value| value.as_i64());
        let process_ref = request
            .result
            .get("process_ref")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let log_tail = request
            .result
            .get("log_tail")
            .and_then(|value| value.as_str())
            .map(|value| value.chars().take(8192).collect::<String>());
        let command = request
            .result
            .get("command")
            .and_then(|value| value.as_str())
            .map(|value| value.chars().take(2048).collect::<String>());
        sqlx::query(
            r#"
            UPDATE model_instances
            SET status = ?,
                base_url = COALESCE(?, base_url),
                endpoint_url = COALESCE(?, endpoint_url),
                process_id = CASE WHEN ? = 'stop_model_instance' THEN NULL ELSE COALESCE(?, process_id) END,
                process_ref = CASE WHEN ? = 'stop_model_instance' THEN NULL ELSE COALESCE(?, process_ref) END,
                log_tail = COALESCE(?, log_tail),
                command = COALESCE(?, command),
                last_checked_at = ?, last_error = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(next_status)
        .bind(base_url)
        .bind(endpoint_url)
        .bind(&kind)
        .bind(process_id)
        .bind(&kind)
        .bind(process_ref)
        .bind(log_tail)
        .bind(command)
        .bind(now)
        .bind(last_error)
        .bind(now)
        .bind(instance_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn mark_timed_out_tasks(pool: &SqlitePool) -> Result<(), Stage3Error> {
    let now = crate::util::now_unix_secs();
    let rows = sqlx::query(
        r#"
        SELECT id, kind, payload_json
        FROM agent_tasks
        WHERE (status = 'running' AND lease_until IS NOT NULL AND lease_until < ?)
           OR (status = 'queued' AND created_at < ?)
        "#,
    )
    .bind(now)
    .bind(now - AGENT_TASK_QUEUE_TIMEOUT_SECS)
    .fetch_all(pool)
    .await?;
    for row in rows {
        let task_id: String = row.get("id");
        mark_task_timed_out(pool, &task_id).await?;
        if row.get::<String, _>("kind") == "verify_model_file" {
            if let Ok(payload) =
                serde_json::from_str::<serde_json::Value>(&row.get::<String, _>("payload_json"))
            {
                if let Some(model_file_id) = payload
                    .get("model_file_id")
                    .and_then(|value| value.as_str())
                {
                    sqlx::query(
                        "UPDATE model_files SET status = 'verify_timeout', last_error = 'verification timed out', updated_at = ? WHERE id = ?",
                    )
                    .bind(now)
                    .bind(model_file_id)
                    .execute(pool)
                    .await?;
                }
            }
        } else if row.get::<String, _>("kind") == "cleanup_model_file" {
            if let Ok(payload) =
                serde_json::from_str::<serde_json::Value>(&row.get::<String, _>("payload_json"))
            {
                if let Some(trash_id) = payload.get("trash_id").and_then(|value| value.as_str()) {
                    crate::domain::update_trash_failure(
                        pool,
                        trash_id,
                        "cleanup_timeout",
                        "file cleanup timed out",
                    )
                    .await?;
                }
            }
        } else if matches!(
            row.get::<String, _>("kind").as_str(),
            "start_model_instance" | "stop_model_instance" | "test_model_instance"
        ) {
            if let Ok(payload) =
                serde_json::from_str::<serde_json::Value>(&row.get::<String, _>("payload_json"))
            {
                if let Some(instance_id) =
                    payload.get("instance_id").and_then(|value| value.as_str())
                {
                    crate::domain::update_instance_check(
                        pool,
                        instance_id,
                        "failed",
                        Some("Local instance task timed out"),
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn mark_task_timed_out(
    pool: &SqlitePool,
    task_id: &str,
) -> Result<(), Stage3Error> {
    sqlx::query(
        "UPDATE agent_tasks SET status = 'timed_out', error_message = 'task execution timed out', updated_at = ? WHERE id = ? AND status IN ('queued', 'running')",
    )
    .bind(crate::util::now_unix_secs())
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn agent_task_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AgentTaskView, Stage3Error> {
    let payload_json: String = row.get("payload_json");
    let payload =
        serde_json::from_str(&payload_json).map_err(|error| Stage3Error::Internal(error.into()))?;
    Ok(AgentTaskView {
        id: row.get("id"),
        node_id: row.get("node_id"),
        kind: row.get("kind"),
        status: row.get("status"),
        payload,
        lease_until: row.get("lease_until"),
        attempt_count: row.get("attempt_count"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}
