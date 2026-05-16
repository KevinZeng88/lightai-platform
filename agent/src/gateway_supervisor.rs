use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
const GATEWAY_LOG_FILE: &str = "lightai-gateway.log";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayProcessSpec {
    pub binary_path: PathBuf,
    pub config_path: PathBuf,
    pub work_dir: PathBuf,
    pub log_path: PathBuf,
    pub state_path: PathBuf,
    pub health_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayCommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub current_dir: PathBuf,
    pub log_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayProcessRecord {
    pub process_id: i64,
    pub process_start_time: Option<u64>,
    pub health_url: String,
    pub command: String,
    pub log_path: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GatewayTaskResult {
    pub gateway_status: String,
    pub message: String,
    pub process_id: Option<i64>,
    pub process_ref: Option<String>,
    pub health_url: Option<String>,
    pub log_tail: Option<String>,
    pub command: Option<String>,
}

impl GatewayProcessSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_non_empty_path("binary_path", &self.binary_path)?;
        validate_non_empty_path("config_path", &self.config_path)?;
        validate_controlled_path("work_dir", &self.work_dir)?;
        validate_controlled_path("log_path", &self.log_path)?;
        validate_controlled_path("state_path", &self.state_path)?;
        validate_health_url(&self.health_url)?;
        if self.work_dir == Path::new("/") {
            return Err("work_dir must not be filesystem root".to_string());
        }
        if self.log_path.file_name().and_then(|value| value.to_str()) != Some(GATEWAY_LOG_FILE) {
            return Err(format!("log_path file name must be {GATEWAY_LOG_FILE}"));
        }
        Ok(())
    }

    pub fn command_spec(&self) -> Result<GatewayCommandSpec, String> {
        self.validate()?;
        Ok(GatewayCommandSpec {
            program: self.binary_path.clone(),
            args: vec![
                "--config".to_string(),
                self.config_path.to_string_lossy().into_owned(),
            ],
            current_dir: self.work_dir.clone(),
            log_path: self.log_path.clone(),
        })
    }
}

pub async fn start_gateway(spec: &GatewayProcessSpec) -> GatewayTaskResult {
    if let Err(message) = spec.validate() {
        return failed(message);
    }
    if let Ok(Some(record)) = load_record(&spec.state_path).await {
        let check = check_record(&record).await;
        if check.is_running {
            return GatewayTaskResult {
                gateway_status: "running".to_string(),
                message: "Gateway is already running".to_string(),
                process_id: Some(record.process_id),
                process_ref: Some(record.process_id.to_string()),
                health_url: Some(record.health_url),
                log_tail: None,
                command: Some(record.command),
            };
        }
    }

    let command_spec = match spec.command_spec() {
        Ok(command_spec) => command_spec,
        Err(message) => return failed(message),
    };
    if let Err(error) = prepare_parent(&spec.state_path).await {
        return failed(format!("Failed to prepare Gateway state path: {error}"));
    }
    if let Err(error) = prepare_parent(&command_spec.log_path).await {
        return failed(format!("Failed to prepare Gateway log path: {error}"));
    }
    let stdout = match tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&command_spec.log_path)
        .await
    {
        Ok(file) => file.into_std().await,
        Err(error) => return failed(format!("Failed to open Gateway log file: {error}")),
    };
    let stderr = match stdout.try_clone() {
        Ok(file) => file,
        Err(error) => return failed(format!("Failed to clone Gateway log file: {error}")),
    };
    let child = match tokio::process::Command::new(&command_spec.program)
        .args(&command_spec.args)
        .current_dir(&command_spec.current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return failed(format!("Failed to start Gateway: {error}")),
    };
    let Some(process_id) = child.id().map(i64::from) else {
        return failed("Failed to start Gateway: missing child process id".to_string());
    };
    drop(child);
    let process_start_time = crate::managed_process::process_start_time(process_id).await;
    let command = format!(
        "{} {}",
        command_spec.program.display(),
        command_spec.args.join(" ")
    );
    let record = GatewayProcessRecord {
        process_id,
        process_start_time,
        health_url: spec.health_url.clone(),
        command: command.clone(),
        log_path: spec.log_path.to_string_lossy().into_owned(),
        started_at: now_unix_secs(),
    };
    if let Err(error) = save_record(&spec.state_path, &record).await {
        return failed(format!("Failed to save Gateway state: {error}"));
    }
    GatewayTaskResult {
        gateway_status: "running".to_string(),
        message: "Gateway started".to_string(),
        process_id: Some(process_id),
        process_ref: Some(process_id.to_string()),
        health_url: Some(spec.health_url.clone()),
        log_tail: None,
        command: Some(command),
    }
}

pub async fn stop_gateway(state_path: &Path) -> GatewayTaskResult {
    let Some(record) = (match load_record(state_path).await {
        Ok(record) => record,
        Err(error) => return failed(format!("Failed to load Gateway state: {error}")),
    }) else {
        return GatewayTaskResult {
            gateway_status: "stopped".to_string(),
            message: "Gateway is not running".to_string(),
            process_id: None,
            process_ref: None,
            health_url: None,
            log_tail: None,
            command: None,
        };
    };
    let check = check_record(&record).await;
    if check.is_running {
        if let Err(message) = kill_process(record.process_id).await {
            return failed(message);
        }
    }
    let _ = tokio::fs::remove_file(state_path).await;
    GatewayTaskResult {
        gateway_status: "stopped".to_string(),
        message: "Gateway stopped".to_string(),
        process_id: Some(record.process_id),
        process_ref: Some(record.process_id.to_string()),
        health_url: Some(record.health_url),
        log_tail: None,
        command: Some(record.command),
    }
}

