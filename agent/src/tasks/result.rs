use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VerifyModelFileResult {
    pub file_status: String,
    pub size_bytes: Option<i64>,
    pub path_type: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupModelFileResult {
    pub cleanup_status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeEnvironmentCheckResult {
    pub check_status: String,
    pub version: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInstanceTaskResult {
    pub instance_status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_tail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

pub(crate) fn runtime_unavailable(message: &str) -> RuntimeEnvironmentCheckResult {
    RuntimeEnvironmentCheckResult {
        check_status: "unavailable".to_string(),
        version: None,
        message: message.to_string(),
    }
}

pub(crate) fn instance_failure(message: &str) -> ModelInstanceTaskResult {
    instance_failure_with_details(message, None, None)
}

pub(crate) fn instance_failure_with_details(
    message: &str,
    log_tail: Option<String>,
    command: Option<String>,
) -> ModelInstanceTaskResult {
    ModelInstanceTaskResult {
        instance_status: "failed".to_string(),
        message: message.to_string(),
        base_url: None,
        endpoint_url: None,
        process_id: None,
        process_ref: None,
        response_summary: None,
        log_tail,
        command,
    }
}
