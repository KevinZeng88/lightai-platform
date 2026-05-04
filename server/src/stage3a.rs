use sqlx::{Row, SqlitePool};
use std::sync::{Arc, OnceLock};

use tokio::sync::Notify;
use tokio::time::{sleep, timeout, Duration, Instant};
use uuid::Uuid;

use crate::http_check;
use crate::models::{
    AgentTaskPollResponse, AgentTaskResultRequest, AgentTaskView, ModelFileListResponse,
    ModelFileRequest, ModelFileTrashListResponse, ModelFileTrashRequest, ModelFileTrashView,
    ModelFileView, ModelInstanceCreateRequest, ModelInstanceListResponse,
    ModelInstanceUpdateRequest, ModelInstanceView, ModelListResponse, ModelRequest, ModelView,
    RuntimeEnvironmentListResponse, RuntimeEnvironmentRequest, RuntimeEnvironmentView,
};
use crate::repository;

const AGENT_TASK_LEASE_SECS: i64 = 30;
const AGENT_TASK_QUEUE_TIMEOUT_SECS: i64 = 300;
const AGENT_TASK_LONG_POLL_SECS: u64 = 25;
const MODEL_FILE_VERIFY_TIMEOUT_SECS: u64 = 5;
const MODEL_FILE_CLEANUP_TIMEOUT_SECS: u64 = 10;
const RUNTIME_ENVIRONMENT_CHECK_TIMEOUT_SECS: u64 = 5;
const MODEL_INSTANCE_TASK_TIMEOUT_SECS: u64 = 30;
const LOG_READ_TIMEOUT_SECS: u64 = 5;
const INSTANCE_LOG_TIMEOUT_SECS: u64 = 5;
static TASK_NOTIFY: OnceLock<Arc<Notify>> = OnceLock::new();

pub fn task_notify() -> Arc<Notify> {
    TASK_NOTIFY.get_or_init(|| Arc::new(Notify::new())).clone()
}

pub fn notify_agent_tasks() {
    task_notify().notify_waiters();
}

#[derive(Debug)]
pub enum Stage3Error {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    Internal(anyhow::Error),
}

struct VerifiedModelFile {
    size_bytes: Option<i64>,
    path_type: Option<String>,
    verified_at: i64,
}

struct InstanceModelFile {
    model_id: String,
    node_id: String,
    path: String,
    path_type: Option<String>,
}

impl From<anyhow::Error> for Stage3Error {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

impl From<sqlx::Error> for Stage3Error {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.into())
    }
}

pub async fn create_runtime_environment(
    pool: &SqlitePool,
    node_id: &str,
    request: RuntimeEnvironmentRequest,
) -> Result<RuntimeEnvironmentView, Stage3Error> {
    validate_non_empty("name", &request.name)?;
    validate_backend(&request.backend)?;
    validate_deploy_type(&request.deploy_type)?;
    validate_runtime_entrypoints(&request)?;
    validate_json_field(
        "allowed_model_dirs_json",
        request.allowed_model_dirs_json.as_deref(),
    )?;
    validate_json_field("config_json", request.config_json.as_deref())?;
    ensure_node_online(pool, node_id).await?;
    let checked = check_runtime_environment_before_save(pool, node_id, &request).await?;

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO runtime_environments (
            id, node_id, name, backend, deploy_type, version, base_url, health_url, endpoint_url,
            binary_path, docker_image, working_dir, log_dir, allowed_model_dirs_json,
            config_json, enabled, last_checked_at, check_status, check_message, created_at, updated_at
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
    .bind(request.config_json)
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
) -> Result<RuntimeEnvironmentListResponse, Stage3Error> {
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
) -> Result<RuntimeEnvironmentView, Stage3Error> {
    let row = sqlx::query("SELECT * FROM runtime_environments WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Stage3Error::NotFound("runtime environment not found".to_string()))?;
    Ok(runtime_environment_from_row(row))
}

pub async fn update_runtime_environment(
    pool: &SqlitePool,
    id: &str,
    request: RuntimeEnvironmentRequest,
) -> Result<RuntimeEnvironmentView, Stage3Error> {
    validate_non_empty("name", &request.name)?;
    validate_backend(&request.backend)?;
    validate_deploy_type(&request.deploy_type)?;
    validate_runtime_entrypoints(&request)?;
    validate_json_field(
        "allowed_model_dirs_json",
        request.allowed_model_dirs_json.as_deref(),
    )?;
    validate_json_field("config_json", request.config_json.as_deref())?;

    let now = now_unix_secs();
    let result = sqlx::query(
        r#"
        UPDATE runtime_environments
        SET name = ?, backend = ?, deploy_type = ?, version = ?, base_url = ?,
            health_url = ?, endpoint_url = ?, binary_path = ?, docker_image = ?, working_dir = ?,
            log_dir = ?, allowed_model_dirs_json = ?, config_json = ?, enabled = ?,
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
    .bind(request.config_json)
    .bind(bool_to_int(request.enabled.unwrap_or(true)))
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(Stage3Error::NotFound(
            "runtime environment not found".to_string(),
        ));
    }

    runtime_environment(pool, id).await
}

pub async fn delete_runtime_environment(pool: &SqlitePool, id: &str) -> Result<(), Stage3Error> {
    let instance_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM model_instances WHERE runtime_environment_id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?;
    if instance_count > 0 {
        return Err(Stage3Error::Conflict(
            "runtime environment is used by model instances".to_string(),
        ));
    }

    let result = sqlx::query("DELETE FROM runtime_environments WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(Stage3Error::NotFound(
            "runtime environment not found".to_string(),
        ));
    }
    Ok(())
}

