use sqlx::{Row, SqlitePool};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

use super::{
    bool_to_int, ensure_node_online, int_to_bool, node_online, now_unix_secs, validate_backend,
    validate_deploy_type, validate_json_field, validate_non_empty, validate_runtime_entrypoints,
    DomainError, RUNTIME_ENVIRONMENT_CHECK_TIMEOUT_SECS,
};
use crate::agent_tasks;
use crate::models::{
    RuntimeEnvironmentListResponse, RuntimeEnvironmentRequest, RuntimeEnvironmentView,
};

pub async fn create_runtime_environment(
    pool: &SqlitePool,
    node_id: &str,
    request: RuntimeEnvironmentRequest,
) -> Result<RuntimeEnvironmentView, DomainError> {
    validate_non_empty("name", &request.name)?;
    validate_backend(&request.backend)?;
    validate_deploy_type(&request.deploy_type)?;
    validate_runtime_entrypoints(&request)?;
    validate_json_field(
        "allowed_model_dirs_json",
        request.allowed_model_dirs_json.as_deref(),
    )?;
    validate_json_field("params_json", request.params_json.as_deref())?;
    let checked = if request.backend == "ollama" {
        ollama_runtime_config_check()
    } else {
        ensure_node_online(pool, node_id).await?;
        check_runtime_environment_before_save(pool, node_id, &request).await?
    };

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO runtime_environments (
            id, node_id, name, backend, deploy_type, version, base_url, health_url, endpoint_url,
            binary_path, docker_image, working_dir, log_dir, allowed_model_dirs_json,
            params_json, enabled, last_checked_at, check_status, check_message, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(node_id)
    .bind(request.name)
    .bind(request.backend)
    .bind(request.deploy_type)
    .bind(checked.version.or(request.version))
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(request.binary_path)
    .bind(request.docker_image)
    .bind(request.working_dir)
    .bind(request.log_dir)
    .bind(request.allowed_model_dirs_json)
    .bind(request.params_json)
    .bind(bool_to_int(request.enabled.unwrap_or(true)))
    .bind(checked.checked_at)
    .bind(checked.check_status)
    .bind(checked.message)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    runtime_environment(pool, &id).await
}

pub async fn list_runtime_environments(
    pool: &SqlitePool,
    node_id: Option<&str>,
) -> Result<RuntimeEnvironmentListResponse, DomainError> {
    let rows = match node_id {
        Some(node_id) => {
            sqlx::query(
                r#"
                SELECT * FROM runtime_environments
                WHERE node_id = ?
                ORDER BY name
                "#,
            )
            .bind(node_id)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query("SELECT * FROM runtime_environments ORDER BY node_id, name")
                .fetch_all(pool)
                .await?
        }
    };

    Ok(RuntimeEnvironmentListResponse {
        runtime_environments: rows.into_iter().map(runtime_environment_from_row).collect(),
    })
}

pub async fn runtime_environment(
    pool: &SqlitePool,
    id: &str,
) -> Result<RuntimeEnvironmentView, DomainError> {
    let row = sqlx::query("SELECT * FROM runtime_environments WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| DomainError::NotFound("runtime environment not found".to_string()))?;
    Ok(runtime_environment_from_row(row))
}

pub async fn update_runtime_environment(
    pool: &SqlitePool,
    id: &str,
    request: RuntimeEnvironmentRequest,
) -> Result<RuntimeEnvironmentView, DomainError> {
    validate_non_empty("name", &request.name)?;
    validate_backend(&request.backend)?;
    validate_deploy_type(&request.deploy_type)?;
    validate_runtime_entrypoints(&request)?;
    validate_json_field(
        "allowed_model_dirs_json",
        request.allowed_model_dirs_json.as_deref(),
    )?;
    validate_json_field("params_json", request.params_json.as_deref())?;

    let running_instances: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM model_instances WHERE runtime_environment_id = ? AND status IN ('running', 'starting', 'stopping')",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    if !running_instances.is_empty() {
        return Err(DomainError::Conflict(format!(
            "Runtime in use by running instance {}. Cannot modify. Stop the instance first.",
            running_instances.join(", ")
        )));
    }

    let is_ollama = request.backend == "ollama";
    let now = now_unix_secs();
    let result = sqlx::query(
        r#"
        UPDATE runtime_environments
        SET name = ?, backend = ?, deploy_type = ?, version = ?, base_url = ?,
            health_url = ?, endpoint_url = ?, binary_path = ?, docker_image = ?, working_dir = ?,
            log_dir = ?, allowed_model_dirs_json = ?, params_json = ?, enabled = ?,
            updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(request.name)
    .bind(request.backend)
    .bind(request.deploy_type)
    .bind(request.version)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(request.binary_path)
    .bind(request.docker_image)
    .bind(request.working_dir)
    .bind(request.log_dir)
    .bind(request.allowed_model_dirs_json)
    .bind(request.params_json)
    .bind(bool_to_int(request.enabled.unwrap_or(true)))
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DomainError::NotFound(
            "runtime environment not found".to_string(),
        ));
    }

    if is_ollama {
        let checked = ollama_runtime_config_check();
        update_runtime_environment_check(pool, id, &checked.check_status, &checked.message).await?;
    }

    runtime_environment(pool, id).await
}

