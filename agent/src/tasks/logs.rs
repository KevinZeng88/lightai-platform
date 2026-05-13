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
                let container_ref = match record
                    .container_id
                    .as_deref()
                    .or(record.container_name.as_deref())
                {
                    Some(r) => r,
                    None => {
                        return ReadInstanceLogResult {
                            log_status: "failed".to_string(),
                            content: String::new(),
                            message:
                                "Docker log read failed: no container ID or name in managed store"
                                    .to_string(),
                        };
                    }
                };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_process::ManagedProcessRecord;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_record(
        instance_id: &str,
        deploy_type: Option<&str>,
        container_id: Option<&str>,
        container_name: Option<&str>,
    ) -> ManagedProcessRecord {
        ManagedProcessRecord {
            instance_id: instance_id.to_string(),
            process_id: 0,
            process_start_time: None,
            base_url: None,
            endpoint_url: None,
            command: None,
            log_path: None,
            started_at: 0,
            container_id: container_id.map(|s| s.to_string()),
            container_name: container_name.map(|s| s.to_string()),
            deploy_type: deploy_type.map(|s| s.to_string()),
        }
    }

    async fn write_store(dir: &TempDir, records: &[ManagedProcessRecord]) {
        let path = dir.path().join("managed-instances.json");
        crate::managed_process::save(&path, records).await.unwrap();
    }

    fn store_path(dir: &TempDir) -> std::path::PathBuf {
        dir.path().join("managed-instances.json")
    }

    #[tokio::test]
    async fn docker_record_with_container_id_reads_logs() {
        // Uses Docker if available, returns clear error if not.
        let dir = TempDir::new().unwrap();
        let record = make_record(
            "inst-1",
            Some("docker"),
            Some("abc123"),
            Some("test-container"),
        );
        write_store(&dir, &[record]).await;
        let payload = json!({"instance_id": "inst-1", "max_bytes": 4096});

        let result = read_instance_log(&payload, Some(&store_path(&dir))).await;
        // With Docker available the read succeeds; without it the error is clear.
        if result.log_status == "available" {
            assert!(result.message.contains("read from docker logs"));
        } else {
            assert_eq!(result.log_status, "failed");
            assert!(result.message.contains("Docker log read failed"));
        }
    }

    #[tokio::test]
    async fn docker_record_missing_container_ref_returns_error() {
        let dir = TempDir::new().unwrap();
        let record = make_record("inst-2", Some("docker"), None, None);
        write_store(&dir, &[record]).await;
        let payload = json!({"instance_id": "inst-2", "max_bytes": 4096});

        let result = read_instance_log(&payload, Some(&store_path(&dir))).await;
        assert_eq!(result.log_status, "failed");
        assert!(result.message.contains("no container ID or name"));
    }

    #[tokio::test]
    async fn binary_record_reads_log_file() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("instance.log");
        tokio::fs::write(
            &log_path,
            "model loaded\nlistening on :8080\nrequest served\n",
        )
        .await
        .unwrap();
        let record = ManagedProcessRecord {
            instance_id: "inst-3".to_string(),
            process_id: 12345,
            process_start_time: None,
            base_url: None,
            endpoint_url: None,
            command: None,
            log_path: Some(log_path.to_str().unwrap().to_string()),
            started_at: 0,
            container_id: None,
            container_name: None,
            deploy_type: Some("binary".to_string()),
        };
        write_store(&dir, &[record]).await;
        let payload = json!({"instance_id": "inst-3", "max_bytes": 4096});

        let result = read_instance_log(&payload, Some(&store_path(&dir))).await;
        assert_eq!(result.log_status, "available");
        assert!(result.content.contains("listening on"));
        assert!(result.message.contains("log file"));
    }

    #[tokio::test]
    async fn missing_managed_record_returns_no_log_found() {
        let dir = TempDir::new().unwrap();
        // Empty store
        write_store(&dir, &[]).await;
        let payload = json!({"instance_id": "inst-nonexistent", "max_bytes": 4096});

        let result = read_instance_log(&payload, Some(&store_path(&dir))).await;
        assert_eq!(result.log_status, "failed");
        assert!(result.message.contains("no instance log found"));
    }

    #[tokio::test]
    async fn log_sanitization_preserves_normal_content() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("instance.log");
        tokio::fs::write(&log_path, "INFO model loaded\nDEBUG request received\n")
            .await
            .unwrap();
        let record = ManagedProcessRecord {
            instance_id: "inst-5".to_string(),
            process_id: 1,
            process_start_time: None,
            base_url: None,
            endpoint_url: None,
            command: None,
            log_path: Some(log_path.to_str().unwrap().to_string()),
            started_at: 0,
            container_id: None,
            container_name: None,
            deploy_type: Some("binary".to_string()),
        };
        write_store(&dir, &[record]).await;
        let payload = json!({"instance_id": "inst-5", "max_bytes": 4096});

        let result = read_instance_log(&payload, Some(&store_path(&dir))).await;
        assert_eq!(result.log_status, "available");
        assert!(result.content.contains("INFO model loaded"));
        assert!(result.content.contains("DEBUG request received"));
    }

    #[tokio::test]
    async fn log_sanitization_redacts_sensitive_tokens() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("instance.log");
        tokio::fs::write(&log_path, "Loaded\nAuthorization: Bearer xyz123\nDone\n")
            .await
            .unwrap();
        let record = ManagedProcessRecord {
            instance_id: "inst-6".to_string(),
            process_id: 1,
            process_start_time: None,
            base_url: None,
            endpoint_url: None,
            command: None,
            log_path: Some(log_path.to_str().unwrap().to_string()),
            started_at: 0,
            container_id: None,
            container_name: None,
            deploy_type: Some("script".to_string()),
        };
        write_store(&dir, &[record]).await;
        let payload = json!({"instance_id": "inst-6", "max_bytes": 4096});

        let result = read_instance_log(&payload, Some(&store_path(&dir))).await;
        assert_eq!(result.log_status, "available");
        assert!(!result.content.contains("Bearer"));
        assert!(!result.content.contains("xyz123"));
        assert!(result.content.contains("redacted"));
        assert!(result.content.contains("Loaded"));
        assert!(result.content.contains("Done"));
    }
}
