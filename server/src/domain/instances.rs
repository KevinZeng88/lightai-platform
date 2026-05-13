use sqlx::{Row, SqlitePool};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

use super::{
    node_online, now_unix_secs, runtime_environment, validate_backend, validate_json_field,
    validate_non_empty, DomainError, MODEL_INSTANCE_TASK_TIMEOUT_SECS,
};
use crate::agent_tasks;
use crate::http_check;
use crate::models::{
    ModelInstanceCreateRequest, ModelInstanceListResponse, ModelInstanceUpdateRequest,
    ModelInstanceView,
};

struct InstanceModelFile {
    model_id: String,
    node_id: String,
    path: String,
    path_type: Option<String>,
}

pub async fn create_model_instance(
    pool: &SqlitePool,
    request: ModelInstanceCreateRequest,
) -> Result<ModelInstanceView, DomainError> {
    validate_non_empty("name", &request.name)?;
    validate_instance_status(request.status.as_deref().unwrap_or("unknown"))?;
    let deploy_type = request.deploy_type.as_deref().unwrap_or("external");
    validate_instance_deploy_type(deploy_type)?;
    if deploy_type == "local" {
        return create_local_model_instance(pool, request).await;
    }
    let backend = request
        .backend
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("custom");
    validate_backend(backend)?;
    validate_base_url_required(&request.base_url)?;
    validate_optional_non_empty("model_name", request.model_name.as_deref())?;
    validate_instance_urls(
        &request.base_url,
        &request.endpoint_url,
        &request.health_url,
    )?;
    validate_json_field("params_json", request.params_json.as_deref())?;

    if let Some(model_id) = request.model_id.as_deref() {
        let model_exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM models WHERE id = ? AND deleted_at IS NULL")
                .bind(model_id)
                .fetch_optional(pool)
                .await?;
        if model_exists.is_none() {
            return Err(DomainError::BadRequest("model not found".to_string()));
        }
    }

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO model_instances (
            id, model_id, model_file_id, node_id, runtime_environment_id, name, backend,
            deploy_type, status, base_url, endpoint_url, health_url, runtime_version,
            model_name, description, params_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(request.model_id)
    .bind(None::<String>)
    .bind(request.node_id)
    .bind(request.runtime_environment_id)
    .bind(request.name)
    .bind(backend)
    .bind(deploy_type)
    .bind(request.status.unwrap_or_else(|| "unknown".to_string()))
    .bind(request.base_url)
    .bind(request.endpoint_url)
    .bind(request.health_url)
    .bind(request.runtime_version)
    .bind(request.model_name)
    .bind(request.description)
    .bind(request.params_json)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    model_instance(pool, &id).await
}

async fn create_local_model_instance(
    pool: &SqlitePool,
    request: ModelInstanceCreateRequest,
) -> Result<ModelInstanceView, DomainError> {
    validate_json_field("params_json", request.params_json.as_deref())?;
    let params_json = request.params_json;
    let params = parse_instance_params(params_json.as_deref())?;
    let node_id = request
        .node_id
        .as_deref()
        .ok_or_else(|| DomainError::BadRequest("Local instance requires a node".to_string()))?;
    let runtime_environment_id = request.runtime_environment_id.as_deref().ok_or_else(|| {
        DomainError::BadRequest("Local instance requires a runtime environment".to_string())
    })?;
    let env = runtime_environment(pool, runtime_environment_id).await?;
    if env.node_id.as_deref() != Some(node_id) {
        return Err(DomainError::BadRequest(
            "Runtime environment does not belong to the selected node".to_string(),
        ));
    }
    if !super::runtimes::runtime_environment_usable(env.check_status.as_deref()) {
        return Err(DomainError::BadRequest(
            "Runtime environment has not passed Agent check".to_string(),
        ));
    }

    // Ollama: model is identified by name, not a model file on disk.
    let is_ollama = env.backend == "ollama";
    let (model_id_val, model_file_id_val) = if is_ollama {
        let name = params
            .get("ollama_model")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                DomainError::BadRequest(
                    "ollama_model is required for Ollama backend instances".to_string(),
                )
            })?;
        if name.len() > 256 || name.chars().any(|c| c.is_control()) {
            return Err(DomainError::BadRequest(
                "ollama_model contains invalid characters".to_string(),
            ));
        }
        (None::<String>, None::<String>)
    } else {
        let model_file_id = request.model_file_id.as_deref().ok_or_else(|| {
            DomainError::BadRequest("Local instance requires a verified model file".to_string())
        })?;
        let file = verified_model_file_for_instance(pool, model_file_id).await?;
        if file.node_id != node_id {
            return Err(DomainError::BadRequest(
                "Model file does not belong to the selected node".to_string(),
            ));
        }
        (Some(file.model_id), Some(model_file_id.to_string()))
    };

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO model_instances (
            id, model_id, model_file_id, node_id, runtime_environment_id, name, backend,
            deploy_type, status, runtime_version, model_name, description, params_json,
            created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, 'local', 'stopped', ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(model_id_val)
    .bind(model_file_id_val)
    .bind(node_id)
    .bind(runtime_environment_id)
    .bind(request.name)
    .bind(env.backend)
    .bind(env.version)
    .bind(request.model_name)
    .bind(request.description)
    .bind(params_json)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    model_instance(pool, &id).await
}