pub async fn delete_runtime_environment(pool: &SqlitePool, id: &str) -> Result<(), DomainError> {
    let instance_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM model_instances WHERE runtime_environment_id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?;
    if instance_count > 0 {
        return Err(DomainError::Conflict(
            "runtime environment is used by model instances".to_string(),
        ));
    }

    let result = sqlx::query("DELETE FROM runtime_environments WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DomainError::NotFound(
            "runtime environment not found".to_string(),
        ));
    }
    Ok(())
}

pub async fn check_runtime_environment(
    pool: &SqlitePool,
    id: &str,
) -> Result<RuntimeEnvironmentView, DomainError> {
    let environment = runtime_environment(pool, id).await?;
    if environment.backend == "ollama" {
        let checked = ollama_runtime_config_check();
        update_runtime_environment_check(pool, id, &checked.check_status, &checked.message).await?;
        return runtime_environment(pool, id).await;
    }
    let node_id = environment
        .node_id
        .as_deref()
        .ok_or_else(|| DomainError::BadRequest("Runtime must be bound to a node".to_string()))?;
    if !node_online(pool, node_id).await? {
        update_runtime_environment_check(
            pool,
            id,
            "agent_offline",
            "Node Agent offline, cannot check runtime environment",
        )
        .await?;
        return Err(DomainError::Conflict(
            "Node Agent offline, cannot check runtime environment".to_string(),
        ));
    }
    let request = RuntimeEnvironmentRequest {
        name: environment.name.clone(),
        backend: environment.backend.clone(),
        deploy_type: environment.deploy_type.clone(),
        version: environment.version.clone(),
        base_url: None,
        health_url: None,
        endpoint_url: None,
        binary_path: environment.binary_path.clone(),
        docker_image: environment.docker_image.clone(),
        working_dir: environment.working_dir.clone(),
        log_dir: environment.log_dir.clone(),
        allowed_model_dirs_json: environment.allowed_model_dirs_json.clone(),
        params_json: environment.params_json.clone(),
        enabled: Some(environment.enabled),
    };
    let checked = check_runtime_environment_before_save(pool, node_id, &request).await?;
    let now = now_unix_secs();
    sqlx::query(
        "UPDATE runtime_environments SET version = COALESCE(?, version), last_checked_at = ?, check_status = ?, check_message = ?, updated_at = ? WHERE id = ?",
    )
    .bind(checked.version)
    .bind(checked.checked_at)
    .bind(checked.check_status)
    .bind(checked.message)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    runtime_environment(pool, id).await
}

