use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::http_check;
use crate::models::{
    ModelFileTrashListResponse, ModelFileTrashRequest, ModelFileTrashView,
    ModelInstanceCreateRequest, ModelInstanceListResponse, ModelInstanceUpdateRequest,
    ModelInstanceView, ModelListResponse, ModelRequest, ModelView, RuntimeEnvironmentListResponse,
    RuntimeEnvironmentRequest, RuntimeEnvironmentView,
};
use crate::repository;

#[derive(Debug)]
pub enum Stage3Error {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    Internal(anyhow::Error),
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
    validate_external_urls(&request)?;
    validate_runtime_entrypoints(&request)?;
    validate_json_field(
        "allowed_model_dirs_json",
        request.allowed_model_dirs_json.as_deref(),
    )?;
    validate_json_field("config_json", request.config_json.as_deref())?;
    let check_status = if request.deploy_type == "external" {
        "unknown"
    } else {
        ensure_node_online(pool, node_id).await?;
        "pending"
    };

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO runtime_environments (
            id, node_id, name, backend, deploy_type, version, base_url, health_url,
            binary_path, docker_image, working_dir, log_dir, allowed_model_dirs_json,
            config_json, enabled, check_status, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(node_id)
    .bind(request.name)
    .bind(request.backend)
    .bind(request.deploy_type)
    .bind(request.version)
    .bind(request.base_url)
    .bind(request.health_url)
    .bind(request.binary_path)
    .bind(request.docker_image)
    .bind(request.working_dir)
    .bind(request.log_dir)
    .bind(request.allowed_model_dirs_json)
    .bind(request.config_json)
    .bind(bool_to_int(request.enabled.unwrap_or(true)))
    .bind(check_status)
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
    validate_external_urls(&request)?;
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
            health_url = ?, binary_path = ?, docker_image = ?, working_dir = ?,
            log_dir = ?, allowed_model_dirs_json = ?, config_json = ?, enabled = ?,
            updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(request.name)
    .bind(request.backend)
    .bind(request.deploy_type)
    .bind(request.version)
    .bind(request.base_url)
    .bind(request.health_url)
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
    if environment.deploy_type != "external" {
        let Some(node_id) = environment.node_id.as_deref() else {
            return Err(Stage3Error::BadRequest(
                "node_id is required for local runtime environments".to_string(),
            ));
        };
        if !node_online(pool, node_id).await? {
            update_runtime_environment_check(
                pool,
                id,
                "agent_offline",
                "node Agent is offline; runtime environment cannot be checked",
            )
            .await?;
            return Err(Stage3Error::Conflict(
                "Agent is offline; runtime environment cannot be checked".to_string(),
            ));
        }
        return update_runtime_environment_check(
            pool,
            id,
            "pending",
            "runtime environment check will be handled by Agent in a later stage",
        )
        .await;
    }
    let Some(url) = environment
        .health_url
        .as_deref()
        .or(environment.base_url.as_deref())
    else {
        return update_runtime_environment_check(pool, id, "unknown", "no health URL configured")
            .await;
    };
    let result = http_check::check_url(url).await;
    let status = if result.status == "running" {
        "available"
    } else {
        "unavailable"
    };
    update_runtime_environment_check(pool, id, status, &result.message).await
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

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
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
    .execute(pool)
    .await
    .map_err(map_sqlx_conflict)?;

    model(pool, &id).await
}

pub async fn list_models(pool: &SqlitePool) -> Result<ModelListResponse, Stage3Error> {
    let rows = sqlx::query("SELECT * FROM models WHERE deleted_at IS NULL ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(ModelListResponse {
        models: rows.into_iter().map(model_from_row).collect(),
    })
}

