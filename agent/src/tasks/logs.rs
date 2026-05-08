use std::path::Path;

use super::process::running_instance_log_tail;
use super::process_logs::{sanitize_log, tail_bytes};
use crate::managed_process;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ReadInstanceLogResult {
    pub(crate) log_status: String,
    pub(crate) content: String,
    pub(crate) message: String,
}

pub(crate) async fn read_instance_log(
    payload: &serde_json::Value,
    managed_store_path: Option<&Path>,
) -> ReadInstanceLogResult {
    let instance_id = payload
        .get("instance_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let max_bytes = payload
        .get("max_bytes")
        .and_then(|value| value.as_u64())
        .unwrap_or(64 * 1024)
        .min(512 * 1024) as usize;

    if let Some(tail) = running_instance_log_tail(instance_id, max_bytes).await {
        if tail.trim().is_empty() {
            return ReadInstanceLogResult {
                log_status: "available".to_string(),
                content: "instance process is running; no log output yet".to_string(),
                message: "read from memory buffer".to_string(),
            };
        }
        return ReadInstanceLogResult {
            log_status: "available".to_string(),
            content: tail,
            message: "read from memory buffer".to_string(),
        };
    }

    if let Some(store_path) = managed_store_path {
        if let Ok(Some(record)) = managed_process::find(store_path, instance_id).await {
            if record.deploy_type.as_deref() == Some("docker") {
                let container_ref = record
                    .container_id
                    .as_deref()
                    .or(record.container_name.as_deref())
                    .unwrap_or("unknown");
                match super::docker_backend::read_docker_logs(container_ref, max_bytes).await {
                    Ok(content) => {
                        if content.trim().is_empty() {
                            return ReadInstanceLogResult {
                                log_status: "available".to_string(),
                                content: "Docker container log is empty".to_string(),
                                message: format!("read from docker logs {}", container_ref),
                            };
                        }
                        return ReadInstanceLogResult {
                            log_status: "available".to_string(),
                            content,
                            message: format!("read from docker logs {}", container_ref),
                        };
                    }
                    Err(error) => {
                        return ReadInstanceLogResult {
                            log_status: "failed".to_string(),
                            content: String::new(),
                            message: format!("Docker log read failed: {error}"),
                        };
                    }
                }
            }
            if let Some(ref log_path) = record.log_path {
                match tokio::fs::read_to_string(log_path).await {
                    Ok(content) => {
                        let tail = tail_bytes(&content, max_bytes);
                        if tail.trim().is_empty() {
                            return ReadInstanceLogResult {
                                log_status: "available".to_string(),
                                content: "log file is empty".to_string(),
                                message: format!("read from log file {}", log_path),
                            };
                        }
                        return ReadInstanceLogResult {
                            log_status: "available".to_string(),
                            content: sanitize_log(&tail),
                            message: format!("read from log file {}", log_path),
                        };
                    }
                    Err(error) => {
                        return ReadInstanceLogResult {
                            log_status: "failed".to_string(),
                            content: String::new(),
                            message: format!("failed to read instance log file: {error}"),
                        };
                    }
                }
            }
        }
    }

    ReadInstanceLogResult {
        log_status: "failed".to_string(),
        content: String::new(),
        message: "no instance log found; instance may have stopped or Agent restarted".to_string(),
    }
}