pub async fn list_model_instances(
    pool: &SqlitePool,
) -> Result<ModelInstanceListResponse, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT mi.*, m.name AS model_definition_name, mf.path AS model_file_path,
               n.name AS node_name, n.last_heartbeat_at AS node_last_heartbeat_at,
               re.name AS runtime_environment_name
        FROM model_instances mi
        LEFT JOIN models m ON m.id = mi.model_id
        LEFT JOIN model_files mf ON mf.id = mi.model_file_id
        LEFT JOIN nodes n ON n.id = mi.node_id
        LEFT JOIN runtime_environments re ON re.id = mi.runtime_environment_id
        ORDER BY mi.created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(ModelInstanceListResponse {
        model_instances: rows.into_iter().map(model_instance_from_row).collect(),
    })
}

pub async fn model_instance(pool: &SqlitePool, id: &str) -> Result<ModelInstanceView, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT mi.*, m.name AS model_definition_name, mf.path AS model_file_path,
               n.name AS node_name, n.last_heartbeat_at AS node_last_heartbeat_at,
               re.name AS runtime_environment_name
        FROM model_instances mi
        LEFT JOIN models m ON m.id = mi.model_id
        LEFT JOIN model_files mf ON mf.id = mi.model_file_id
        LEFT JOIN nodes n ON n.id = mi.node_id
        LEFT JOIN runtime_environments re ON re.id = mi.runtime_environment_id
        WHERE mi.id = ?
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DomainError::NotFound("model instance not found".to_string()))?;
    Ok(model_instance_from_row(row))
}

pub async fn update_model_instance(
    pool: &SqlitePool,
    id: &str,
    request: ModelInstanceUpdateRequest,
) -> Result<ModelInstanceView, DomainError> {
    let current = model_instance(pool, id).await?;
    if matches!(current.status.as_str(), "running" | "starting" | "stopping") {
        let is_config_change = request.name.is_some()
            || request.params_json.is_some()
            || request.backend.is_some()
            || request.base_url.is_some()
            || request.endpoint_url.is_some()
            || request.health_url.is_some()
            || request.runtime_version.is_some()
            || request.model_name.is_some();
        if is_config_change {
            return Err(DomainError::Conflict(
                "Cannot modify a running instance. Stop it first.".to_string(),
            ));
        }
    }
    if current.deploy_type == "local" {
        let name = request.name.unwrap_or(current.name);
        validate_non_empty("name", &name)?;
        let params_json = request.params_json.or(current.params_json);
        validate_json_field("params_json", params_json.as_deref())?;
        let now = now_unix_secs();
        sqlx::query(
            "UPDATE model_instances SET name = ?, description = ?, params_json = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(request.description.or(current.description))
        .bind(params_json)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
        return model_instance(pool, id).await;
    }
    let name = request.name.unwrap_or(current.name);
    validate_non_empty("name", &name)?;
    let status = request.status.unwrap_or(current.status);
    validate_instance_status(&status)?;
    if let Some(backend) = request.backend.as_deref() {
        if !backend.trim().is_empty() {
            validate_backend(backend)?;
        }
    }
    validate_base_url_required(&request.base_url.clone().or(current.base_url.clone()))?;
    validate_optional_non_empty(
        "model_name",
        request
            .model_name
            .as_deref()
            .or(current.model_name.as_deref()),
    )?;
    validate_instance_urls(
        &request.base_url,
        &request.endpoint_url,
        &request.health_url,
    )?;
    let params_json = request.params_json.or(current.params_json);
    validate_json_field("params_json", params_json.as_deref())?;

    let now = now_unix_secs();
    sqlx::query(
        r#"
        UPDATE model_instances
        SET name = ?, backend = ?, status = ?, base_url = ?, endpoint_url = ?,
            health_url = ?, runtime_version = ?, model_name = ?, description = ?,
            params_json = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(name)
    .bind(
        request
            .backend
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(current.backend),
    )
    .bind(status)
    .bind(request.base_url.or(current.base_url))
    .bind(request.endpoint_url.or(current.endpoint_url))
    .bind(request.health_url.or(current.health_url))
    .bind(request.runtime_version.or(current.runtime_version))
    .bind(request.model_name.or(current.model_name))
    .bind(request.description.or(current.description))
    .bind(params_json)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    model_instance(pool, id).await
}

pub async fn delete_model_instance(pool: &SqlitePool, id: &str) -> Result<(), DomainError> {
    let instance = model_instance(pool, id).await?;
    if matches!(
        instance.status.as_str(),
        "running" | "starting" | "stopping"
    ) {
        return Err(DomainError::Conflict(
            "Cannot delete a running instance. Stop it first.".to_string(),
        ));
    }
    let result = sqlx::query("DELETE FROM model_instances WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DomainError::NotFound(
            "model instance not found".to_string(),
        ));
    }
    Ok(())
}

