use sqlx::{Row, SqlitePool};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

use super::{node_online, now_unix_secs, validate_path, DomainError};
use crate::agent_tasks;
use crate::models::{GatewayTaskRequest, GatewayTaskResponse};

const GATEWAY_TASK_TIMEOUT_SECS: u64 = 10;

pub async fn start_gateway(
    pool: &SqlitePool,
    node_id: &str,
    request: GatewayTaskRequest,
) -> Result<GatewayTaskResponse, DomainError> {
    run_gateway_task(pool, node_id, "start_gateway", request).await
}

pub async fn stop_gateway(
    pool: &SqlitePool,
    node_id: &str,
    request: GatewayTaskRequest,
) -> Result<GatewayTaskResponse, DomainError> {
    run_gateway_task(pool, node_id, "stop_gateway", request).await
}

pub async fn restart_gateway(
    pool: &SqlitePool,
    node_id: &str,
    request: GatewayTaskRequest,
) -> Result<GatewayTaskResponse, DomainError> {
    run_gateway_task(pool, node_id, "restart_gateway", request).await
}

pub async fn check_gateway(
    pool: &SqlitePool,
    node_id: &str,
    request: GatewayTaskRequest,
) -> Result<GatewayTaskResponse, DomainError> {
    run_gateway_task(pool, node_id, "check_gateway", request).await
}

pub async fn read_gateway_log(
    pool: &SqlitePool,
    node_id: &str,
    request: GatewayTaskRequest,
) -> Result<GatewayTaskResponse, DomainError> {
    run_gateway_task(pool, node_id, "read_gateway_log", request).await
}

async fn run_gateway_task(
    pool: &SqlitePool,
    node_id: &str,
    task_kind: &str,
    request: GatewayTaskRequest,
) -> Result<GatewayTaskResponse, DomainError> {
    ensure_node_can_run_gateway_task(pool, node_id).await?;
    validate_gateway_request(task_kind, &request)?;

    let payload = gateway_payload(task_kind, request);
    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    sqlx::query(
        r#"
        INSERT INTO agent_tasks (
            id, node_id, kind, status, payload_json, attempt_count, created_at, updated_at
        )
        VALUES (?, ?, ?, 'queued', ?, 0, ?, ?)
        "#,
    )
    .bind(&task_id)
    .bind(node_id)
    .bind(task_kind)
    .bind(payload.to_string())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    agent_tasks::notify_agent_tasks();
    wait_for_gateway_task(pool, &task_id).await
}

async fn ensure_node_can_run_gateway_task(
    pool: &SqlitePool,
    node_id: &str,
) -> Result<(), DomainError> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM nodes WHERE id = ?")
        .bind(node_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(DomainError::NotFound("node not found".to_string()));
    }
    if !node_online(pool, node_id).await? {
        return Err(DomainError::Conflict(
            "Node Agent offline, cannot execute Gateway task".to_string(),
        ));
    }
    Ok(())
}

fn validate_gateway_request(
    task_kind: &str,
    request: &GatewayTaskRequest,
) -> Result<(), DomainError> {
    match task_kind {
        "start_gateway" | "restart_gateway" => {
            validate_required_path("binary_path", request.binary_path.as_deref())?;
            validate_required_path("config_path", request.config_path.as_deref())?;
            validate_required_path("work_dir", request.work_dir.as_deref())?;
            validate_required_path("log_path", request.log_path.as_deref())?;
            validate_required_url("health_url", request.health_url.as_deref())?;
        }
        "stop_gateway" | "check_gateway" | "read_gateway_log" => {}
        _ => {
            return Err(DomainError::BadRequest(
                "Gateway task kind is invalid".to_string(),
            ))
        }
    }
    Ok(())
}

fn validate_required_path(field: &str, value: Option<&str>) -> Result<(), DomainError> {
    let Some(value) = value else {
        return Err(DomainError::BadRequest(format!("{field} is required")));
    };
    validate_path(field, value)
}

fn validate_required_url(field: &str, value: Option<&str>) -> Result<(), DomainError> {
    let Some(value) = value else {
        return Err(DomainError::BadRequest(format!("{field} is required")));
    };
    let parsed = reqwest::Url::parse(value)
        .map_err(|_| DomainError::BadRequest(format!("{field} is invalid")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => Err(DomainError::BadRequest(format!(
            "{field} must use http:// or https://"
        )))?,
    }
    match parsed.host_str() {
        Some("127.0.0.1" | "localhost" | "::1") => {}
        _ => {
            return Err(DomainError::BadRequest(format!(
                "{field} must target localhost or loopback"
            )))
        }
    }
    if parsed.path() != "/health" {
        return Err(DomainError::BadRequest(format!(
            "{field} path must be /health"
        )));
    }
    if parsed.query().is_some() {
        return Err(DomainError::BadRequest(format!(
            "{field} must not include query parameters"
        )));
    }
    Ok(())
}

fn gateway_payload(task_kind: &str, request: GatewayTaskRequest) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    if matches!(task_kind, "start_gateway" | "restart_gateway") {
        insert_opt(&mut payload, "binary_path", request.binary_path);
        insert_opt(&mut payload, "config_path", request.config_path);
        insert_opt(&mut payload, "work_dir", request.work_dir);
        insert_opt(&mut payload, "log_path", request.log_path);
        insert_opt(&mut payload, "health_url", request.health_url);
    }
    if let Some(max_bytes) = request.max_bytes {
        payload.insert("max_bytes".to_string(), serde_json::json!(max_bytes));
    }
    serde_json::Value::Object(payload)
}

fn insert_opt(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value {
        map.insert(key.to_string(), serde_json::Value::String(value));
    }
}

async fn wait_for_gateway_task(
    pool: &SqlitePool,
    task_id: &str,
) -> Result<GatewayTaskResponse, DomainError> {
    let deadline = Instant::now() + Duration::from_secs(GATEWAY_TASK_TIMEOUT_SECS);
    loop {
        let row =
            sqlx::query("SELECT status, result_json, error_message FROM agent_tasks WHERE id = ?")
                .bind(task_id)
                .fetch_one(pool)
                .await?;
        let status: String = row.get("status");
        match status.as_str() {
            "succeeded" => return gateway_response_from_row(&row),
            "failed" => {
                let response = gateway_response_from_row(&row).unwrap_or_else(|_| {
                    let message = row
                        .get::<Option<String>, _>("error_message")
                        .unwrap_or_else(|| "Gateway task failed".to_string());
                    GatewayTaskResponse {
                        gateway_status: "failed".to_string(),
                        message,
                        process_id: None,
                        process_ref: None,
                        health_url: None,
                        log_tail: None,
                        command: None,
                    }
                });
                return Err(DomainError::Conflict(response.message));
            }
            "timed_out" => {
                return Err(DomainError::Conflict(
                    "Gateway task timed out; confirm Agent is online and retry".to_string(),
                ))
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            agent_tasks::mark_task_timed_out(pool, task_id).await?;
            return Err(DomainError::Conflict(
                "Gateway task timed out; confirm Agent is online and retry".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn gateway_response_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<GatewayTaskResponse, DomainError> {
    let result_json: Option<String> = row.get("result_json");
    let Some(result_json) = result_json else {
        return Err(DomainError::Conflict(
            "Gateway task has no result".to_string(),
        ));
    };
    serde_json::from_str::<GatewayTaskResponse>(&result_json)
        .map_err(|error| DomainError::Internal(error.into()))
}