pub async fn model(pool: &SqlitePool, id: &str) -> Result<ModelView, Stage3Error> {
    let row = sqlx::query("SELECT * FROM models WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Stage3Error::NotFound("model not found".to_string()))?;
    Ok(model_from_row(row))
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

    let now = now_unix_secs();
    let result = sqlx::query(
        r#"
        UPDATE models
        SET deleted_at = ?, updated_at = ?
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

pub async fn create_model_instance(
    pool: &SqlitePool,
    request: ModelInstanceCreateRequest,
) -> Result<ModelInstanceView, Stage3Error> {
    validate_non_empty("name", &request.name)?;
    validate_instance_status(request.status.as_deref().unwrap_or("unknown"))?;
    let deploy_type = "external";
    let backend = request
        .backend
        .as_deref()
        .ok_or_else(|| Stage3Error::BadRequest("backend is required".to_string()))?;
    validate_backend(backend)?;
    validate_instance_urls(
        &request.base_url,
        &request.endpoint_url,
        &request.health_url,
    )?;
    validate_has_check_url(
        &request.base_url,
        &request.endpoint_url,
        &request.health_url,
    )?;
    validate_json_field("params_json", request.params_json.as_deref())?;

    let model_exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM models WHERE id = ? AND deleted_at IS NULL")
            .bind(&request.model_id)
            .fetch_optional(pool)
            .await?;
    if model_exists.is_none() {
        return Err(Stage3Error::BadRequest("model not found".to_string()));
    }

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO model_instances (
            id, model_id, node_id, runtime_environment_id, name, backend,
            deploy_type, status, base_url, endpoint_url, health_url, runtime_version,
            model_name, description, params_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(request.model_id)
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

pub async fn list_model_instances(
    pool: &SqlitePool,
) -> Result<ModelInstanceListResponse, Stage3Error> {
    let rows = sqlx::query(
        r#"
        SELECT mi.*, m.name AS model_definition_name, n.name AS node_name,
               re.name AS runtime_environment_name
        FROM model_instances mi
        LEFT JOIN models m ON m.id = mi.model_id
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
        SELECT mi.*, m.name AS model_definition_name, n.name AS node_name,
               re.name AS runtime_environment_name
        FROM model_instances mi
        LEFT JOIN models m ON m.id = mi.model_id
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
    let name = request.name.unwrap_or(current.name);
    validate_non_empty("name", &name)?;
    let status = request.status.unwrap_or(current.status);
    validate_instance_status(&status)?;
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
    .bind(request.backend.unwrap_or(current.backend))
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
        return Err(Stage3Error::BadRequest(
            "Stage 3A only checks external instances".to_string(),
        ));
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
    model_id: &str,
    request: ModelFileTrashRequest,
) -> Result<ModelFileTrashView, Stage3Error> {
    validate_non_empty("path", &request.path)?;
    let model_exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM models WHERE id = ?")
        .bind(model_id)
        .fetch_optional(pool)
        .await?;
    if model_exists.is_none() {
        return Err(Stage3Error::NotFound("model not found".to_string()));
    }

    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO model_file_trash (
            id, model_id, node_id, path, reason, status, note, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, 'pending', ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(model_id)
    .bind(request.node_id)
    .bind(request.path)
    .bind(request.reason)
    .bind(request.note)
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
        SELECT t.*, m.name AS model_name, n.name AS node_name
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

async fn model_file_trash_item(
    pool: &SqlitePool,
    id: &str,
) -> Result<ModelFileTrashView, Stage3Error> {
    let row = sqlx::query(
        r#"
        SELECT t.*, m.name AS model_name, n.name AS node_name
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

fn model_from_row(row: sqlx::sqlite::SqliteRow) -> ModelView {
    ModelView {
        id: row.get("id"),
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
    }
}

fn model_instance_from_row(row: sqlx::sqlite::SqliteRow) -> ModelInstanceView {
    ModelInstanceView {
        id: row.get("id"),
        model_id: row.get("model_id"),
        model_definition_name: row.get("model_definition_name"),
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
        last_checked_at: row.get("last_checked_at"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn model_file_trash_from_row(row: sqlx::sqlite::SqliteRow) -> ModelFileTrashView {
    ModelFileTrashView {
        id: row.get("id"),
        model_id: row.get("model_id"),
        model_name: row.get("model_name"),
        node_id: row.get("node_id"),
        node_name: row.get("node_name"),
        path: row.get("path"),
        reason: row.get("reason"),
        status: row.get("status"),
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
    validate_one_of("deploy_type", value, &["external", "docker", "script"])
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

fn validate_external_urls(request: &RuntimeEnvironmentRequest) -> Result<(), Stage3Error> {
    for (field, value) in [
        ("base_url", request.base_url.as_deref()),
        ("health_url", request.health_url.as_deref()),
    ] {
        if let Some(value) = value {
            validate_http_url(field, value)?;
        }
    }
    Ok(())
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

fn validate_has_check_url(
    base_url: &Option<String>,
    endpoint_url: &Option<String>,
    health_url: &Option<String>,
) -> Result<(), Stage3Error> {
    if health_url
        .as_deref()
        .or(endpoint_url.as_deref())
        .or(base_url.as_deref())
        .is_some()
    {
        Ok(())
    } else {
        Err(Stage3Error::BadRequest(
            "base_url, endpoint_url, or health_url is required".to_string(),
        ))
    }
}

fn validate_http_url(field: &str, value: &str) -> Result<(), Stage3Error> {
    if value.starts_with("http://") || value.starts_with("https://") {
        Ok(())
    } else {
        Err(Stage3Error::BadRequest(format!(
            "{field} must start with http:// or https://"
        )))
    }
}

fn validate_runtime_entrypoints(request: &RuntimeEnvironmentRequest) -> Result<(), Stage3Error> {
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

async fn ensure_node_online(pool: &SqlitePool, node_id: &str) -> Result<(), Stage3Error> {
    if node_online(pool, node_id).await? {
        Ok(())
    } else {
        Err(Stage3Error::Conflict(
            "Agent is offline; runtime environment cannot be confirmed".to_string(),
        ))
    }
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
            Stage3Error::Conflict(database_error.message().to_string())
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
