use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Arc, OnceLock};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Child;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Duration};

use super::process_command::{build_launch_args, command_summary};
use super::process_logs::{
    combined_output_log, controlled_log_path, first_log_line, log_tail, log_tail_with_path,
    sanitize_log, tail_bytes, trim_log_buffer,
};
use super::{
    instance_failure, instance_failure_with_details, verify_controlled_entrypoint,
    verify_model_file, ModelInstanceTaskResult,
};
use crate::managed_process::{self, ManagedProcessRecord};
use crate::platform_log::{self, LogPolicy};
use crate::tasks::docker_backend;
use crate::tasks::probe::{
    endpoint_ready, ProbeConfig, CUSTOM_SCRIPT_STARTUP_WAIT_MS, POST_KILL_LOG_WAIT_MS,
    POST_READINESS_VERIFY_DELAY_MS,
};

#[derive(Debug, Clone)]
pub(crate) struct DockerCheckResult {
    pub(crate) is_running: bool,
    pub(crate) message: String,
}

/// Runtime monitoring: check interval (seconds). Agent background checks managed process liveness.
/// Independent of startup readiness probe. Not configurable in this phase.
const PROCESS_MONITOR_INTERVAL_SECS: u64 = 3;

#[derive(Debug)]
pub(super) struct InstanceLaunchParams {
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) ctx_size: Option<u64>,
    pub(super) gpu_layers: Option<i64>,
    pub(super) threads: Option<u64>,
    pub(super) extra_args: Vec<String>,
}

impl InstanceLaunchParams {
    fn from_payload(payload: &serde_json::Value) -> Result<Self, String> {
        let params = payload
            .get("params_json")
            .unwrap_or(&serde_json::Value::Null);
        let parsed = if let Some(value) = params.as_str() {
            serde_json::from_str::<serde_json::Value>(value).unwrap_or(serde_json::Value::Null)
        } else {
            params.clone()
        };
        let host = parsed
            .get("host")
            .and_then(|value| value.as_str())
            .unwrap_or("127.0.0.1")
            .trim()
            .to_string();
        if !is_safe_host(&host) {
            return Err("invalid listen address".to_string());
        }
        let port = parsed
            .get("port")
            .and_then(|value| value.as_u64())
            .unwrap_or(8080);
        if port == 0 || port > u16::MAX as u64 {
            return Err("invalid listen port".to_string());
        }
        let extra_args = parsed
            .get("extra_args")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for arg in &extra_args {
            validate_arg(arg)?;
        }
        Ok(Self {
            host,
            port: port as u16,
            ctx_size: parsed.get("ctx_size").and_then(|value| value.as_u64()),
            gpu_layers: parsed.get("gpu_layers").and_then(|value| value.as_i64()),
            threads: parsed.get("threads").and_then(|value| value.as_u64()),
            extra_args,
        })
    }
}

pub async fn start_model_instance(payload: &serde_json::Value) -> ModelInstanceTaskResult {
    start_model_instance_with_store(payload, None).await
}