pub async fn check_gateway(state_path: &Path) -> GatewayTaskResult {
    let Some(record) = (match load_record(state_path).await {
        Ok(record) => record,
        Err(error) => return failed(format!("Failed to load Gateway state: {error}")),
    }) else {
        return failed("Gateway is not running".to_string());
    };
    let check = check_record(&record).await;
    if !check.is_running {
        return failed(check.message);
    }
    match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(client) => match client.get(&record.health_url).send().await {
            Ok(response) if response.status().is_success() => GatewayTaskResult {
                gateway_status: "running".to_string(),
                message: format!("Gateway health check succeeded: HTTP {}", response.status()),
                process_id: Some(record.process_id),
                process_ref: Some(record.process_id.to_string()),
                health_url: Some(record.health_url),
                log_tail: None,
                command: Some(record.command),
            },
            Ok(response) => failed(format!(
                "Gateway health check failed: HTTP {}",
                response.status()
            )),
            Err(error) => failed(format!("Gateway health check failed: {error}")),
        },
        Err(error) => failed(format!(
            "Gateway health check client initialization failed: {error}"
        )),
    }
}

pub async fn read_gateway_log(state_path: &Path, max_bytes: usize) -> GatewayTaskResult {
    let Some(record) = (match load_record(state_path).await {
        Ok(record) => record,
        Err(error) => return failed(format!("Failed to load Gateway state: {error}")),
    }) else {
        return failed("Gateway is not running".to_string());
    };
    let log_path = Path::new(&record.log_path);
    if let Err(message) = validate_controlled_path("log_path", log_path) {
        return failed(message);
    }
    if log_path.file_name().and_then(|value| value.to_str()) != Some(GATEWAY_LOG_FILE) {
        return failed(format!("log_path file name must be {GATEWAY_LOG_FILE}"));
    }
    let bytes = match tokio::fs::read(log_path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return failed("Gateway log file not found".to_string())
        }
        Err(error) => return failed(format!("Failed to read Gateway log file: {error}")),
    };
    let start = bytes.len().saturating_sub(max_bytes.min(512 * 1024));
    let content = crate::platform_log::sanitize(&String::from_utf8_lossy(&bytes[start..]));
    GatewayTaskResult {
        gateway_status: "log_available".to_string(),
        message: "Gateway log read succeeded".to_string(),
        process_id: None,
        process_ref: None,
        health_url: None,
        log_tail: Some(content),
        command: None,
    }
}

async fn load_record(path: &Path) -> anyhow::Result<Option<GatewayProcessRecord>> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Ok(Some(serde_json::from_str(&content)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn save_record(path: &Path, record: &GatewayProcessRecord) -> anyhow::Result<()> {
    prepare_parent(path).await?;
    let content = serde_json::to_vec_pretty(record)?;
    tokio::fs::write(path, content).await?;
    Ok(())
}

async fn prepare_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    Ok(())
}

struct GatewayProcessCheck {
    is_running: bool,
    message: String,
}

async fn check_record(record: &GatewayProcessRecord) -> GatewayProcessCheck {
    let Some(current_start_time) =
        crate::managed_process::process_start_time(record.process_id).await
    else {
        return GatewayProcessCheck {
            is_running: false,
            message: "Gateway process not found; may have exited".to_string(),
        };
    };
    if let Some(expected_start_time) = record.process_start_time {
        if current_start_time != expected_start_time {
            return GatewayProcessCheck {
                is_running: false,
                message: "PID reused by another process; cannot confirm as managed Gateway"
                    .to_string(),
            };
        }
    }
    GatewayProcessCheck {
        is_running: true,
        message: "Gateway process is running".to_string(),
    }
}

#[cfg(target_os = "linux")]
async fn kill_process(pid: i64) -> Result<(), String> {
    let status = tokio::process::Command::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .await
        .map_err(|error| format!("Failed to stop Gateway: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Failed to stop Gateway: kill exit status {status}"))
    }
}

#[cfg(not(target_os = "linux"))]
async fn kill_process(_pid: i64) -> Result<(), String> {
    Err("Platform does not support stopping Gateway by persisted PID".to_string())
}

fn failed(message: String) -> GatewayTaskResult {
    GatewayTaskResult {
        gateway_status: "failed".to_string(),
        message,
        process_id: None,
        process_ref: None,
        health_url: None,
        log_tail: None,
        command: None,
    }
}

pub fn gateway_state_path_from_agent_state_path(agent_state_path: &str) -> PathBuf {
    let path = Path::new(agent_state_path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("agent-state.toml");
    path.with_file_name(format!("{file_name}.gateway.json"))
}

fn validate_non_empty_path(field: &str, path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if path_contains_nul(path) {
        return Err(format!("{field} must not contain NUL bytes"));
    }
    Ok(())
}

fn validate_controlled_path(field: &str, path: &Path) -> Result<(), String> {
    validate_non_empty_path(field, path)?;
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "{field} must not contain parent directory components"
        ));
    }
    Ok(())
}

fn validate_health_url(value: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(value).map_err(|_| "health_url is invalid".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("health_url must use http:// or https://".to_string());
    }
    match parsed.host_str() {
        Some("127.0.0.1" | "localhost" | "::1") => {}
        _ => return Err("health_url must target localhost or loopback".to_string()),
    }
    if parsed.path() != "/health" {
        return Err("health_url path must be /health".to_string());
    }
    if parsed.query().is_some() {
        return Err("health_url must not include query parameters".to_string());
    }
    Ok(())
}

fn path_contains_nul(path: &Path) -> bool {
    path.to_string_lossy().chars().any(|value| value == '\0')
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
