use sqlx::SqlitePool;

use crate::models::RuntimeEnvironmentRequest;
use crate::repository;

pub(super) const MODEL_FILE_VERIFY_TIMEOUT_SECS: u64 = 5;
pub(super) const MODEL_FILE_CLEANUP_TIMEOUT_SECS: u64 = 10;
pub(super) const RUNTIME_ENVIRONMENT_CHECK_TIMEOUT_SECS: u64 = 5;
pub(super) const MODEL_INSTANCE_TASK_TIMEOUT_SECS: u64 = 30;
pub(super) const LOG_READ_TIMEOUT_SECS: u64 = 5;
pub(super) const INSTANCE_LOG_TIMEOUT_SECS: u64 = 5;

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

pub(super) fn validate_non_empty(field: &str, value: &str) -> Result<(), Stage3Error> {
    if value.trim().is_empty() {
        return Err(Stage3Error::BadRequest(format!("{field} is required")));
    }
    Ok(())
}

pub(super) fn validate_backend(value: &str) -> Result<(), Stage3Error> {
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

pub(super) fn validate_deploy_type(value: &str) -> Result<(), Stage3Error> {
    validate_one_of("deploy_type", value, &["docker", "script", "binary"])
}

pub(super) fn validate_model_type(value: &str) -> Result<(), Stage3Error> {
    validate_one_of(
        "model_type",
        value,
        &["llm", "embedding", "rerank", "vlm", "asr", "tts", "other"],
    )
}

pub(super) fn validate_one_of(
    field: &str,
    value: &str,
    allowed: &[&str],
) -> Result<(), Stage3Error> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(Stage3Error::BadRequest(format!("{field} is invalid")))
    }
}

pub(super) fn validate_runtime_entrypoints(
    request: &RuntimeEnvironmentRequest,
) -> Result<(), Stage3Error> {
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
            "Runtime no longer accepts External URL; connect external services via Instance"
                .to_string(),
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
                "Docker runtime must specify an image".to_string(),
            ));
        }
        "script" | "binary" if request.binary_path.as_deref().is_none_or(str::is_empty) => {
            return Err(Stage3Error::BadRequest(
                "Runtime must specify a controlled entry path".to_string(),
            ));
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_path(field: &str, value: &str) -> Result<(), Stage3Error> {
    if value.trim().is_empty() || value.contains("..") {
        return Err(Stage3Error::BadRequest(format!("{field} is invalid")));
    }
    Ok(())
}

pub(super) fn validate_no_whitespace(field: &str, value: &str) -> Result<(), Stage3Error> {
    if value.trim().is_empty() || value.chars().any(char::is_whitespace) {
        return Err(Stage3Error::BadRequest(format!("{field} is invalid")));
    }
    Ok(())
}

pub(super) fn validate_json_field(field: &str, value: Option<&str>) -> Result<(), Stage3Error> {
    if let Some(value) = value {
        serde_json::from_str::<serde_json::Value>(value).map_err(|_| {
            Stage3Error::BadRequest(format!("{field} must be valid JSON when provided"))
        })?;
    }
    Ok(())
}

pub(super) async fn ensure_node_online(
    pool: &SqlitePool,
    node_id: &str,
) -> Result<(), Stage3Error> {
    if node_online(pool, node_id).await? {
        Ok(())
    } else {
        Err(Stage3Error::Conflict(
            "Agent is offline; runtime environment cannot be confirmed".to_string(),
        ))
    }
}

pub(super) async fn ensure_model_exists(
    pool: &SqlitePool,
    model_id: &str,
) -> Result<(), Stage3Error> {
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

pub(super) async fn ensure_node_exists(
    pool: &SqlitePool,
    node_id: &str,
) -> Result<(), Stage3Error> {
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

pub(super) async fn node_online(pool: &SqlitePool, node_id: &str) -> Result<bool, Stage3Error> {
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

pub(super) fn bool_to_int(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

pub(super) fn int_to_bool(value: i64) -> bool {
    value != 0
}

pub(super) fn map_sqlx_conflict(error: sqlx::Error) -> Stage3Error {
    match &error {
        sqlx::Error::Database(database_error)
            if database_error
                .message()
                .contains("UNIQUE constraint failed") =>
        {
            Stage3Error::Conflict("model name already exists; use a different name".to_string())
        }
        _ => Stage3Error::Internal(error.into()),
    }
}

pub(super) fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
