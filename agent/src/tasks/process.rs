use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Arc, OnceLock};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Child;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Duration};

use super::{
    has_parent_dir, instance_failure, instance_failure_with_details, verify_controlled_entrypoint,
    verify_model_file, ModelInstanceTaskResult,
};
use crate::managed_process::{self, ManagedProcessRecord};
use crate::platform_log::{self, LogPolicy};
use crate::tasks::probe::{
    endpoint_ready, ProbeConfig, CUSTOM_SCRIPT_STARTUP_WAIT_MS, POST_KILL_LOG_WAIT_MS,
    POST_READINESS_VERIFY_DELAY_MS,
};

/// 运行状态监控：检查周期（秒）。Agent 后台持续检查受管进程是否存活，
/// 与启动就绪探测无关。本轮不进入配置。
const PROCESS_MONITOR_INTERVAL_SECS: u64 = 3;

#[derive(Debug)]
struct InstanceLaunchParams {
    host: String,
    port: u16,
    ctx_size: Option<u64>,
    gpu_layers: Option<i64>,
    threads: Option<u64>,
    extra_args: Vec<String>,
}

impl InstanceLaunchParams {
    fn from_payload(payload: &serde_json::Value) -> Result<Self, String> {
        let params = payload
            .get("params")
            .or_else(|| payload.get("params_json"))
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
            return Err("监听地址非法".to_string());
        }
        let port = parsed
            .get("port")
            .and_then(|value| value.as_u64())
            .unwrap_or(8080);
        if port == 0 || port > u16::MAX as u64 {
            return Err("监听端口非法".to_string());
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
    let backend = payload
        .get("backend")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let deploy_type = payload
        .get("deploy_type")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let model_path = payload
        .get("model_path")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if verify_model_file(model_path).await.file_status != "verified" {
        return instance_failure("模型文件或目录不可用，实例未启动");
    }
    let Some(binary_path) = payload
        .get("binary_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return instance_failure("运行环境缺少受控入口路径");
    };
    if verify_controlled_entrypoint(binary_path).await.check_status != "available" {
        return instance_failure("运行环境入口不可用，实例未启动");
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
                &format!("启动进程失败：{error}"),
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
                    &format!("日志目录不可用：{message}"),
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
                &format!("启动进程已退出：{detail}"),
                log_tail,
                Some(command_summary),
            );
        }
        Ok(None) => {}
        Err(error) => {
            return instance_failure_with_details(
                &format!("确认进程状态失败：{error}"),
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
            .unwrap_or_else(|| "端口或健康接口未就绪".to_string());
        return instance_failure_with_details(
            &format!("本地进程已启动但服务不可用：{detail}"),
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
                &format!("本地进程在服务就绪后异常退出：{detail}"),
                log_tail,
                Some(command_summary),
            );
        }
        Ok(None) => {}
        Err(error) => {
            let _ = child.kill().await;
            return instance_failure_with_details(
                &format!("确认进程状态失败：{error}"),
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
            },
        )
        .await
        {
            let _ = child.kill().await;
            return instance_failure_with_details(
                &format!("受管进程记录写入失败，实例已停止：{error}"),
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
        message: format!("本地实例已启动，监听地址 {base_url}"),
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
    let Some(mut handle) = process_registry().lock().await.remove(instance_id) else {
        let Some(store_path) = managed_store_path else {
            return instance_failure("未找到本地进程引用，且未配置受管进程记录，拒绝停止");
        };
        let record = match managed_process::find(store_path, instance_id).await {
            Ok(Some(record)) => record,
            Ok(None) => return instance_failure("未找到平台受管进程记录，拒绝停止该实例"),
            Err(error) => {
                return instance_failure(&format!("读取受管进程记录失败，拒绝停止：{error}"));
            }
        };
        match managed_process::kill_managed(&record).await {
            Ok(()) => {
                if let Err(error) = managed_process::remove(store_path, instance_id).await {
                    return instance_failure(&format!(
                        "实例进程已停止，但清理受管记录失败：{error}"
                    ));
                }
                return ModelInstanceTaskResult {
                    instance_status: "stopped".to_string(),
                    message: "已根据平台受管进程记录停止实例".to_string(),
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
                        "本地实例进程已停止，但清理受管记录失败：{error}"
                    ));
                }
            }
            ModelInstanceTaskResult {
                instance_status: "stopped".to_string(),
                message: "本地实例进程已停止".to_string(),
                base_url: None,
                endpoint_url: None,
                process_id: None,
                process_ref: None,
                response_summary: None,
                log_tail: log_tail(&handle.log_buffer).await,
                command: Some(handle.command),
            }
        }
        Err(error) => instance_failure(&format!("停止进程失败：{error}")),
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
        return instance_failure("custom 脚本路径未配置");
    };
    if verify_controlled_entrypoint(binary_path).await.check_status != "available" {
        return instance_failure("custom 脚本入口不可用");
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
                &format!("custom 脚本执行失败：{error}"),
                None,
                Some(command),
            );
        }
        Err(_) => return instance_failure_with_details("custom 脚本执行超时", None, Some(command)),
    };
    let log_tail = combined_output_log(&output.stdout, &output.stderr);
    if output.status.success() {
        ModelInstanceTaskResult {
            instance_status: success_status.to_string(),
            message: format!("custom 脚本 {action} 执行成功"),
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
            &format!("custom 脚本 {action} 执行失败：{detail}"),
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

pub(crate) fn sanitize_log(text: &str) -> String {
    text.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "token",
                "secret",
                "password",
                "api_key",
                "apikey",
                "authorization",
            ]
            .iter()
            .any(|needle| lower.contains(needle))
            {
                "[已隐藏敏感日志行]".to_string()
            } else {
                line.chars().take(500).collect()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn tail_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let start = text.len() - max_bytes;
    text[start..].to_string()
}

fn build_launch_args(
    backend: &str,
    deploy_type: &str,
    model_path: &str,
    params: &InstanceLaunchParams,
) -> Result<Vec<String>, String> {
    if deploy_type == "script" {
        let mut args = vec![
            "start".to_string(),
            "--model".to_string(),
            model_path.to_string(),
            "--host".to_string(),
            params.host.clone(),
            "--port".to_string(),
            params.port.to_string(),
        ];
        args.extend(params.extra_args.clone());
        return Ok(args);
    }
    match backend {
        "llama_cpp" => {
            let mut args = vec![
                "-m".to_string(),
                model_path.to_string(),
                "--host".to_string(),
                params.host.clone(),
                "--port".to_string(),
                params.port.to_string(),
            ];
            if let Some(ctx_size) = params.ctx_size {
                args.extend(["--ctx-size".to_string(), ctx_size.to_string()]);
            }
            if let Some(gpu_layers) = params.gpu_layers {
                args.extend(["--n-gpu-layers".to_string(), gpu_layers.to_string()]);
            }
            if let Some(threads) = params.threads {
                args.extend(["--threads".to_string(), threads.to_string()]);
            }
            args.extend(params.extra_args.clone());
            Ok(args)
        }
        "ollama" | "vllm" | "lmdeploy" | "mindie" | "custom" | "triton" => {
            let mut args = vec![
                "--model".to_string(),
                model_path.to_string(),
                "--host".to_string(),
                params.host.clone(),
                "--port".to_string(),
                params.port.to_string(),
            ];
            args.extend(params.extra_args.clone());
            Ok(args)
        }
        _ => Err("后端类型不受支持".to_string()),
    }
}

fn validate_arg(arg: &str) -> Result<(), String> {
    if arg.trim().is_empty() {
        return Err("高级参数不能为空".to_string());
    }
    if arg.len() > 256 || arg.chars().any(|ch| ch.is_control()) {
        return Err("高级参数包含非法字符".to_string());
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

async fn controlled_log_path(log_dir: &str, instance_id: &str) -> Result<PathBuf, String> {
    if log_dir.trim().is_empty() || has_parent_dir(log_dir) {
        return Err("日志目录路径非法".to_string());
    }
    let dir = Path::new(log_dir);
    if !dir.is_absolute() {
        return Err("日志目录必须是绝对路径".to_string());
    }
    if let Ok(metadata) = tokio::fs::symlink_metadata(dir).await {
        if metadata.file_type().is_symlink() {
            return Err("日志目录不能是软链接".to_string());
        }
        if !metadata.is_dir() {
            return Err("日志目录不是目录".to_string());
        }
    }
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| format!("创建日志目录失败：{error}"))?;
    let safe_id = instance_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    Ok(dir.join(format!("{safe_id}.log")))
}

async fn log_tail(log_buffer: &Arc<Mutex<String>>) -> Option<String> {
    let value = log_buffer.lock().await.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

async fn log_tail_with_path(
    log_buffer: &Arc<Mutex<String>>,
    log_path: Option<&str>,
) -> Option<String> {
    match (log_path, log_tail(log_buffer).await) {
        (Some(path), Some(tail)) => Some(format!("日志文件：{path}\n{tail}")),
        (Some(path), None) => Some(format!("日志文件：{path}")),
        (None, tail) => tail,
    }
}

fn trim_log_buffer(buffer: &mut String) {
    const MAX_LOG_BYTES: usize = 8192;
    if buffer.len() > MAX_LOG_BYTES {
        let start = buffer.len() - MAX_LOG_BYTES;
        *buffer = buffer[start..].to_string();
    }
}

fn combined_output_log(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    let mut parts = Vec::new();
    if !stdout.is_empty() {
        parts.push(format!(
            "stdout:\n{}",
            sanitize_log(&String::from_utf8_lossy(stdout))
        ));
    }
    if !stderr.is_empty() {
        parts.push(format!(
            "stderr:\n{}",
            sanitize_log(&String::from_utf8_lossy(stderr))
        ));
    }
    let text = parts.join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(
            text.chars()
                .rev()
                .take(8192)
                .collect::<String>()
                .chars()
                .rev()
                .collect(),
        )
    }
}

fn command_summary(program: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(program.to_string());
    parts.extend(args.iter().map(|arg| sanitize_arg_for_display(arg)));
    serde_json::to_string(&parts).unwrap_or_else(|_| "[\"<command>\"]".to_string())
}

fn sanitize_arg_for_display(arg: &str) -> String {
    let lower = arg.to_ascii_lowercase();
    if [
        "token",
        "secret",
        "password",
        "api-key",
        "api_key",
        "authorization",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        "[已隐藏]".to_string()
    } else {
        arg.to_string()
    }
}

fn first_log_line(log_tail: Option<&str>) -> Option<String> {
    log_tail?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && *line != "stdout:" && *line != "stderr:")
        .map(|line| line.chars().take(220).collect())
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
                Err(format!("端口 {addr} 已被占用，无法启动实例"))
            } else {
                Err(format!("端口 {addr} 不可用：{error}"))
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
                        .unwrap_or_else(|| "未知".to_string());
                    guard.remove(&instance_id);
                    drop(guard);
                    // 不移除 managed store 记录：下次心跳 reports() 会检查到进程不存在，
                    // 报告 status="failed" 并附带具体原因，Server 据此更新实例状态。
                    let _ = platform_log::append(
                        &LogPolicy::default(),
                        "agent.log",
                        "warn",
                        &format!(
                            "受管实例进程异常退出 instance_id={instance_id} pid={pid} exit_status={status}；managed store 保留记录等待下次心跳上报，最近日志：{}",
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