pub(super) async fn update_runtime_environment_check(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    message: &str,
) -> Result<RuntimeEnvironmentView, DomainError> {
    let now = now_unix_secs();
    sqlx::query(
        r#"
        UPDATE runtime_environments
        SET last_checked_at = ?, check_status = ?, check_message = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(now)
    .bind(status)
    .bind(message)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    runtime_environment(pool, id).await
}

struct CheckedRuntimeEnvironment {
    check_status: String,
    version: Option<String>,
    message: String,
    checked_at: i64,
}

fn ollama_runtime_config_check() -> CheckedRuntimeEnvironment {
    CheckedRuntimeEnvironment {
        check_status: "available".to_string(),
        version: None,
        message: "Ollama runtime config saved; daemon availability is checked when listing models or starting instances".to_string(),
        checked_at: now_unix_secs(),
    }
}

async fn check_runtime_environment_before_save(
    pool: &SqlitePool,
    node_id: &str,
    request: &RuntimeEnvironmentRequest,
) -> Result<CheckedRuntimeEnvironment, DomainError> {
    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let payload = serde_json::json!({
        "name": request.name,
        "backend": request.backend,
        "deploy_type": request.deploy_type,
        "version": request.version,
        "binary_path": request.binary_path,
        "docker_image": request.docker_image,
        "working_dir": request.working_dir,
        "params_json": request.params_json,
    });
    sqlx::query(
        r#"
        INSERT INTO agent_tasks (
            id, node_id, kind, status, payload_json, attempt_count, created_at, updated_at
        )
        VALUES (?, ?, 'check_runtime_environment', 'queued', ?, 0, ?, ?)
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

    let deadline = Instant::now() + Duration::from_secs(RUNTIME_ENVIRONMENT_CHECK_TIMEOUT_SECS);
    loop {
        let row =
            sqlx::query("SELECT status, result_json, error_message FROM agent_tasks WHERE id = ?")
                .bind(&task_id)
                .fetch_one(pool)
                .await?;
        let status: String = row.get("status");
        match status.as_str() {
            "succeeded" => {
                let result = row
                    .get::<Option<String>, _>("result_json")
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                    .unwrap_or_else(|| serde_json::json!({}));
                let check_status = result
                    .get("check_status")
                    .and_then(|value| value.as_str())
                    .unwrap_or("available");
                if !matches!(check_status, "available" | "version_unavailable") {
                    return Err(DomainError::BadRequest(runtime_check_message(&result)));
                }
                return Ok(CheckedRuntimeEnvironment {
                    check_status: check_status.to_string(),
                    version: result
                        .get("version")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    message: runtime_check_message(&result),
                    checked_at: now_unix_secs(),
                });
            }
            "failed" => {
                let result = row
                    .get::<Option<String>, _>("result_json")
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                    .unwrap_or_else(|| serde_json::json!({}));
                return Err(DomainError::BadRequest(runtime_check_message(&result)));
            }
            "timed_out" => {
                return Err(DomainError::Conflict(
                    "Runtime check timed out; confirm Agent is online and retry".to_string(),
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            agent_tasks::mark_task_timed_out(pool, &task_id).await?;
            return Err(DomainError::Conflict(
                "Runtime check timed out; confirm Agent is online and retry".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn runtime_check_message(result: &serde_json::Value) -> String {
    result
        .get("message")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Runtime check failed")
        .to_string()
}

pub(super) fn runtime_environment_from_row(row: sqlx::sqlite::SqliteRow) -> RuntimeEnvironmentView {
    RuntimeEnvironmentView {
        id: row.get("id"),
        node_id: row.get("node_id"),
        name: row.get("name"),
        backend: row.get("backend"),
        deploy_type: row.get("deploy_type"),
        version: row.get("version"),
        base_url: row.get("base_url"),
        health_url: row.get("health_url"),
        endpoint_url: row.get("endpoint_url"),
        binary_path: row.get("binary_path"),
        docker_image: row.get("docker_image"),
        working_dir: row.get("working_dir"),
        log_dir: row.get("log_dir"),
        allowed_model_dirs_json: row.get("allowed_model_dirs_json"),
        params_json: row.get("params_json"),
        enabled: int_to_bool(row.get("enabled")),
        last_checked_at: row.get("last_checked_at"),
        check_status: row.get("check_status"),
        check_message: row.get("check_message"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

pub(super) fn runtime_environment_usable(status: Option<&str>) -> bool {
    matches!(status, Some("available" | "version_unavailable"))
}
