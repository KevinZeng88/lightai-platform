use sqlx::{Row, SqlitePool};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

use super::{
    model_instance, node_online, now_unix_secs, runtime_environment, DomainError,
    INSTANCE_LOG_TIMEOUT_SECS, LOG_READ_TIMEOUT_SECS,
};
use crate::agent_tasks;

pub async fn read_agent_log(
    pool: &SqlitePool,
    node_id: &str,
    max_bytes: usize,
) -> Result<String, DomainError> {
    if !node_online(pool, node_id).await? {
        return Err(DomainError::Conflict(
            "Node Agent offline, cannot read Agent log".to_string(),
        ));
    }
    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let payload = serde_json::json!({
        "log_type": "agent_service",
        "file_name": "lightai-agent.log",
        "max_bytes": max_bytes.min(512 * 1024)
    });
    sqlx::query(
        r#"
        INSERT INTO agent_tasks (
            id, node_id, kind, status, payload_json, attempt_count, created_at, updated_at
        )
        VALUES (?, ?, 'read_agent_log', 'queued', ?, 0, ?, ?)
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

    let deadline = Instant::now() + Duration::from_secs(LOG_READ_TIMEOUT_SECS);
    loop {
        let row =
            sqlx::query("SELECT status, result_json, error_message FROM agent_tasks WHERE id = ?")
                .bind(&task_id)
                .fetch_one(pool)
                .await?;
        let status: String = row.get("status");
        match status.as_str() {
            "succeeded" => {
                let result_json: Option<String> = row.get("result_json");
                let result = result_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                    .unwrap_or_default();
                return Ok(result
                    .get("content")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string());
            }
            "failed" => {
                return Err(DomainError::Conflict(
                    row.get::<Option<String>, _>("error_message")
                        .unwrap_or_else(|| "Agent log read failed".to_string()),
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            agent_tasks::mark_task_timed_out(pool, &task_id).await?;
            return Err(DomainError::Conflict(
                "Agent log read timed out".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub async fn refresh_instance_logs(pool: &SqlitePool, id: &str) -> Result<String, DomainError> {
    let instance = model_instance(pool, id).await?;
    if instance.deploy_type != "local" {
        return Err(DomainError::BadRequest(
            "External instances are not managed by the platform Agent and do not support log refresh. (外部实例不由平台 Agent 管理，不支持读取日志)".to_string(),
        ));
    }
    let node_id = instance
        .node_id
        .as_deref()
        .ok_or_else(|| DomainError::BadRequest("Local instance missing node".to_string()))?;
    if !node_online(pool, node_id).await? {
        return Err(DomainError::Conflict(
            "Node Agent offline, cannot refresh instance log".to_string(),
        ));
    }

    let runtime_environment_id = instance.runtime_environment_id.as_deref().ok_or_else(|| {
        DomainError::BadRequest("Local instance missing runtime environment".to_string())
    })?;
    let env = runtime_environment(pool, runtime_environment_id).await?;

    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let payload = serde_json::json!({
        "instance_id": id,
        "backend": env.backend,
        "runtime_environment_id": runtime_environment_id,
        "log_dir": env.log_dir,
        "max_bytes": 64 * 1024_u64,
    });
    sqlx::query(
        r#"
        INSERT INTO agent_tasks (
            id, node_id, kind, status, payload_json, attempt_count, created_at, updated_at
        )
        VALUES (?, ?, 'read_instance_log', 'queued', ?, 0, ?, ?)
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

    let deadline = Instant::now() + Duration::from_secs(INSTANCE_LOG_TIMEOUT_SECS);
    loop {
        let row =
            sqlx::query("SELECT status, result_json, error_message FROM agent_tasks WHERE id = ?")
                .bind(&task_id)
                .fetch_one(pool)
                .await?;
        let status: String = row.get("status");
        match status.as_str() {
            "succeeded" => {
                let result_json: Option<String> = row.get("result_json");
                let result = result_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                    .unwrap_or_default();
                let content = result
                    .get("content")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string();
                let log_message = result
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string();
                let log_tail = if log_message.is_empty() {
                    content.clone()
                } else {
                    format!("【{log_message}】\n{content}")
                };
                sqlx::query("UPDATE model_instances SET log_tail = ?, updated_at = ? WHERE id = ?")
                    .bind(&log_tail)
                    .bind(now_unix_secs())
                    .bind(id)
                    .execute(pool)
                    .await?;
                return Ok(log_tail);
            }
            "failed" => {
                return Err(DomainError::Conflict(
                    row.get::<Option<String>, _>("error_message")
                        .unwrap_or_else(|| "Instance log read failed".to_string()),
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            agent_tasks::mark_task_timed_out(pool, &task_id).await?;
            return Err(DomainError::Conflict(
                "Instance log read timed out".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub async fn ollama_model_list(
    pool: &SqlitePool,
    node_id: &str,
    runtime_env_id: &str,
) -> Result<Vec<serde_json::Value>, DomainError> {
    if !node_online(pool, node_id).await? {
        return Err(DomainError::Conflict(
            "Node Agent offline, cannot query Ollama models".to_string(),
        ));
    }
    let env = runtime_environment(pool, runtime_env_id).await?;
    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let payload = serde_json::json!({
        "runtime_environment_id": runtime_env_id,
        "runtime_params": env.params_json
            .as_deref()
            .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
            .unwrap_or_default(),
    });
    sqlx::query(
        r#"
        INSERT INTO agent_tasks (
            id, node_id, kind, status, payload_json, attempt_count, created_at, updated_at
        )
        VALUES (?, ?, 'list_ollama_models', 'queued', ?, 0, ?, ?)
        "#,
    )
    .bind(&task_id)
    .bind(node_id)
    .bind(payload.to_string())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    crate::agent_tasks::notify_agent_tasks();

    let deadline = Instant::now() + Duration::from_secs(LOG_READ_TIMEOUT_SECS);
    loop {
        let row =
            sqlx::query("SELECT status, result_json, error_message FROM agent_tasks WHERE id = ?")
                .bind(&task_id)
                .fetch_one(pool)
                .await?;
        let status: String = row.get("status");
        match status.as_str() {
            "succeeded" => {
                let result_json: Option<String> = row.get("result_json");
                let result = result_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                    .unwrap_or_default();
                let models = result
                    .get("models")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                return Ok(models);
            }
            "failed" => {
                return Err(DomainError::Conflict(
                    row.get::<Option<String>, _>("error_message")
                        .unwrap_or_else(|| "Ollama model list query failed".to_string()),
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            crate::agent_tasks::mark_task_timed_out(pool, &task_id).await?;
            return Err(DomainError::Conflict(
                "Ollama model list query timed out".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub async fn frontend_error_summary(pool: &SqlitePool) -> Result<String, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT occurred_at, error_message, detail_json
        FROM audit_events
        WHERE operation_type = 'frontend_error'
        ORDER BY occurred_at DESC
        LIMIT 50
        "#,
    )
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok("No frontend errors".to_string());
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            let ts: i64 = row.get("occurred_at");
            let message: Option<String> = row.get("error_message");
            let detail: Option<String> = row.get("detail_json");
            let url_info = detail
                .as_deref()
                .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
                .and_then(|v| v.get("url").and_then(|u| u.as_str()).map(str::to_string))
                .map(|u| format!(" [URL: {u}]"))
                .unwrap_or_default();
            format!(
                "{} Frontend error: {}{}",
                ts,
                message.as_deref().unwrap_or("no details"),
                url_info
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

pub async fn recent_error_summary(pool: &SqlitePool) -> Result<String, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT 'instance' AS source, name AS target, last_error AS message, updated_at AS ts
        FROM model_instances
        WHERE last_error IS NOT NULL AND TRIM(last_error) != ''
        UNION ALL
        SELECT 'model_file' AS source, path AS target, last_error AS message, updated_at AS ts
        FROM model_files
        WHERE last_error IS NOT NULL AND TRIM(last_error) != ''
        UNION ALL
        SELECT 'trash' AS source, path AS target, last_error AS message, updated_at AS ts
        FROM model_file_trash
        WHERE last_error IS NOT NULL AND TRIM(last_error) != ''
        ORDER BY ts DESC
        LIMIT 50
        "#,
    )
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok("No error summary available".to_string());
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            format!(
                "{} [{}] {}: {}",
                row.get::<i64, _>("ts"),
                row.get::<String, _>("source"),
                row.get::<String, _>("target"),
                crate::platform_log::sanitize(&row.get::<String, _>("message"))
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}