pub async fn check_runtime_environment(
    pool: &SqlitePool,
    id: &str,
) -> Result<RuntimeEnvironmentView, Stage3Error> {
    let environment = runtime_environment(pool, id).await?;
    let node_id = environment
        .node_id
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("运行环境必须绑定节点".to_string()))?;
    if !node_online(pool, node_id).await? {
        update_runtime_environment_check(
            pool,
            id,
            "agent_offline",
            "节点 Agent 离线，无法检查运行环境",
        )
        .await?;
        return Err(Stage3Error::Conflict(
            "节点 Agent 离线，无法检查运行环境".to_string(),
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
        config_json: environment.config_json.clone(),
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

async fn update_runtime_environment_check(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    message: &str,
) -> Result<RuntimeEnvironmentView, Stage3Error> {
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

async fn check_runtime_environment_before_save(
    pool: &SqlitePool,
    node_id: &str,
    request: &RuntimeEnvironmentRequest,
) -> Result<CheckedRuntimeEnvironment, Stage3Error> {
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
        "config_json": request.config_json,
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
    notify_agent_tasks();

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
                    return Err(Stage3Error::BadRequest(runtime_check_message(&result)));
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
                return Err(Stage3Error::BadRequest(runtime_check_message(&result)));
            }
            "timed_out" => {
                return Err(Stage3Error::Conflict(
                    "运行环境检查超时，请确认 Agent 在线并重试".to_string(),
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            mark_task_timed_out(pool, &task_id).await?;
            return Err(Stage3Error::Conflict(
                "运行环境检查超时，请确认 Agent 在线并重试".to_string(),
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
        .unwrap_or("运行环境检查失败")
        .to_string()
}

pub async fn create_model(
    pool: &SqlitePool,
    request: ModelRequest,
) -> Result<ModelView, Stage3Error> {
    validate_non_empty("name", &request.name)?;
    validate_model_type(&request.model_type)?;
    if let Some(default_backend) = request.default_backend.as_deref() {
        validate_backend(default_backend)?;
    }
    if let Some(model_path) = request.model_path.as_deref() {
        validate_path("model_path", model_path)?;
    }
    validate_json_field("config_json", request.config_json.as_deref())?;
    let initial_file = request
        .initial_file
        .ok_or_else(|| Stage3Error::BadRequest("initial_file is required".to_string()))?;
    validate_non_empty("initial_file.path", &initial_file.path)?;
    validate_path("initial_file.path", &initial_file.path)?;
    ensure_node_exists(pool, &initial_file.node_id).await?;
    let verified_file =
        verify_model_file_before_save(pool, &initial_file.node_id, &initial_file.path).await?;

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    let file_id = Uuid::new_v4().to_string();
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE models
        SET name = name || '__deleted__' || substr(id, 1, 8), updated_at = ?
        WHERE name = ? AND deleted_at IS NOT NULL
        "#,
    )
    .bind(now)
    .bind(&request.name)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO models (
            id, name, display_name, model_type, model_path, description,
            default_backend, config_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(request.name)
    .bind(request.display_name)
    .bind(request.model_type)
    .bind(request.model_path)
    .bind(request.description)
    .bind(request.default_backend)
    .bind(request.config_json)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx_conflict)?;
    sqlx::query(
        r#"
        INSERT INTO model_files (
            id, model_id, node_id, path, status, size_bytes, last_verified_at,
            path_type, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, 'verified', ?, ?, ?, ?, ?)
        "#,
    )
    .bind(file_id)
    .bind(&id)
    .bind(initial_file.node_id)
    .bind(initial_file.path)
    .bind(verified_file.size_bytes)
    .bind(verified_file.verified_at)
    .bind(verified_file.path_type)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    model(pool, &id).await
}

pub async fn list_models(pool: &SqlitePool) -> Result<ModelListResponse, Stage3Error> {
    let rows = sqlx::query("SELECT * FROM models WHERE deleted_at IS NULL ORDER BY name")
        .fetch_all(pool)
        .await?;
    let mut models = Vec::with_capacity(rows.len());
    for row in rows {
        models.push(model_from_row(pool, row).await?);
    }
    Ok(ModelListResponse { models })
}

pub async fn model(pool: &SqlitePool, id: &str) -> Result<ModelView, Stage3Error> {
    let row = sqlx::query("SELECT * FROM models WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Stage3Error::NotFound("model not found".to_string()))?;
    model_from_row(pool, row).await
}

pub async fn update_model(
    pool: &SqlitePool,
    id: &str,
    request: ModelRequest,
) -> Result<ModelView, Stage3Error> {
    validate_non_empty("name", &request.name)?;
    validate_model_type(&request.model_type)?;
    if let Some(default_backend) = request.default_backend.as_deref() {
        validate_backend(default_backend)?;
    }
    if let Some(model_path) = request.model_path.as_deref() {
        validate_path("model_path", model_path)?;
    }
    validate_json_field("config_json", request.config_json.as_deref())?;

    let now = now_unix_secs();
    let result = sqlx::query(
        r#"
        UPDATE models
        SET name = ?, display_name = ?, model_type = ?, model_path = ?,
            description = ?, default_backend = ?, config_json = ?, updated_at = ?
        WHERE id = ? AND deleted_at IS NULL
        "#,
    )
    .bind(request.name)
    .bind(request.display_name)
    .bind(request.model_type)
    .bind(request.model_path)
    .bind(request.description)
    .bind(request.default_backend)
    .bind(request.config_json)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(map_sqlx_conflict)?;

    if result.rows_affected() == 0 {
        return Err(Stage3Error::NotFound("model not found".to_string()));
    }
    model(pool, id).await
}

pub async fn delete_model(pool: &SqlitePool, id: &str) -> Result<(), Stage3Error> {
    let protected_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM model_instances
        WHERE model_id = ? AND status IN ('starting', 'running')
        "#,
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    if protected_count > 0 {
        return Err(Stage3Error::Conflict(
            "model has starting or running instances".to_string(),
        ));
    }

    let model_exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM models WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    if model_exists.is_none() {
        return Err(Stage3Error::NotFound("model not found".to_string()));
    }

    let file_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM model_files WHERE model_id = ? AND deleted_at IS NULL ORDER BY created_at",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    for file_id in file_ids {
        ensure_model_file_trash(
            pool,
            &file_id,
            Some("删除模型配置".to_string()),
            Some("模型配置删除后不再显示；真实文件未删除，可在模型垃圾箱中逐条处理。".to_string()),
        )
        .await?;
    }

    let now = now_unix_secs();
    let result = sqlx::query(
        r#"
        UPDATE models
        SET name = name || '__deleted__' || substr(id, 1, 8),
            deleted_at = ?, updated_at = ?
        WHERE id = ? AND deleted_at IS NULL
        "#,
    )
    .bind(now)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(Stage3Error::NotFound("model not found".to_string()));
    }
    Ok(())
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
    ensure_model_file_trash(
        pool,
        id,
        Some("从模型中删除节点文件路径".to_string()),
        Some("该操作只移除模型与节点文件路径的关联；真实文件未删除。".to_string()),
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
        "等待节点 Agent 执行验证"
    } else {
        "等待节点 Agent 上线后执行验证"
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
    notify_agent_tasks();
    model_file(pool, id).await
}

async fn verify_model_file_before_save(
    pool: &SqlitePool,
    node_id: &str,
    path: &str,
) -> Result<VerifiedModelFile, Stage3Error> {
    if !node_online(pool, node_id).await? {
        return Err(Stage3Error::Conflict(
            "节点 Agent 离线，无法验证模型文件".to_string(),
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
    notify_agent_tasks();

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
                    .unwrap_or_else(|| "模型文件验证失败".to_string());
                return Err(Stage3Error::BadRequest(message));
            }
            "timed_out" => {
                return Err(Stage3Error::Conflict(
                    "模型文件验证超时，请确认 Agent 在线并重试".to_string(),
                ));
            }
            _ => {}
        }

        if Instant::now() >= deadline {
            mark_task_timed_out(pool, task_id).await?;
            return Err(Stage3Error::Conflict(
                "模型文件验证超时，请确认 Agent 在线并重试".to_string(),
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
        .unwrap_or("模型文件验证失败")
        .to_string()
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
    let now = now_unix_secs();
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
    validate_one_of("status", &request.status, &["succeeded", "failed"])?;

    let now = now_unix_secs();
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
            error_message.as_deref().or(Some("文件验证失败"))
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
                "文件已清理"
            } else {
                "文件清理失败"
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
                .or(Some("实例任务执行失败".to_string()))
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
                .unwrap_or("测试成功");
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

pub async fn create_model_instance(
    pool: &SqlitePool,
    request: ModelInstanceCreateRequest,
) -> Result<ModelInstanceView, Stage3Error> {
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
            return Err(Stage3Error::BadRequest("model not found".to_string()));
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
) -> Result<ModelInstanceView, Stage3Error> {
    validate_json_field("params_json", request.params_json.as_deref())?;
    let params_json = request.params_json;
    parse_instance_params(params_json.as_deref())?;
    let node_id = request
        .node_id
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("本地实例必须选择节点".to_string()))?;
    let runtime_environment_id = request
        .runtime_environment_id
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("本地实例必须选择运行环境".to_string()))?;
    let model_file_id = request
        .model_file_id
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("本地实例必须选择已验证模型文件".to_string()))?;
    let env = runtime_environment(pool, runtime_environment_id).await?;
    if env.node_id.as_deref() != Some(node_id) {
        return Err(Stage3Error::BadRequest(
            "运行环境不属于所选节点".to_string(),
        ));
    }
    if !runtime_environment_usable(env.check_status.as_deref()) {
        return Err(Stage3Error::BadRequest(
            "运行环境未通过 Agent 检查".to_string(),
        ));
    }
    let file = verified_model_file_for_instance(pool, model_file_id).await?;
    if file.node_id != node_id {
        return Err(Stage3Error::BadRequest(
            "模型文件不属于所选节点".to_string(),
        ));
    }

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
    .bind(file.model_id)
    .bind(model_file_id)
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
) -> Result<ModelInstanceListResponse, Stage3Error> {
    let rows = sqlx::query(
        r#"
        SELECT mi.*, m.name AS model_definition_name, mf.path AS model_file_path,
               n.name AS node_name, re.name AS runtime_environment_name
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

pub async fn model_instance(pool: &SqlitePool, id: &str) -> Result<ModelInstanceView, Stage3Error> {
    let row = sqlx::query(
        r#"
        SELECT mi.*, m.name AS model_definition_name, mf.path AS model_file_path,
               n.name AS node_name, re.name AS runtime_environment_name
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
    .ok_or_else(|| Stage3Error::NotFound("model instance not found".to_string()))?;
    Ok(model_instance_from_row(row))
}

pub async fn update_model_instance(
    pool: &SqlitePool,
    id: &str,
    request: ModelInstanceUpdateRequest,
) -> Result<ModelInstanceView, Stage3Error> {
    let current = model_instance(pool, id).await?;
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

pub async fn delete_model_instance(pool: &SqlitePool, id: &str) -> Result<(), Stage3Error> {
    let result = sqlx::query("DELETE FROM model_instances WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(Stage3Error::NotFound(
            "model instance not found".to_string(),
        ));
    }
    Ok(())
}

pub async fn check_model_instance(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelInstanceView, Stage3Error> {
    let instance = model_instance(pool, id).await?;
    if instance.deploy_type != "external" {
        if instance.status != "running" {
            return Ok(instance);
        }
        let node_id = instance
            .node_id
            .as_deref()
            .ok_or_else(|| Stage3Error::BadRequest("本地实例缺少节点".to_string()))?;
        if !node_online(pool, node_id).await? {
            return update_instance_check(
                pool,
                id,
                instance.status.as_str(),
                Some("Agent 离线，无法检查实例状态"),
            )
            .await;
        }
        return run_local_instance_task(pool, id, "test_model_instance", "running").await;
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
) -> Result<ModelInstanceView, Stage3Error> {
    run_local_instance_task(pool, id, "start_model_instance", "starting").await
}

pub async fn stop_model_instance(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelInstanceView, Stage3Error> {
    run_local_instance_task(pool, id, "stop_model_instance", "stopping").await
}

pub async fn test_model_instance(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelInstanceView, Stage3Error> {
    let instance = model_instance(pool, id).await?;
    if instance.deploy_type == "external" {
        return check_model_instance(pool, id).await;
    }
    if instance.status != "running" {
        return Err(Stage3Error::BadRequest(
            "本地实例未运行，无法测试".to_string(),
        ));
    }
    run_local_instance_task(pool, id, "test_model_instance", "running").await
}

pub async fn read_agent_log(
    pool: &SqlitePool,
    node_id: &str,
    max_bytes: usize,
) -> Result<String, Stage3Error> {
    if !node_online(pool, node_id).await? {
        return Err(Stage3Error::Conflict(
            "节点 Agent 离线，无法查看 Agent 日志".to_string(),
        ));
    }
    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let payload = serde_json::json!({ "max_bytes": max_bytes.min(512 * 1024) });
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
    notify_agent_tasks();

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
                return Err(Stage3Error::Conflict(
                    row.get::<Option<String>, _>("error_message")
                        .unwrap_or_else(|| "Agent 日志读取失败".to_string()),
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            mark_task_timed_out(pool, &task_id).await?;
            return Err(Stage3Error::Conflict("Agent 日志读取超时".to_string()));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub async fn refresh_instance_logs(pool: &SqlitePool, id: &str) -> Result<String, Stage3Error> {
    let instance = model_instance(pool, id).await?;
    if instance.deploy_type != "local" {
        return Err(Stage3Error::BadRequest(
            "仅本地实例支持刷新日志".to_string(),
        ));
    }
    let node_id = instance
        .node_id
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("本地实例缺少节点".to_string()))?;
    if !node_online(pool, node_id).await? {
        return Err(Stage3Error::Conflict(
            "节点 Agent 离线，无法刷新实例日志".to_string(),
        ));
    }

    let runtime_environment_id = instance
        .runtime_environment_id
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("本地实例缺少运行环境".to_string()))?;
    let env = runtime_environment(pool, runtime_environment_id).await?;

    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let payload = serde_json::json!({
        "instance_id": id,
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
    notify_agent_tasks();

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
                return Err(Stage3Error::Conflict(
                    row.get::<Option<String>, _>("error_message")
                        .unwrap_or_else(|| "实例日志读取失败".to_string()),
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            mark_task_timed_out(pool, &task_id).await?;
            return Err(Stage3Error::Conflict("实例日志读取超时".to_string()));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub async fn frontend_error_summary(pool: &SqlitePool) -> Result<String, Stage3Error> {
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
        return Ok("暂无前端错误".to_string());
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
                "{} 前端错误：{}{}",
                ts,
                message.as_deref().unwrap_or("无详情"),
                url_info
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

pub async fn recent_error_summary(pool: &SqlitePool) -> Result<String, Stage3Error> {
    let rows = sqlx::query(
        r#"
        SELECT '实例' AS source, name AS target, last_error AS message, updated_at AS ts
        FROM model_instances
        WHERE last_error IS NOT NULL AND TRIM(last_error) != ''
        UNION ALL
        SELECT '模型文件' AS source, path AS target, last_error AS message, updated_at AS ts
        FROM model_files
        WHERE last_error IS NOT NULL AND TRIM(last_error) != ''
        UNION ALL
        SELECT '垃圾箱' AS source, path AS target, last_error AS message, updated_at AS ts
        FROM model_file_trash
        WHERE last_error IS NOT NULL AND TRIM(last_error) != ''
        ORDER BY ts DESC
        LIMIT 50
        "#,
    )
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok("暂无错误摘要".to_string());
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

async fn run_local_instance_task(
    pool: &SqlitePool,
    id: &str,
    task_kind: &str,
    pending_status: &str,
) -> Result<ModelInstanceView, Stage3Error> {
    let instance = model_instance(pool, id).await?;
    if instance.deploy_type != "local" {
        return Err(Stage3Error::BadRequest(
            "External 实例不由平台启动或停止".to_string(),
        ));
    }
    let node_id = instance
        .node_id
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("本地实例缺少节点".to_string()))?;
    if !node_online(pool, node_id).await? {
        return Err(Stage3Error::Conflict(
            "节点 Agent 离线，无法执行本地实例任务".to_string(),
        ));
    }
    let model_file_id = instance
        .model_file_id
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("本地实例缺少模型文件".to_string()))?;
    let file = verified_model_file_for_instance(pool, model_file_id).await?;
    let runtime_environment_id = instance
        .runtime_environment_id
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("本地实例缺少运行环境".to_string()))?;
    let env = runtime_environment(pool, runtime_environment_id).await?;
    if !runtime_environment_usable(env.check_status.as_deref()) {
        return Err(Stage3Error::BadRequest(
            "运行环境未通过 Agent 检查".to_string(),
        ));
    }
    let params = parse_instance_params(instance.params_json.as_deref())?;

    let task_id = Uuid::new_v4().to_string();
    let now = now_unix_secs();
    let payload = serde_json::json!({
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
        "params": params,
    });
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
    notify_agent_tasks();
    wait_for_model_instance_task(pool, id, &task_id).await
}

async fn wait_for_model_instance_task(
    pool: &SqlitePool,
    instance_id: &str,
    task_id: &str,
) -> Result<ModelInstanceView, Stage3Error> {
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
                    .unwrap_or_else(|| "本地实例任务失败".to_string());
                return Err(Stage3Error::Conflict(message));
            }
            "timed_out" => {
                return Err(Stage3Error::Conflict(
                    "本地实例任务超时，请确认 Agent 在线并重试".to_string(),
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            mark_task_timed_out(pool, task_id).await?;
            update_instance_check(pool, instance_id, "failed", Some("本地实例任务超时")).await?;
            return Err(Stage3Error::Conflict(
                "本地实例任务超时，请确认 Agent 在线并重试".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn update_instance_check(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<ModelInstanceView, Stage3Error> {
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

pub async fn create_model_file_trash(
    pool: &SqlitePool,
    model_file_id: &str,
    request: ModelFileTrashRequest,
) -> Result<ModelFileTrashView, Stage3Error> {
    ensure_model_file_trash(pool, model_file_id, request.reason, request.note).await
}

async fn ensure_model_file_trash(
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
        let message = "节点 Agent 离线，无法清理文件";
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
    notify_agent_tasks();
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
                    .unwrap_or_else(|| "文件清理失败".to_string());
                return Err(Stage3Error::Conflict(message));
            }
            "timed_out" => {
                return Err(Stage3Error::Conflict(
                    "文件清理超时，请确认 Agent 在线并重试".to_string(),
                ));
            }
            _ => {}
        }

        if Instant::now() >= deadline {
            mark_task_timed_out(pool, task_id).await?;
            update_trash_failure(pool, trash_id, "cleanup_timeout", "文件清理超时").await?;
            return Err(Stage3Error::Conflict(
                "文件清理超时，请确认 Agent 在线并重试".to_string(),
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn update_trash_failure(
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

fn runtime_environment_from_row(row: sqlx::sqlite::SqliteRow) -> RuntimeEnvironmentView {
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
        config_json: row.get("config_json"),
        enabled: int_to_bool(row.get("enabled")),
        last_checked_at: row.get("last_checked_at"),
        check_status: row.get("check_status"),
        check_message: row.get("check_message"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

async fn model_from_row(
    pool: &SqlitePool,
    row: sqlx::sqlite::SqliteRow,
) -> Result<ModelView, Stage3Error> {
    let id: String = row.get("id");
    let summary = model_file_summary(pool, &id).await?;
    Ok(ModelView {
        id,
        name: row.get("name"),
        display_name: row.get("display_name"),
        model_type: row.get("model_type"),
        model_path: row.get("model_path"),
        description: row.get("description"),
        default_backend: row.get("default_backend"),
        config_json: row.get("config_json"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        deleted_at: row.get("deleted_at"),
        file_status: summary.file_status,
        total_file_count: summary.total_file_count,
        verified_file_count: summary.verified_file_count,
        available_node_count: summary.available_node_count,
        last_file_verified_at: summary.last_file_verified_at,
    })
}

struct ModelFileSummary {
    file_status: String,
    total_file_count: i64,
    verified_file_count: i64,
    available_node_count: i64,
    last_file_verified_at: Option<i64>,
}

async fn model_file_summary(
    pool: &SqlitePool,
    model_id: &str,
) -> Result<ModelFileSummary, Stage3Error> {
    mark_timed_out_tasks(pool).await?;
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
    mark_timed_out_tasks(pool).await?;
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
                "UPDATE model_files SET status = 'verify_timeout', last_error = '验证超时', updated_at = ? WHERE id = ?",
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

fn model_instance_from_row(row: sqlx::sqlite::SqliteRow) -> ModelInstanceView {
    ModelInstanceView {
        id: row.get("id"),
        model_id: row.get("model_id"),
        model_file_id: row.get("model_file_id"),
        model_definition_name: row.get("model_definition_name"),
        model_file_path: row.get("model_file_path"),
        node_id: row.get("node_id"),
        node_name: row.get("node_name"),
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

fn validate_non_empty(field: &str, value: &str) -> Result<(), Stage3Error> {
    if value.trim().is_empty() {
        return Err(Stage3Error::BadRequest(format!("{field} is required")));
    }
    Ok(())
}

fn validate_optional_non_empty(field: &str, value: Option<&str>) -> Result<(), Stage3Error> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(()),
        _ => Err(Stage3Error::BadRequest(format!("{field} is required"))),
    }
}

fn validate_backend(value: &str) -> Result<(), Stage3Error> {
    validate_one_of(
        "backend",
        value,
        &[
            "vllm",
            "ollama",
            "lmdeploy",
            "mindie",
            "llama_cpp",
            "triton",
            "custom",
        ],
    )
}

fn validate_deploy_type(value: &str) -> Result<(), Stage3Error> {
    validate_one_of("deploy_type", value, &["docker", "script", "binary"])
}

fn validate_instance_deploy_type(value: &str) -> Result<(), Stage3Error> {
    validate_one_of("deploy_type", value, &["external", "local"])
}

fn validate_model_type(value: &str) -> Result<(), Stage3Error> {
    validate_one_of(
        "model_type",
        value,
        &["llm", "embedding", "rerank", "vlm", "asr", "tts", "other"],
    )
}

fn validate_instance_status(value: &str) -> Result<(), Stage3Error> {
    validate_one_of(
        "status",
        value,
        &[
            "pending", "starting", "running", "stopping", "stopped", "failed", "unknown",
        ],
    )
}

fn validate_one_of(field: &str, value: &str, allowed: &[&str]) -> Result<(), Stage3Error> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(Stage3Error::BadRequest(format!("{field} is invalid")))
    }
}

fn validate_instance_urls(
    base_url: &Option<String>,
    endpoint_url: &Option<String>,
    health_url: &Option<String>,
) -> Result<(), Stage3Error> {
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

fn validate_base_url_required(base_url: &Option<String>) -> Result<(), Stage3Error> {
    match base_url.as_deref() {
        Some(value) if !value.trim().is_empty() => Ok(()),
        _ => Err(Stage3Error::BadRequest("base_url is required".to_string())),
    }
}

fn validate_http_url(field: &str, value: &str) -> Result<(), Stage3Error> {
    let parsed = reqwest::Url::parse(value).map_err(|_| {
        Stage3Error::BadRequest(format!("{field} must be a valid http:// or https:// URL"))
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        _ => Err(Stage3Error::BadRequest(format!(
            "{field} must use http:// or https://"
        ))),
    }
}

fn validate_runtime_entrypoints(request: &RuntimeEnvironmentRequest) -> Result<(), Stage3Error> {
    if request
        .base_url
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || request
            .health_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || request
            .endpoint_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return Err(Stage3Error::BadRequest(
            "运行环境不再配置 External URL，请在实例中接入外部服务".to_string(),
        ));
    }
    for (field, value) in [
        ("binary_path", request.binary_path.as_deref()),
        ("working_dir", request.working_dir.as_deref()),
        ("log_dir", request.log_dir.as_deref()),
    ] {
        if let Some(value) = value {
            validate_path(field, value)?;
        }
    }
    if let Some(value) = request.docker_image.as_deref() {
        validate_no_whitespace("docker_image", value)?;
    }
    match request.deploy_type.as_str() {
        "docker" if request.docker_image.as_deref().is_none_or(str::is_empty) => {
            return Err(Stage3Error::BadRequest(
                "Docker 运行环境必须配置镜像".to_string(),
            ));
        }
        "script" | "binary" if request.binary_path.as_deref().is_none_or(str::is_empty) => {
            return Err(Stage3Error::BadRequest(
                "运行环境必须配置受控入口路径".to_string(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn validate_path(field: &str, value: &str) -> Result<(), Stage3Error> {
    if value.trim().is_empty() || value.contains("..") {
        return Err(Stage3Error::BadRequest(format!("{field} is invalid")));
    }
    Ok(())
}

fn validate_no_whitespace(field: &str, value: &str) -> Result<(), Stage3Error> {
    if value.trim().is_empty() || value.chars().any(char::is_whitespace) {
        return Err(Stage3Error::BadRequest(format!("{field} is invalid")));
    }
    Ok(())
}

fn validate_json_field(field: &str, value: Option<&str>) -> Result<(), Stage3Error> {
    if let Some(value) = value {
        serde_json::from_str::<serde_json::Value>(value).map_err(|_| {
            Stage3Error::BadRequest(format!("{field} must be valid JSON when provided"))
        })?;
    }
    Ok(())
}

fn parse_instance_params(value: Option<&str>) -> Result<serde_json::Value, Stage3Error> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(serde_json::json!({}));
    };
    let parsed = serde_json::from_str::<serde_json::Value>(value)
        .map_err(|_| Stage3Error::BadRequest("params_json must be valid JSON".to_string()))?;
    if !parsed.is_object() {
        return Err(Stage3Error::BadRequest(
            "运行参数必须是 JSON 对象".to_string(),
        ));
    }
    if let Some(host) = parsed.get("host").and_then(|value| value.as_str()) {
        if host.trim().is_empty() || host.len() > 128 || host.chars().any(char::is_control) {
            return Err(Stage3Error::BadRequest("监听地址非法".to_string()));
        }
    }
    if let Some(port) = parsed.get("port").and_then(|value| value.as_u64()) {
        if port == 0 || port > u16::MAX as u64 {
            return Err(Stage3Error::BadRequest("监听端口非法".to_string()));
        }
    }
    if let Some(extra_args) = parsed.get("extra_args").and_then(|value| value.as_array()) {
        for arg in extra_args {
            let Some(arg) = arg.as_str() else {
                return Err(Stage3Error::BadRequest(
                    "高级参数必须是一行一个字符串".to_string(),
                ));
            };
            if arg.trim().is_empty() || arg.len() > 256 || arg.chars().any(char::is_control) {
                return Err(Stage3Error::BadRequest("高级参数非法".to_string()));
            }
        }
    }
    Ok(parsed)
}

fn runtime_environment_usable(status: Option<&str>) -> bool {
    matches!(status, Some("available" | "version_unavailable"))
}

async fn ensure_node_online(pool: &SqlitePool, node_id: &str) -> Result<(), Stage3Error> {
    if node_online(pool, node_id).await? {
        Ok(())
    } else {
        Err(Stage3Error::Conflict(
            "Agent is offline; runtime environment cannot be confirmed".to_string(),
        ))
    }
}

async fn ensure_model_exists(pool: &SqlitePool, model_id: &str) -> Result<(), Stage3Error> {
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM models WHERE id = ? AND deleted_at IS NULL")
            .bind(model_id)
            .fetch_optional(pool)
            .await?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(Stage3Error::NotFound("model not found".to_string()))
    }
}

async fn ensure_node_exists(pool: &SqlitePool, node_id: &str) -> Result<(), Stage3Error> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM nodes WHERE id = ?")
        .bind(node_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(Stage3Error::NotFound("node not found".to_string()))
    }
}

async fn verified_model_file_for_instance(
    pool: &SqlitePool,
    model_file_id: &str,
) -> Result<InstanceModelFile, Stage3Error> {
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
    .ok_or_else(|| Stage3Error::BadRequest("模型文件不存在".to_string()))?;
    let status: String = row.get("status");
    if status != "verified" {
        return Err(Stage3Error::BadRequest(
            "本地实例只能使用已验证通过的模型文件".to_string(),
        ));
    }
    Ok(InstanceModelFile {
        model_id: row.get("model_id"),
        node_id: row.get("node_id"),
        path: row.get("path"),
        path_type: row.get("path_type"),
    })
}

async fn mark_timed_out_tasks(pool: &SqlitePool) -> Result<(), Stage3Error> {
    let now = now_unix_secs();
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
                        "UPDATE model_files SET status = 'verify_timeout', last_error = '验证超时', updated_at = ? WHERE id = ?",
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
                    update_trash_failure(pool, trash_id, "cleanup_timeout", "文件清理超时").await?;
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
                    update_instance_check(pool, instance_id, "failed", Some("本地实例任务超时"))
                        .await?;
                }
            }
        }
    }
    Ok(())
}

async fn mark_task_timed_out(pool: &SqlitePool, task_id: &str) -> Result<(), Stage3Error> {
    sqlx::query(
        "UPDATE agent_tasks SET status = 'timed_out', error_message = '任务执行超时', updated_at = ? WHERE id = ? AND status IN ('queued', 'running')",
    )
    .bind(now_unix_secs())
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn node_online(pool: &SqlitePool, node_id: &str) -> Result<bool, Stage3Error> {
    let last_heartbeat_at: Option<i64> =
        sqlx::query_scalar("SELECT last_heartbeat_at FROM nodes WHERE id = ?")
            .bind(node_id)
            .fetch_optional(pool)
            .await?
            .flatten();
    let Some(last_heartbeat_at) = last_heartbeat_at else {
        return Ok(false);
    };
    Ok(now_unix_secs() - last_heartbeat_at <= repository::ONLINE_THRESHOLD_SECS)
}

fn bool_to_int(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn int_to_bool(value: i64) -> bool {
    value != 0
}

fn map_sqlx_conflict(error: sqlx::Error) -> Stage3Error {
    match &error {
        sqlx::Error::Database(database_error)
            if database_error
                .message()
                .contains("UNIQUE constraint failed") =>
        {
            Stage3Error::Conflict("模型名称已存在，请使用其他名称".to_string())
        }
        _ => Stage3Error::Internal(error.into()),
    }
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