pub async fn check_model_instance(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelInstanceView, DomainError> {
    let instance = model_instance(pool, id).await?;
    if instance.deploy_type != "external" {
        if instance.status != "running" && instance.backend != "ollama" {
            return Ok(instance);
        }
        let node_id = instance
            .node_id
            .as_deref()
            .ok_or_else(|| DomainError::BadRequest("Local instance missing node".to_string()))?;
        if !node_online(pool, node_id).await? {
            let _ = crate::platform_log::append(
                &crate::platform_log::global(),
                "lightai-server.log",
                "warn",
                &format!(
                    "check_instance: instance={id} node={node_id} agent offline, cannot check status"
                ),
            )
            .await;
            return update_instance_check(
                pool,
                id,
                instance.status.as_str(),
                Some("Agent offline, cannot check instance status"),
            )
            .await;
        }
        return run_local_instance_task(pool, id, "check_model_instance", "running").await;
    }
    let Some(url) = instance
        .health_url
        .as_deref()
        .or(instance.endpoint_url.as_deref())
        .or(instance.base_url.as_deref())
    else {
        return update_instance_check(pool, id, "unknown", Some("no check URL configured")).await;
    };
    let result = http_check::check_url(url).await;
    let error = if result.status == "running" {
        None
    } else {
        Some(result.message.as_str())
    };
    update_instance_check(pool, id, &result.status, error).await
}

pub async fn start_model_instance(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelInstanceView, DomainError> {
    run_local_instance_task(pool, id, "start_model_instance", "starting").await
}

pub async fn stop_model_instance(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelInstanceView, DomainError> {
    run_local_instance_task(pool, id, "stop_model_instance", "stopping").await
}

pub async fn test_model_instance(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelInstanceView, DomainError> {
    let instance = model_instance(pool, id).await?;
    if instance.deploy_type == "external" {
        return check_model_instance(pool, id).await;
    }
    if instance.status != "running" && instance.backend != "ollama" {
        return Err(DomainError::BadRequest(
            "Local instance is not running, cannot test".to_string(),
        ));
    }
    run_local_instance_task(pool, id, "test_model_instance", "running").await
}

async fn run_local_instance_task(
    pool: &SqlitePool,
    id: &str,
    task_kind: &str,
    pending_status: &str,
) -> Result<ModelInstanceView, DomainError> {
    let instance = model_instance(pool, id).await?;
    if instance.deploy_type != "local" {
        return Err(DomainError::BadRequest(
            "External instances are not started or stopped by the platform".to_string(),
        ));
    }
    let node_id = instance
        .node_id
        .as_deref()
        .ok_or_else(|| DomainError::BadRequest("Local instance missing node".to_string()))?;
    if !node_online(pool, node_id).await? {
        return Err(DomainError::Conflict(
            "Node Agent offline, cannot execute local instance task".to_string(),
        ));
    }
    let runtime_environment_id = instance.runtime_environment_id.as_deref().ok_or_else(|| {
        DomainError::BadRequest("Local instance missing runtime environment".to_string())
    })?;
    let env = runtime_environment(pool, runtime_environment_id).await?;
    if !super::runtimes::runtime_environment_usable(env.check_status.as_deref()) {
        return Err(DomainError::BadRequest(
            "Runtime environment has not passed Agent check".to_string(),
        ));
    }
    let params = parse_instance_params(instance.params_json.as_deref())?;

    let runtime_params: Option<serde_json::Value> = env
        .params_json
        .as_deref()
        .and_then(|v| serde_json::from_str(v).ok());

    let is_ollama = env.backend == "ollama";
    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();

    let mut payload = if is_ollama {
        serde_json::json!({
            "instance_id": id,
            "runtime_environment_id": runtime_environment_id,
            "backend": env.backend,
            "deploy_type": env.deploy_type,
            "binary_path": env.binary_path,
            "working_dir": env.working_dir,
            "log_dir": env.log_dir,
            "base_url": instance.base_url,
            "endpoint_url": instance.endpoint_url,
            "params_json": serde_json::to_string(&params).unwrap_or_default(),
        })
    } else {
        let model_file_id = instance.model_file_id.as_deref().ok_or_else(|| {
            DomainError::BadRequest("Local instance missing model file".to_string())
        })?;
        let file = verified_model_file_for_instance(pool, model_file_id).await?;
        serde_json::json!({
            "instance_id": id,
            "runtime_environment_id": runtime_environment_id,
            "backend": env.backend,
            "deploy_type": env.deploy_type,
            "binary_path": env.binary_path,
            "docker_image": env.docker_image,
            "working_dir": env.working_dir,
            "log_dir": env.log_dir,
            "model_file_id": model_file_id,
            "model_path": file.path,
            "model_path_type": file.path_type,
            "base_url": instance.base_url,
            "endpoint_url": instance.endpoint_url,
            "params_json": serde_json::to_string(&params).unwrap_or_default(),
        })
    };
    if let Some(rp) = runtime_params {
        payload["runtime_params"] = rp;
    }
    if let Some(ref model_name) = instance.model_definition_name {
        payload["model_name"] = serde_json::Value::String(model_name.clone());
    }
    let mut tx = pool.begin().await?;
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
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE model_instances SET status = ?, last_error = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(pending_status)
    .bind(now)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    agent_tasks::notify_agent_tasks();
    wait_for_model_instance_task(pool, id, &task_id).await
}

async fn wait_for_model_instance_task(
    pool: &SqlitePool,
    instance_id: &str,
    task_id: &str,
) -> Result<ModelInstanceView, DomainError> {
    let deadline = Instant::now() + Duration::from_secs(MODEL_INSTANCE_TASK_TIMEOUT_SECS);
    loop {
        let row =
            sqlx::query("SELECT status, result_json, error_message FROM agent_tasks WHERE id = ?")
                .bind(task_id)
                .fetch_one(pool)
                .await?;
        let status: String = row.get("status");
        match status.as_str() {
            "succeeded" => return model_instance(pool, instance_id).await,
            "failed" => {
                let result_json: Option<String> = row.get("result_json");
                let message = result_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                    .and_then(|value| {
                        value
                            .get("message")
                            .and_then(|m| m.as_str())
                            .map(str::to_string)
                    })
                    .or_else(|| row.get::<Option<String>, _>("error_message"))
                    .unwrap_or_else(|| "Local instance task failed".to_string());
                return Err(DomainError::Conflict(message));
            }
            "timed_out" => {
                return Err(DomainError::Conflict(
                    "Local instance task timed out; confirm Agent is online and retry".to_string(),
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            agent_tasks::mark_task_timed_out(pool, task_id).await?;
            update_instance_check(
                pool,
                instance_id,
                "failed",
                Some("local instance task timed out"),
            )
            .await?;
            return Err(DomainError::Conflict(
                "Local instance task timed out; confirm Agent is online and retry".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn update_instance_check(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<ModelInstanceView, DomainError> {
    let now = now_unix_secs();
    sqlx::query(
        r#"
        UPDATE model_instances
        SET status = ?, last_checked_at = ?, last_error = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(status)
    .bind(now)
    .bind(error)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    model_instance(pool, id).await
}

fn model_instance_from_row(row: sqlx::sqlite::SqliteRow) -> ModelInstanceView {
    let last_heartbeat_at: Option<i64> = row.get("node_last_heartbeat_at");
    let node_online = match last_heartbeat_at {
        Some(last_seen) => now_unix_secs() - last_seen <= crate::repository::ONLINE_THRESHOLD_SECS,
        None => false,
    };
    ModelInstanceView {
        id: row.get("id"),
        model_id: row.get("model_id"),
        model_file_id: row.get("model_file_id"),
        model_definition_name: row.get("model_definition_name"),
        model_file_path: row.get("model_file_path"),
        node_id: row.get("node_id"),
        node_name: row.get("node_name"),
        node_online,
        last_heartbeat_at,
        runtime_environment_id: row.get("runtime_environment_id"),
        runtime_environment_name: row.get("runtime_environment_name"),
        name: row.get("name"),
        backend: row.get("backend"),
        deploy_type: row.get("deploy_type"),
        status: row.get("status"),
        base_url: row.get("base_url"),
        endpoint_url: row.get("endpoint_url"),
        health_url: row.get("health_url"),
        runtime_version: row.get("runtime_version"),
        model_name: row.get("model_name"),
        description: row.get("description"),
        params_json: row.get("params_json"),
        process_id: row.get("process_id"),
        process_ref: row.get("process_ref"),
        log_tail: row.get("log_tail"),
        command: row.get("command"),
        last_checked_at: row.get("last_checked_at"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn validate_optional_non_empty(field: &str, value: Option<&str>) -> Result<(), DomainError> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(()),
        _ => Err(DomainError::BadRequest(format!("{field} is required"))),
    }
}

fn validate_instance_deploy_type(value: &str) -> Result<(), DomainError> {
    super::validate_one_of("deploy_type", value, &["external", "local"])
}

fn validate_instance_status(value: &str) -> Result<(), DomainError> {
    super::validate_one_of(
        "status",
        value,
        &[
            "pending", "starting", "running", "stopping", "stopped", "failed", "unknown",
        ],
    )
}

fn validate_instance_urls(
    base_url: &Option<String>,
    endpoint_url: &Option<String>,
    health_url: &Option<String>,
) -> Result<(), DomainError> {
    for (field, value) in [
        ("base_url", base_url.as_deref()),
        ("endpoint_url", endpoint_url.as_deref()),
        ("health_url", health_url.as_deref()),
    ] {
        if let Some(value) = value {
            validate_http_url(field, value)?;
        }
    }
    Ok(())
}

fn validate_base_url_required(base_url: &Option<String>) -> Result<(), DomainError> {
    match base_url.as_deref() {
        Some(value) if !value.trim().is_empty() => Ok(()),
        _ => Err(DomainError::BadRequest("base_url is required".to_string())),
    }
}

fn validate_http_url(field: &str, value: &str) -> Result<(), DomainError> {
    let parsed = reqwest::Url::parse(value).map_err(|_| {
        DomainError::BadRequest(format!("{field} must be a valid http:// or https:// URL"))
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        _ => Err(DomainError::BadRequest(format!(
            "{field} must use http:// or https://"
        ))),
    }
}

fn parse_instance_params(value: Option<&str>) -> Result<serde_json::Value, DomainError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(serde_json::json!({}));
    };
    let parsed = serde_json::from_str::<serde_json::Value>(value)
        .map_err(|_| DomainError::BadRequest("params_json must be valid JSON".to_string()))?;
    if !parsed.is_object() {
        return Err(DomainError::BadRequest(
            "runtime params must be a JSON object".to_string(),
        ));
    }
    if let Some(host) = parsed.get("host").and_then(|value| value.as_str()) {
        if host.trim().is_empty() || host.len() > 128 || host.chars().any(char::is_control) {
            return Err(DomainError::BadRequest(
                "invalid listen address".to_string(),
            ));
        }
    }
    if let Some(port) = parsed.get("port").and_then(|value| value.as_u64()) {
        if port == 0 || port > u16::MAX as u64 {
            return Err(DomainError::BadRequest("invalid listen port".to_string()));
        }
    }
    if let Some(extra_args) = parsed.get("extra_args").and_then(|value| value.as_array()) {
        for arg in extra_args {
            let Some(arg) = arg.as_str() else {
                return Err(DomainError::BadRequest(
                    "extra args must be one string per line".to_string(),
                ));
            };
            if arg.trim().is_empty() || arg.len() > 256 || arg.chars().any(char::is_control) {
                return Err(DomainError::BadRequest("invalid extra args".to_string()));
            }
        }
    }
    Ok(parsed)
}

async fn verified_model_file_for_instance(
    pool: &SqlitePool,
    model_file_id: &str,
) -> Result<InstanceModelFile, DomainError> {
    let row = sqlx::query(
        r#"
        SELECT model_id, node_id, path, path_type, status
        FROM model_files
        WHERE id = ? AND deleted_at IS NULL
        "#,
    )
    .bind(model_file_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DomainError::BadRequest("model file does not exist".to_string()))?;
    let status: String = row.get("status");
    if status != "verified" {
        return Err(DomainError::BadRequest(
            "local instance requires a verified model file".to_string(),
        ));
    }
    Ok(InstanceModelFile {
        model_id: row.get("model_id"),
        node_id: row.get("node_id"),
        path: row.get("path"),
        path_type: row.get("path_type"),
    })
}