pub async fn start_model_instance_with_store(
    payload: &serde_json::Value,
    managed_store_path: Option<&Path>,
) -> ModelInstanceTaskResult {
    let instance_id = payload
        .get("instance_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let deploy_type = payload
        .get("deploy_type")
        .and_then(|value| value.as_str())
        .unwrap_or_default();

    if deploy_type == "docker" {
        return start_docker_instance(instance_id, payload, managed_store_path).await;
    }

    let backend = payload
        .get("backend")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let model_path = payload
        .get("model_path")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if verify_model_file(model_path).await.file_status != "verified" {
        return instance_failure("model file or directory unavailable; instance not started");
    }
    let Some(binary_path) = payload
        .get("binary_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return instance_failure("runtime is missing managed entry path");
    };
    if verify_controlled_entrypoint(binary_path).await.check_status != "available" {
        return instance_failure("runtime entry unavailable; instance not started");
    }
    let params = match InstanceLaunchParams::from_payload(payload) {
        Ok(params) => params,
        Err(message) => return instance_failure(&message),
    };
    let args = match build_launch_args(backend, deploy_type, model_path, &params) {
        Ok(args) => args,
        Err(message) => return instance_failure(&message),
    };
    if let Err(message) = check_port_available(&params.host, params.port).await {
        return instance_failure(&message);
    }
    let command_summary = command_summary(binary_path, &args);
    let mut std_cmd = StdCommand::new(binary_path);
    std_cmd.args(&args);
    std_cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    if let Some(working_dir) = payload.get("working_dir").and_then(|value| value.as_str()) {
        if !working_dir.trim().is_empty() {
            std_cmd.current_dir(working_dir);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        std_cmd.process_group(0);
    }
    let mut command = tokio::process::Command::from(std_cmd);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return instance_failure_with_details(
                &format!("process start failed: {error}"),
                None,
                Some(command_summary),
            );
        }
    };
    let process_id = child.id().map(|pid| pid as i64);
    let process_start_time = match process_id {
        Some(pid) => managed_process::process_start_time(pid).await,
        None => None,
    };
    let log_buffer = Arc::new(Mutex::new(String::new()));
    let log_path = match payload
        .get("log_dir")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(log_dir) => match controlled_log_path(log_dir, instance_id).await {
            Ok(path) => Some(path),
            Err(message) => {
                return instance_failure_with_details(
                    &format!("log directory unavailable: {message}"),
                    None,
                    Some(command_summary),
                );
            }
        },
        None => None,
    };
    attach_log_reader(
        "stdout",
        child.stdout.take(),
        log_buffer.clone(),
        log_path.clone(),
    );
    attach_log_reader(
        "stderr",
        child.stderr.take(),
        log_buffer.clone(),
        log_path.clone(),
    );
    sleep(Duration::from_millis(250)).await;
    match child.try_wait() {
        Ok(Some(status)) => {
            sleep(Duration::from_millis(50)).await;
            let log_tail = log_tail(&log_buffer).await;
            let detail = first_log_line(log_tail.as_deref()).unwrap_or_else(|| status.to_string());
            return instance_failure_with_details(
                &format!("process exited: {detail}"),
                log_tail,
                Some(command_summary),
            );
        }
        Ok(None) => {}
        Err(error) => {
            return instance_failure_with_details(
                &format!("failed to verify process status: {error}"),
                log_tail(&log_buffer).await,
                Some(command_summary),
            );
        }
    }
    let base_url = format!("http://{}:{}", params.host, params.port);
    let probe = ProbeConfig::from_payload(payload);
    let service_ready = if backend == "custom" && deploy_type == "script" && probe.paths.is_none() {
        sleep(Duration::from_millis(CUSTOM_SCRIPT_STARTUP_WAIT_MS)).await;
        match child.try_wait() {
            Ok(Some(_status)) => false,
            Ok(None) => true,
            Err(_) => false,
        }
    } else {
        endpoint_ready(backend, &base_url, &probe).await
    };
    if !service_ready {
        let _ = child.kill().await;
        sleep(Duration::from_millis(POST_KILL_LOG_WAIT_MS)).await;
        let log_tail = log_tail(&log_buffer).await;
        let detail = first_log_line(log_tail.as_deref())
            .unwrap_or_else(|| "port or health endpoint not ready".to_string());
        return instance_failure_with_details(
            &format!("local process started but service unavailable: {detail}"),
            log_tail,
            Some(command_summary),
        );
    }
    sleep(Duration::from_millis(POST_READINESS_VERIFY_DELAY_MS)).await;
    match child.try_wait() {
        Ok(Some(status)) => {
            let log_tail = log_tail(&log_buffer).await;
            let detail = first_log_line(log_tail.as_deref()).unwrap_or_else(|| status.to_string());
            return instance_failure_with_details(
                &format!("local process exited unexpectedly after service became ready: {detail}"),
                log_tail,
                Some(command_summary),
            );
        }
        Ok(None) => {}
        Err(error) => {
            let _ = child.kill().await;
            return instance_failure_with_details(
                &format!("failed to verify process status: {error}"),
                log_tail(&log_buffer).await,
                Some(command_summary),
            );
        }
    }
    let log_path_text = log_path
        .as_ref()
        .and_then(|path| path.to_str())
        .map(str::to_string);
    let initial_log_tail = log_tail_with_path(&log_buffer, log_path_text.as_deref()).await;
    if let (Some(path), Some(pid)) = (managed_store_path, process_id) {
        if let Err(error) = managed_process::upsert(
            path,
            ManagedProcessRecord {
                instance_id: instance_id.to_string(),
                process_id: pid,
                process_start_time,
                base_url: Some(base_url.clone()),
                endpoint_url: Some(base_url.clone()),
                command: Some(command_summary.clone()),
                log_path: log_path_text.clone(),
                started_at: now_unix_secs(),
                container_id: None,
                container_name: None,
                deploy_type: Some("local".to_string()),
            },
        )
        .await
        {
            let _ = child.kill().await;
            return instance_failure_with_details(
                &format!("failed to write managed process record; instance stopped: {error}"),
                log_tail(&log_buffer).await,
                Some(command_summary),
            );
        }
    }
    spawn_process_monitor(
        instance_id.to_string(),
        managed_store_path.map(Path::to_path_buf),
    );
    process_registry().lock().await.insert(
        instance_id.to_string(),
        ProcessHandle {
            child,
            log_buffer,
            command: command_summary.clone(),
        },
    );
    ModelInstanceTaskResult {
        instance_status: "running".to_string(),
        message: format!("local instance started, listening at {base_url}"),
        base_url: Some(base_url.clone()),
        endpoint_url: Some(base_url),
        process_id,
        process_ref: Some(instance_id.to_string()),
        response_summary: None,
        log_tail: initial_log_tail,
        command: Some(command_summary),
    }
}

