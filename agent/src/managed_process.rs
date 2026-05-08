use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::models::ManagedInstanceReport;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManagedProcessRecord {
    pub instance_id: String,
    pub process_id: i64,
    pub process_start_time: Option<u64>,
    pub base_url: Option<String>,
    pub endpoint_url: Option<String>,
    pub command: Option<String>,
    pub log_path: Option<String>,
    pub started_at: i64,
    #[serde(default)]
    pub container_id: Option<String>,
    #[serde(default)]
    pub container_name: Option<String>,
    #[serde(default)]
    pub deploy_type: Option<String>,
}

pub fn store_path_from_state_path(state_path: &str) -> PathBuf {
    let path = Path::new(state_path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("agent-state.toml");
    path.with_file_name(format!("{file_name}.managed-instances.json"))
}

pub async fn load(path: &Path) -> anyhow::Result<Vec<ManagedProcessRecord>> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

pub async fn save(path: &Path, records: &[ManagedProcessRecord]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let content = serde_json::to_vec_pretty(records)?;
    tokio::fs::write(path, content).await?;
    Ok(())
}

pub async fn upsert(path: &Path, record: ManagedProcessRecord) -> anyhow::Result<()> {
    let mut records = load(path).await?;
    records.retain(|existing| existing.instance_id != record.instance_id);
    records.push(record);
    save(path, &records).await
}

pub async fn remove(path: &Path, instance_id: &str) -> anyhow::Result<()> {
    let mut records = load(path).await?;
    records.retain(|existing| existing.instance_id != instance_id);
    save(path, &records).await
}

pub async fn find(path: &Path, instance_id: &str) -> anyhow::Result<Option<ManagedProcessRecord>> {
    Ok(load(path)
        .await?
        .into_iter()
        .find(|record| record.instance_id == instance_id))
}

pub async fn reports(path: Option<&Path>) -> Vec<ManagedInstanceReport> {
    let Some(path) = path else {
        return Vec::new();
    };
    let records = match load(path).await {
        Ok(records) => records,
        Err(error) => {
            tracing::warn!(%error, "managed process store load failed");
            return Vec::new();
        }
    };
    let mut reports = Vec::with_capacity(records.len());
    let mut active_records = Vec::new();
    for record in records {
        let check = check_record(&record).await;
        if check.is_running {
            active_records.push(record.clone());
        }
        reports.push(ManagedInstanceReport {
            instance_id: record.instance_id.clone(),
            status: if check.is_running {
                "running".to_string()
            } else {
                "failed".to_string()
            },
            message: check.message,
            process_id: Some(record.process_id),
            process_ref: Some(record.instance_id.clone()),
            base_url: record.base_url.clone(),
            endpoint_url: record.endpoint_url.clone(),
            command: record.command.clone(),
            log_path: record.log_path.clone(),
        });
    }
    if let Err(error) = save(path, &active_records).await {
        tracing::warn!(%error, "managed process store prune failed");
    }
    reports
}

pub async fn process_start_time(pid: i64) -> Option<u64> {
    platform_process_start_time(pid).await
}

pub async fn kill_managed(record: &ManagedProcessRecord) -> Result<(), String> {
    if record.deploy_type.as_deref() == Some("docker") {
        return crate::tasks::docker_backend::stop_docker_container(record).await;
    }
    let check = check_record(record).await;
    if !check.is_running {
        return Err(check.message);
    }
    platform_kill_process(record.process_id).await
}

struct ProcessCheck {
    is_running: bool,
    message: String,
}

async fn check_record(record: &ManagedProcessRecord) -> ProcessCheck {
    if record.deploy_type.as_deref() == Some("docker") {
        let result = crate::tasks::docker_backend::check_docker_record(record).await;
        return ProcessCheck {
            is_running: result.is_running,
            message: result.message,
        };
    }
    let Some(current_start_time) = process_start_time(record.process_id).await else {
        return ProcessCheck {
            is_running: false,
            message: "Managed process not found; may have exited abnormally".to_string(),
        };
    };
    if let Some(expected_start_time) = record.process_start_time {
        if current_start_time != expected_start_time {
            return ProcessCheck {
                is_running: false,
                message: "PID reused by another process; cannot confirm as platform managed instance; management stopped".to_string(),
            };
        }
    }
    ProcessCheck {
        is_running: true,
        message: "Agent restarted and recovered managed process: still running".to_string(),
    }
}

#[cfg(target_os = "linux")]
async fn platform_process_start_time(pid: i64) -> Option<u64> {
    let stat = tokio::fs::read_to_string(format!("/proc/{pid}/stat"))
        .await
        .ok()?;
    parse_linux_stat_start_time(&stat)
}

#[cfg(not(target_os = "linux"))]
async fn platform_process_start_time(_pid: i64) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
async fn platform_kill_process(pid: i64) -> Result<(), String> {
    let status = tokio::process::Command::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .await
        .map_err(|error| format!("Failed to stop managed process: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to stop managed process: kill exit status {status}"
        ))
    }
}

#[cfg(not(target_os = "linux"))]
async fn platform_kill_process(_pid: i64) -> Result<(), String> {
    Err("Platform does not support stopping managed processes via persisted PID".to_string())
}

#[cfg(target_os = "linux")]
fn parse_linux_stat_start_time(stat: &str) -> Option<u64> {
    let after_comm = stat.rsplit_once(") ")?.1;
    after_comm
        .split_whitespace()
        .nth(19)
        .and_then(|value| value.parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::parse_linux_stat_start_time;
    use super::ManagedProcessRecord;

    #[test]
    fn parses_linux_proc_stat_start_time() {
        let stat = "123 (cmd with space) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 987654 20";

        assert_eq!(parse_linux_stat_start_time(stat), Some(987654));
    }

    #[test]
    fn managed_record_backward_compat_without_docker_fields() {
        let old_json = r#"{
            "instance_id": "inst-1",
            "process_id": 12345,
            "process_start_time": 987654,
            "base_url": "http://127.0.0.1:18080",
            "endpoint_url": "http://127.0.0.1:18080",
            "command": "[\"/usr/local/bin/llama-server\",\"-m\",\"/models/test.gguf\"]",
            "log_path": "/tmp/instance.log",
            "started_at": 1700000000
        }"#;
        let record: ManagedProcessRecord = serde_json::from_str(old_json).unwrap();
        assert_eq!(record.instance_id, "inst-1");
        assert_eq!(record.process_id, 12345);
        assert_eq!(record.process_start_time, Some(987654));
        assert_eq!(record.base_url.as_deref(), Some("http://127.0.0.1:18080"));
        assert!(record.container_id.is_none());
        assert!(record.container_name.is_none());
        assert!(record.deploy_type.is_none());
        // Old local record should NOT be treated as Docker
        assert_ne!(record.deploy_type.as_deref(), Some("docker"));
    }
}
