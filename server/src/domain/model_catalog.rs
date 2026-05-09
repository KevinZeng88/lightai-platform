use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::{
    ensure_node_exists, map_sqlx_conflict, now_unix_secs, validate_backend, validate_json_field,
    validate_model_type, validate_non_empty, validate_path, DomainError,
};
use crate::models::{ModelListResponse, ModelRequest, ModelView};

pub async fn create_model(
    pool: &SqlitePool,
    request: ModelRequest,
) -> Result<ModelView, DomainError> {
    validate_non_empty("name", &request.name)?;
    validate_model_type(&request.model_type)?;
    if let Some(default_backend) = request.default_backend.as_deref() {
        validate_backend(default_backend)?;
    }
    if let Some(model_path) = request.model_path.as_deref() {
        validate_path("model_path", model_path)?;
    }
    validate_json_field("params_json", request.params_json.as_deref())?;
    let initial_file = request
        .initial_file
        .ok_or_else(|| DomainError::BadRequest("initial_file is required".to_string()))?;
    validate_non_empty("initial_file.path", &initial_file.path)?;
    validate_path("initial_file.path", &initial_file.path)?;
    ensure_node_exists(pool, &initial_file.node_id).await?;
    let verified_file = super::model_files::verify_model_file_before_save(
        pool,
        &initial_file.node_id,
        &initial_file.path,
    )
    .await?;

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
            default_backend, params_json, created_at, updated_at
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
    .bind(request.params_json)
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

pub async fn list_models(pool: &SqlitePool) -> Result<ModelListResponse, DomainError> {
    let rows = sqlx::query("SELECT * FROM models WHERE deleted_at IS NULL ORDER BY name")
        .fetch_all(pool)
        .await?;
    let mut models = Vec::with_capacity(rows.len());
    for row in rows {
        models.push(model_from_row(pool, row).await?);
    }
    Ok(ModelListResponse { models })
}

pub async fn model(pool: &SqlitePool, id: &str) -> Result<ModelView, DomainError> {
    let row = sqlx::query("SELECT * FROM models WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| DomainError::NotFound("model not found".to_string()))?;
    model_from_row(pool, row).await
}

pub async fn update_model(
    pool: &SqlitePool,
    id: &str,
    request: ModelRequest,
) -> Result<ModelView, DomainError> {
    validate_non_empty("name", &request.name)?;
    validate_model_type(&request.model_type)?;
    if let Some(default_backend) = request.default_backend.as_deref() {
        validate_backend(default_backend)?;
    }
    if let Some(model_path) = request.model_path.as_deref() {
        validate_path("model_path", model_path)?;
    }
    validate_json_field("params_json", request.params_json.as_deref())?;

    let running_instances: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM model_instances WHERE model_id = ? AND status IN ('running', 'starting', 'stopping')",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    if !running_instances.is_empty() {
        return Err(DomainError::Conflict(format!(
            "Model in use by running instance {}. Cannot modify. Stop the instance first.",
            running_instances.join(", ")
        )));
    }

    let now = now_unix_secs();
    let result = sqlx::query(
        r#"
        UPDATE models
        SET name = ?, display_name = ?, model_type = ?, model_path = ?,
            description = ?, default_backend = ?, params_json = ?, updated_at = ?
        WHERE id = ? AND deleted_at IS NULL
        "#,
    )
    .bind(request.name)
    .bind(request.display_name)
    .bind(request.model_type)
    .bind(request.model_path)
    .bind(request.description)
    .bind(request.default_backend)
    .bind(request.params_json)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(map_sqlx_conflict)?;

    if result.rows_affected() == 0 {
        return Err(DomainError::NotFound("model not found".to_string()));
    }
    model(pool, id).await
}

pub async fn delete_model(pool: &SqlitePool, id: &str) -> Result<(), DomainError> {
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
        return Err(DomainError::Conflict(
            "model has starting or running instances".to_string(),
        ));
    }

    let model_exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM models WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    if model_exists.is_none() {
        return Err(DomainError::NotFound("model not found".to_string()));
    }

    let file_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM model_files WHERE model_id = ? AND deleted_at IS NULL ORDER BY created_at",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    for file_id in file_ids {
        super::model_trash::ensure_model_file_trash(
            pool,
            &file_id,
            Some("Delete model configuration".to_string()),
            Some(
                "Model config removed; actual files not deleted. Process individually in Trash."
                    .to_string(),
            ),
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
        return Err(DomainError::NotFound("model not found".to_string()));
    }
    Ok(())
}

async fn model_from_row(
    pool: &SqlitePool,
    row: sqlx::sqlite::SqliteRow,
) -> Result<ModelView, DomainError> {
    let id: String = row.get("id");
    let summary = super::model_files::model_file_summary(pool, &id).await?;
    Ok(ModelView {
        id,
        name: row.get("name"),
        display_name: row.get("display_name"),
        model_type: row.get("model_type"),
        model_path: row.get("model_path"),
        description: row.get("description"),
        default_backend: row.get("default_backend"),
        params_json: row.get("params_json"),
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