pub async fn stop_model_instance(payload: &serde_json::Value) -> ModelInstanceTaskResult {
    stop_model_instance_with_store(payload, None).await
}

pub async fn stop_model_instance_with_store(
    payload: &serde_json::Value,
    managed_store_path: Option<&Path>,
) -> ModelInstanceTaskResult {
    let instance_id = payload
        .get("instance_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default();

    if let Some(store_path) = managed_store_path {
        if let Ok(Some(record)) = managed_process::find(store_path, instance_id).await {
            if record.deploy_type.as_deref() == Some("docker") {
                return stop_docker_instance(instance_id, &record, store_path).await;
            }
        }
    }

    let Some(mut handle) = process_registry().lock().await.remove(instance_id) else {
        let Some(store_path) = managed_store_path else {
            return instance_failure("no local process reference found and no managed process record configured; refusing to stop");
        };
        let record = match managed_process::find(store_path, instance_id).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                return instance_failure(
                    "no platform managed process record found; refusing to stop this instance",
                )
            }
            Err(error) => {
                return instance_failure(&format!(
                    "failed to read managed process record; refusing to stop: {error}"
                ));
            }
        };
        match managed_process::kill_managed(&record).await {
            Ok(()) => {
                if let Err(error) = managed_process::remove(store_path, instance_id).await {
                    return instance_failure(&format!(
                        "instance process stopped but failed to clean managed record: {error}"
                    ));
                }
                return ModelInstanceTaskResult {
                    instance_status: "stopped".to_string(),
                    message: "instance stopped using platform managed process record".to_string(),
                    base_url: None,
                    endpoint_url: None,
                    process_id: None,
                    process_ref: None,
                    response_summary: None,
                    log_tail: None,
                    command: record.command,
                };
            }
            Err(message) => {
                let _ = managed_process::remove(store_path, instance_id).await;
                return ModelInstanceTaskResult {
                    instance_status: "stopped".to_string(),
                    message,
                    base_url: None,
                    endpoint_url: None,
                    process_id: None,
                    process_ref: None,
                    response_summary: None,
                    log_tail: None,
                    command: record.command,
                };
            }
        }
    };
    match handle.child.kill().await {
        Ok(()) => {
            if let Some(store_path) = managed_store_path {
                if let Err(error) = managed_process::remove(store_path, instance_id).await {
                    return instance_failure(&format!(
                        "local instance process stopped but failed to clean managed record: {error}"
                    ));
                }
            }
            ModelInstanceTaskResult {
                instance_status: "stopped".to_string(),
                message: "local instance process stopped".to_string(),
                base_url: None,
                endpoint_url: None,
                process_id: None,
                process_ref: None,
                response_summary: None,
                log_tail: log_tail(&handle.log_buffer).await,
                command: Some(handle.command),
            }
        }
        Err(error) => instance_failure(&format!("failed to stop process: {error}")),
    }
}

pub async fn collect_managed_instance_reports(
    managed_store_path: Option<&Path>,
) -> Vec<crate::models::ManagedInstanceReport> {
    managed_process::reports(managed_store_path).await
}

pub(crate) fn is_custom_script(payload: &serde_json::Value) -> bool {
    payload.get("backend").and_then(|value| value.as_str()) == Some("custom")
        && payload.get("deploy_type").and_then(|value| value.as_str()) == Some("script")
}

pub(crate) async fn run_controlled_script_action(
    payload: &serde_json::Value,
    action: &str,
    success_status: &str,
) -> ModelInstanceTaskResult {
    let Some(binary_path) = payload
        .get("binary_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return instance_failure("custom script path not configured");
    };
    if verify_controlled_entrypoint(binary_path).await.check_status != "available" {
        return instance_failure("custom script entry unavailable");
    }
    let args = vec![action.to_string()];
    let command = command_summary(binary_path, &args);
    let output = match timeout(Duration::from_secs(10), {
        let mut cmd = tokio::process::Command::new(binary_path);
        cmd.args(&args).stdin(Stdio::null());
        cmd.output()
    })
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return instance_failure_with_details(
                &format!("custom script execution failed: {error}"),
                None,
                Some(command),
            );
        }
        Err(_) => {
            return instance_failure_with_details(
                "custom script execution timed out",
                None,
                Some(command),
            )
        }
    };
    let log_tail = combined_output_log(&output.stdout, &output.stderr);
    if output.status.success() {
        ModelInstanceTaskResult {
            instance_status: success_status.to_string(),
            message: format!("custom script {action} completed successfully"),
            base_url: None,
            endpoint_url: None,
            process_id: None,
            process_ref: None,
            response_summary: log_tail.clone(),
            log_tail,
            command: Some(command),
        }
    } else {
        let detail =
            first_log_line(log_tail.as_deref()).unwrap_or_else(|| output.status.to_string());
        instance_failure_with_details(
            &format!("custom script {action} failed: {detail}"),
            log_tail,
            Some(command),
        )
    }
}

pub(crate) async fn running_instance_log_tail(
    instance_id: &str,
    max_bytes: usize,
) -> Option<String> {
    let guard = process_registry().lock().await;
    let handle = guard.get(instance_id)?;
    let log_text = handle.log_buffer.lock().await.clone();
    Some(tail_bytes(&log_text, max_bytes))
}

fn validate_arg(arg: &str) -> Result<(), String> {
    if arg.trim().is_empty() {
        return Err("extra args must not be empty".to_string());
    }
    if arg.len() > 256 || arg.chars().any(|ch| ch.is_control()) {
        return Err("extra args contain invalid characters".to_string());
    }
    Ok(())
}

fn is_safe_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 128
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':' | '_'))
}

fn attach_log_reader(
    label: &'static str,
    stream: Option<impl tokio::io::AsyncRead + Unpin + Send + 'static>,
    log_buffer: Arc<Mutex<String>>,
    log_path: Option<PathBuf>,
) {
    if let Some(mut stream) = stream {
        tokio::spawn(async move {
            let mut file = match log_path {
                Some(path) => tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .await
                    .ok(),
                None => None,
            };
            let mut header_written = false;
            let mut bytes = [0_u8; 1024];
            loop {
                let read = match stream.read(&mut bytes).await {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(_) => break,
                };
                let text = String::from_utf8_lossy(&bytes[..read]);
                let sanitized = sanitize_log(&text);
                if !header_written {
                    let header = format!("{label}:\n");
                    let mut buffer = log_buffer.lock().await;
                    buffer.push_str(&header);
                    if let Some(file) = file.as_mut() {
                        let _ = file.write_all(header.as_bytes()).await;
                    }
                    header_written = true;
                }
                {
                    let mut buffer = log_buffer.lock().await;
                    buffer.push_str(&sanitized);
                    buffer.push('\n');
                    trim_log_buffer(&mut buffer);
                }
                if let Some(file) = file.as_mut() {
                    let _ = file.write_all(sanitized.as_bytes()).await;
                    let _ = file.write_all(b"\n").await;
                    let _ = file.flush().await;
                }
            }
        });
    }
}

async fn start_docker_instance(
    instance_id: &str,
    payload: &serde_json::Value,
    managed_store_path: Option<&Path>,
) -> ModelInstanceTaskResult {
    let docker_payload = resolve_docker_payload(payload).await;

    let container_name = docker_payload.docker.container_name.clone();

    let result = docker_backend::start_docker_container(instance_id, &docker_payload).await;

    if result.instance_status == "running" {
        let now = now_unix_secs();
        let container_id = result.process_ref.clone().unwrap_or_default();
        let base_url = result.base_url.clone().unwrap_or_default();
        let command_summary = result.command.clone().unwrap_or_default();

        if let Some(store_path) = managed_store_path {
            let record = docker_backend::create_docker_managed_record(
                instance_id,
                &container_id,
                &container_name,
                &docker_payload,
                &base_url,
                &command_summary,
                now,
            );
            if let Err(error) = managed_process::upsert(store_path, record).await {
                return instance_failure_with_details(
                    &format!("failed to write Docker managed record: {error}"),
                    None,
                    Some(command_summary),
                );
            }
        }
    }

    result
}

async fn resolve_docker_payload(payload: &serde_json::Value) -> docker_backend::DockerPayload {
    let runtime_params = payload
        .get("runtime_params")
        .or_else(|| payload.get("runtime_config"));

    let runtime_cfg: Option<docker_backend::DockerRuntimeConfig> = runtime_params.and_then(|v| {
        let val: serde_json::Value = if let Some(s) = v.as_str() {
            serde_json::from_str(s).unwrap_or_default()
        } else {
            v.clone()
        };
        serde_json::from_value(val).ok()
    });

    let overrides: Option<docker_backend::DockerInstanceOverrides> = {
        let params = payload.get("params_json");
        params.and_then(|v| {
            let val: serde_json::Value = if let Some(s) = v.as_str() {
                serde_json::from_str(s).unwrap_or_default()
            } else {
                v.clone()
            };
            serde_json::from_value(val).ok()
        })
    };

    if runtime_cfg.is_some() || overrides.is_some() {
        let model_path = payload
            .get("model_path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let model_name = payload
            .get("model_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if let Ok(merged) = docker_backend::merge_docker_config(
            model_path,
            model_name,
            runtime_cfg.as_ref(),
            overrides.as_ref(),
        ) {
            return merged;
        }
    }

    docker_backend::DockerPayload::default()
}

async fn stop_docker_instance(
    instance_id: &str,
    record: &ManagedProcessRecord,
    managed_store_path: &Path,
) -> ModelInstanceTaskResult {
    match docker_backend::stop_docker_container(record).await {
        Ok(()) => {
            let _ = managed_process::remove(managed_store_path, instance_id).await;
            ModelInstanceTaskResult {
                instance_status: "stopped".to_string(),
                message: "Docker container stopped".to_string(),
                base_url: None,
                endpoint_url: None,
                process_id: None,
                process_ref: None,
                response_summary: None,
                log_tail: None,
                command: record.command.clone(),
            }
        }
        Err(message) => {
            let _ = managed_process::remove(managed_store_path, instance_id).await;
            ModelInstanceTaskResult {
                instance_status: "stopped".to_string(),
                message,
                base_url: None,
                endpoint_url: None,
                process_id: None,
                process_ref: None,
                response_summary: None,
                log_tail: None,
                command: record.command.clone(),
            }
        }
    }
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn check_port_available(host: &str, port: u16) -> Result<(), String> {
    let addr = format!("{host}:{port}");
    match TcpListener::bind(&addr).await {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(error) => {
            if error.kind() == std::io::ErrorKind::AddrInUse {
                Err(format!("port {addr} already in use; instance cannot start"))
            } else {
                Err(format!("port {addr} unavailable: {error}"))
            }
        }
    }
}

struct ProcessHandle {
    child: Child,
    log_buffer: Arc<Mutex<String>>,
    command: String,
}

fn process_registry() -> &'static Mutex<HashMap<String, ProcessHandle>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, ProcessHandle>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn spawn_process_monitor(instance_id: String, _managed_store_path: Option<PathBuf>) {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(PROCESS_MONITOR_INTERVAL_SECS)).await;
            let mut guard = process_registry().lock().await;
            let Some(handle) = guard.get_mut(&instance_id) else {
                return;
            };
            match handle.child.try_wait() {
                Ok(Some(status)) => {
                    let log_tail = {
                        let buffer = handle.log_buffer.lock().await;
                        buffer.trim().to_string()
                    };
                    tracing::warn!(
                        instance_id = %instance_id,
                        exit_status = %status,
                        "managed process exited; removing from registry, keeping store record for heartbeat"
                    );
                    let pid = handle
                        .child
                        .id()
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    guard.remove(&instance_id);
                    drop(guard);
                    // Do not remove managed store record: next heartbeat reports() will detect
                    // Report status="failed" with specific reason; Server updates instance state accordingly.
                    let _ = platform_log::append(
                        &LogPolicy::default(),
                        "agent.log",
                        "warn",
                        &format!(
                            "managed instance process exited unexpectedly instance_id={instance_id} pid={pid} exit_status={status}; managed store retains record for next heartbeat reconcile, recent log: {}",
                            &log_tail.chars().take(300).collect::<String>()
                        ),
                    )
                    .await;
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, instance_id = %instance_id, "process try_wait error; removing from registry");
                    guard.remove(&instance_id);
                    return;
                }
            }
            drop(guard);
        }
    });
}
