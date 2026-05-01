use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::time::{sleep, timeout, Duration};

use crate::client::ServerClient;
use crate::config::Config;
use crate::heartbeat::RuntimeConfig;
use crate::models::{AgentConfig, AgentTaskPollRequest, AgentTaskResultRequest};
use crate::state::{self, AgentState};

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

pub async fn run(config: Config, runtime_config: Arc<RwLock<RuntimeConfig>>) {
    let client = ServerClient::new(config.server_url.clone());
    loop {
        let sleep_secs = match state::load(&config.state_path).await {
            Ok(Some(agent_state)) => {
                let allowed_model_dirs = runtime_config.read().await.allowed_model_dirs.clone();
                let current_config_version = runtime_config.read().await.config_version;
                match run_once(
                    &client,
                    &agent_state.agent_token,
                    &agent_state,
                    &allowed_model_dirs,
                    current_config_version,
                )
                .await
                {
                    Ok(next_config) => {
                        if let Some(next_config) = next_config {
                            runtime_config
                                .write()
                                .await
                                .apply_server_config(Some(next_config));
                        }
                        0
                    }
                    Err(error) => {
                        tracing::warn!(%error, "agent task long poll failed");
                        5
                    }
                }
            }
            Ok(None) => 1,
            Err(error) => {
                tracing::warn!(%error, "agent state load failed before task poll");
                5
            }
        };
        if sleep_secs > 0 {
            sleep(Duration::from_secs(sleep_secs)).await;
        }
    }
}

pub async fn run_once(
    client: &ServerClient,
    token: &str,
    state: &AgentState,
    allowed_model_dirs: &[String],
    current_config_version: i64,
) -> anyhow::Result<Option<AgentConfig>> {
    let response = client
        .poll_task(
            token,
            &AgentTaskPollRequest {
                node_id: state.node_id.clone(),
                current_config_version,
            },
        )
        .await?;
    let next_config = response.agent_config;
    let Some(task) = response.task else {
        return Ok(next_config);
    };

    let (status, result) = match task.kind.as_str() {
        "verify_model_file" => {
            let path = task
                .payload
                .get("path")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let result = verify_model_file(path).await;
            let status = if result.file_status == "verified" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "cleanup_model_file" => {
            let path = task
                .payload
                .get("path")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let result = cleanup_model_file(path, allowed_model_dirs).await;
            let status = if result.cleanup_status == "deleted" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "check_runtime_environment" => {
            let result = check_runtime_environment(&task.payload).await;
            let status = if matches!(
                result.check_status.as_str(),
                "available" | "version_unavailable"
            ) {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "start_model_instance" => {
            let result = start_model_instance(&task.payload).await;
            let status = if result.instance_status == "running" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "stop_model_instance" => {
            let result = stop_model_instance(&task.payload).await;
            let status = if result.instance_status == "stopped" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "test_model_instance" => {
            let result = test_model_instance(&task.payload).await;
            let status = if result.instance_status == "running" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        _ => (
            "failed".to_string(),
            serde_json::json!({
                "cleanup_status": "failed",
                "message": "未知任务类型"
            }),
        ),
    };
    client
        .report_task_result(
            token,
            &task.id,
            &AgentTaskResultRequest {
                node_id: state.node_id.clone(),
                status,
                result,
            },
        )
        .await?;
    Ok(next_config)
}

pub async fn check_runtime_environment(
    payload: &serde_json::Value,
) -> RuntimeEnvironmentCheckResult {
    let deploy_type = payload
        .get("deploy_type")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let backend = payload
        .get("backend")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    match deploy_type {
        "docker" => {
            let image = payload
                .get("docker_image")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if image.trim().is_empty() || image.chars().any(char::is_whitespace) {
                return runtime_unavailable("Docker 镜像配置非法");
            }
            RuntimeEnvironmentCheckResult {
                check_status: "available".to_string(),
                version: None,
                message: "Docker 镜像配置已通过基础校验，版本无法自动获取".to_string(),
            }
        }
        "script" | "binary" => {
            let path = payload
                .get("binary_path")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let result = verify_controlled_entrypoint(path).await;
            if result.check_status != "available" {
                return result;
            }
            if let Some(version) = payload
                .get("version")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return RuntimeEnvironmentCheckResult {
                    check_status: "available".to_string(),
                    version: Some(version.to_string()),
                    message: format!("{backend} 入口可用，使用手工填写版本 {version}"),
                };
            }
            if let Some(version) = detect_entrypoint_version(path).await {
                return RuntimeEnvironmentCheckResult {
                    check_status: "available".to_string(),
                    version: Some(version),
                    message: format!("{backend} 入口可用，版本已自动获取"),
                };
            }
            RuntimeEnvironmentCheckResult {
                check_status: "version_unavailable".to_string(),
                version: None,
                message: format!(
                    "{backend} 入口可用，但版本无法自动获取：执行 --version 未返回可识别版本；可手工填写版本"
                ),
            }
        }
        _ => runtime_unavailable("运行方式不受支持"),
    }
}

pub async fn start_model_instance(payload: &serde_json::Value) -> ModelInstanceTaskResult {
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
    let command_summary = command_summary(binary_path, &args);
    let mut command = Command::new(binary_path);
    command.args(&args);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(working_dir) = payload.get("working_dir").and_then(|value| value.as_str()) {
        if !working_dir.trim().is_empty() {
            command.current_dir(working_dir);
        }
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return instance_failure_with_details(
                &format!("启动进程失败：{error}"),
                None,
                Some(command_summary),
            )
        }
    };
    let process_id = child.id().map(|pid| pid as i64);
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
                )
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
            )
        }
    }
    let base_url = format!("http://{}:{}", params.host, params.port);
    if !(backend == "custom" && deploy_type == "script") && !endpoint_ready(&base_url).await {
        let _ = child.kill().await;
        let log_tail = log_tail(&log_buffer).await;
        let detail = first_log_line(log_tail.as_deref())
            .unwrap_or_else(|| "端口或健康接口未就绪".to_string());
        return instance_failure_with_details(
            &format!("本地进程已启动但服务不可用：{detail}"),
            log_tail,
            Some(command_summary),
        );
    }
    let initial_log_tail = log_tail(&log_buffer).await;
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
    let instance_id = payload
        .get("instance_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let Some(mut handle) = process_registry().lock().await.remove(instance_id) else {
        if is_custom_script(payload) {
            return run_controlled_script_action(payload, "stop", "stopped").await;
        }
        return ModelInstanceTaskResult {
            instance_status: "stopped".to_string(),
            message: "未找到本地进程引用，实例状态已标记为停止".to_string(),
            base_url: None,
            endpoint_url: None,
            process_id: None,
            process_ref: None,
            response_summary: None,
            log_tail: None,
            command: None,
        };
    };
    match handle.child.kill().await {
        Ok(()) => ModelInstanceTaskResult {
            instance_status: "stopped".to_string(),
            message: "本地实例进程已停止".to_string(),
            base_url: None,
            endpoint_url: None,
            process_id: None,
            process_ref: None,
            response_summary: None,
            log_tail: log_tail(&handle.log_buffer).await,
            command: Some(handle.command),
        },
        Err(error) => instance_failure(&format!("停止进程失败：{error}")),
    }
}

pub async fn test_model_instance(payload: &serde_json::Value) -> ModelInstanceTaskResult {
    if is_custom_script(payload) {
        return run_controlled_script_action(payload, "test", "running").await;
    }
    let Some(url) = payload
        .get("endpoint_url")
        .and_then(|value| value.as_str())
        .or_else(|| payload.get("base_url").and_then(|value| value.as_str()))
    else {
        return instance_failure("实例缺少测试地址");
    };
    let urls = match build_test_urls(
        payload
            .get("backend")
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
        url,
    ) {
        Ok(urls) => urls,
        Err(message) => return instance_failure(&message),
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(error) => return instance_failure(&format!("测试客户端初始化失败：{error}")),
    };
    let mut failures = Vec::new();
    for url in &urls {
        match client.get(url).send().await {
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let summary = text.chars().take(300).collect::<String>();
                if status.is_success() || status.is_redirection() {
                    return ModelInstanceTaskResult {
                        instance_status: "running".to_string(),
                        message: format!("测试成功：HTTP {status} {url}"),
                        base_url: None,
                        endpoint_url: None,
                        process_id: None,
                        process_ref: None,
                        response_summary: Some(summary),
                        log_tail: None,
                        command: None,
                    };
                }
                failures.push(format!("{url} -> HTTP {status} {summary}"));
            }
            Err(error) => failures.push(format!("{url} -> 请求失败：{error}")),
        }
    }
    instance_failure(&summarize_test_failures(&urls, &failures))
}

async fn verify_controlled_entrypoint(path: &str) -> RuntimeEnvironmentCheckResult {
    if path.trim().is_empty() || has_parent_dir(path) {
        return runtime_unavailable("入口路径非法");
    }
    let path = Path::new(path);
    if !path.is_absolute() {
        return runtime_unavailable("入口路径必须是绝对路径");
    }
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return runtime_unavailable("入口文件不存在");
        }
        Err(error) => return runtime_unavailable(&format!("入口文件不可访问：{error}")),
    };
    if metadata.file_type().is_symlink() {
        return runtime_unavailable("安全风险：入口文件不能是软链接");
    }
    if !metadata.is_file() {
        return RuntimeEnvironmentCheckResult {
            check_status: "invalid_path".to_string(),
            version: None,
            message: "入口路径不是普通文件".to_string(),
        };
    }
    if !is_executable(&metadata) {
        return RuntimeEnvironmentCheckResult {
            check_status: "not_executable".to_string(),
            version: None,
            message: "入口文件不可执行".to_string(),
        };
    }
    RuntimeEnvironmentCheckResult {
        check_status: "available".to_string(),
        version: None,
        message: "入口文件可访问，版本无法自动获取".to_string(),
    }
}

fn runtime_unavailable(message: &str) -> RuntimeEnvironmentCheckResult {
    RuntimeEnvironmentCheckResult {
        check_status: "unavailable".to_string(),
        version: None,
        message: message.to_string(),
    }
}

pub async fn verify_model_file(path: &str) -> VerifyModelFileResult {
    if path.trim().is_empty() || path.contains("..") {
        return failure("invalid_path", "路径非法");
    }

    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return failure("missing", "文件不存在");
        }
        Err(error) => {
            return failure("failed", &format!("读取文件信息失败：{error}"));
        }
    };
    if metadata.file_type().is_symlink() {
        return failure("security_risk", "安全风险：模型路径不能是软链接");
    }

    if metadata.is_dir() {
        return VerifyModelFileResult {
            file_status: "verified".to_string(),
            size_bytes: None,
            path_type: Some("directory".to_string()),
            message: "目录已验证".to_string(),
        };
    }

    if !metadata.is_file() {
        return failure("not_file", "路径不是普通文件或目录");
    }

    VerifyModelFileResult {
        file_status: "verified".to_string(),
        size_bytes: Some(metadata.len().min(i64::MAX as u64) as i64),
        path_type: Some("file".to_string()),
        message: "文件已验证".to_string(),
    }
}

pub async fn cleanup_model_file(
    path: &str,
    allowed_model_dirs: &[String],
) -> CleanupModelFileResult {
    if allowed_model_dirs.is_empty() {
        return cleanup_failure("未配置受控模型目录，拒绝删除文件");
    }
    if path.trim().is_empty() || has_parent_dir(path) {
        return cleanup_failure("路径非法");
    }
    let target = Path::new(path);
    if !target.is_absolute() {
        return cleanup_failure("路径必须是绝对路径");
    }

    let allowed_dirs = match allowed_canonical_dirs(allowed_model_dirs).await {
        AllowedDirResolution::Dirs(dirs) => dirs,
        AllowedDirResolution::InvalidConfig => return cleanup_failure("受控模型目录配置非法"),
        AllowedDirResolution::Missing => return cleanup_failure("受控模型目录不存在"),
        AllowedDirResolution::Inaccessible => return cleanup_failure("受控模型目录不可访问"),
    };
    if allowed_dirs.is_empty() {
        return cleanup_failure("受控模型目录配置非法");
    }

    let metadata = match tokio::fs::symlink_metadata(target).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return cleanup_failure("文件不存在");
        }
        Err(error) => {
            return cleanup_failure(&format!("读取文件信息失败：{error}"));
        }
    };

    if metadata.file_type().is_symlink() {
        return cleanup_failure("安全风险：拒绝删除软链接");
    }
    if !metadata.is_file() {
        return cleanup_failure("拒绝删除目录或非普通文件");
    }

    let canonical_target = match tokio::fs::canonicalize(target).await {
        Ok(path) => path,
        Err(error) => {
            return cleanup_failure(&format!("解析文件路径失败：{error}"));
        }
    };
    if !allowed_dirs
        .iter()
        .any(|allowed_dir| canonical_target.starts_with(allowed_dir))
    {
        return cleanup_failure("文件不在受控模型目录内");
    }

    match tokio::fs::remove_file(target).await {
        Ok(()) => CleanupModelFileResult {
            cleanup_status: "deleted".to_string(),
            message: "文件已清理".to_string(),
        },
        Err(error) => cleanup_failure(&format!("删除文件失败：{error}")),
    }
}

fn failure(status: &str, message: &str) -> VerifyModelFileResult {
    VerifyModelFileResult {
        file_status: status.to_string(),
        size_bytes: None,
        path_type: None,
        message: message.to_string(),
    }
}

fn cleanup_failure(message: &str) -> CleanupModelFileResult {
    CleanupModelFileResult {
        cleanup_status: "failed".to_string(),
        message: message.to_string(),
    }
}

fn has_parent_dir(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}

enum AllowedDirResolution {
    Dirs(Vec<PathBuf>),
    InvalidConfig,
    Missing,
    Inaccessible,
}

async fn allowed_canonical_dirs(allowed_model_dirs: &[String]) -> AllowedDirResolution {
    let mut dirs = Vec::new();
    let mut saw_invalid = false;
    let mut saw_missing = false;
    let mut saw_inaccessible = false;
    for dir in allowed_model_dirs {
        if dir.trim().is_empty() || has_parent_dir(dir) {
            saw_invalid = true;
            continue;
        }
        let path = Path::new(dir);
        if !path.is_absolute() {
            saw_invalid = true;
            continue;
        }
        match tokio::fs::canonicalize(path).await {
            Ok(canonical) => {
                if canonical.is_dir() {
                    dirs.push(canonical);
                } else {
                    saw_invalid = true;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => saw_missing = true,
            Err(_) => saw_inaccessible = true,
        }
    }
    if !dirs.is_empty() {
        AllowedDirResolution::Dirs(dirs)
    } else if saw_invalid {
        AllowedDirResolution::InvalidConfig
    } else if saw_missing {
        AllowedDirResolution::Missing
    } else if saw_inaccessible {
        AllowedDirResolution::Inaccessible
    } else {
        AllowedDirResolution::InvalidConfig
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

async fn detect_entrypoint_version(path: &str) -> Option<String> {
    let output = timeout(
        Duration::from_secs(3),
        Command::new(path).arg("--version").output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.lines()
        .filter_map(parse_version_line)
        .next()
        .map(|version| version.chars().take(120).collect())
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

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

fn is_custom_script(payload: &serde_json::Value) -> bool {
    payload.get("backend").and_then(|value| value.as_str()) == Some("custom")
        && payload.get("deploy_type").and_then(|value| value.as_str()) == Some("script")
}

async fn run_controlled_script_action(
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
    let output = match timeout(
        Duration::from_secs(10),
        Command::new(binary_path).args(&args).output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return instance_failure_with_details(
                &format!("custom 脚本执行失败：{error}"),
                None,
                Some(command),
            )
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

pub fn build_test_urls(backend: &str, base: &str) -> Result<Vec<String>, String> {
    let trimmed = base.trim().trim_end_matches('/');
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("实例测试地址必须是 http:// 或 https://".to_string());
    }
    let parsed = reqwest::Url::parse(trimmed).map_err(|_| "实例测试地址格式非法".to_string())?;
    let path = parsed.path().trim_end_matches('/');
    let host = parsed
        .host_str()
        .ok_or_else(|| "实例测试地址缺少主机".to_string())?;
    let root = match parsed.port() {
        Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
        None => format!("{}://{}", parsed.scheme(), host),
    };
    let mut urls = Vec::new();
    if path.is_empty() {
        match backend {
            "llama_cpp" | "vllm" | "ollama" => {
                urls.push(format!("{trimmed}/v1/models"));
                urls.push(format!("{trimmed}/health"));
                urls.push(trimmed.to_string());
            }
            _ => {
                urls.push(format!("{trimmed}/health"));
                urls.push(format!("{trimmed}/v1/models"));
                urls.push(trimmed.to_string());
            }
        }
    } else {
        urls.push(trimmed.to_string());
        if !path.ends_with("/v1/models") {
            urls.push(format!("{root}/v1/models"));
        }
        if !path.ends_with("/health") {
            urls.push(format!("{root}/health"));
        }
    }
    urls.dedup();
    Ok(urls)
}

pub fn summarize_test_failures(urls: &[String], failures: &[String]) -> String {
    if failures.iter().all(|failure| failure.contains("HTTP 404")) {
        format!(
            "未找到可用测试接口；已尝试 {}。请确认后端是否启用 OpenAI 兼容接口，或在实例中配置正确 endpoint_url。",
            urls.join(", ")
        )
    } else {
        format!("测试失败：{}", failures.join("；"))
    }
}

fn instance_failure(message: &str) -> ModelInstanceTaskResult {
    instance_failure_with_details(message, None, None)
}

fn instance_failure_with_details(
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

fn attach_log_reader(
    label: &'static str,
    stream: Option<impl tokio::io::AsyncRead + Unpin + Send + 'static>,
    log_buffer: Arc<Mutex<String>>,
    log_path: Option<PathBuf>,
) {
    if let Some(mut stream) = stream {
        tokio::spawn(async move {
            let mut file = match log_path {
                Some(path) => {
                    match tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .await
                    {
                        Ok(file) => Some(file),
                        Err(_) => None,
                    }
                }
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

fn trim_log_buffer(buffer: &mut String) {
    const MAX_LOG_BYTES: usize = 8192;
    if buffer.len() > MAX_LOG_BYTES {
        let start = buffer.len() - MAX_LOG_BYTES;
        *buffer = buffer[start..].to_string();
    }
}

fn sanitize_log(text: &str) -> String {
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

async fn endpoint_ready(base_url: &str) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(600))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    for _ in 0..10 {
        for path in ["/health", "/v1/models", "/"] {
            let url = format!("{}{}", base_url.trim_end_matches('/'), path);
            if let Ok(response) = client.get(&url).send().await {
                if response.status().is_success() || response.status().is_redirection() {
                    return true;
                }
            }
        }
        sleep(Duration::from_millis(400)).await;
    }
    false
}

fn parse_version_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("cuda")
        || lower.contains("ggml")
        || lower.starts_with("main:")
        || lower.starts_with("warning")
        || lower.starts_with("error")
    {
        return None;
    }
    let has_digit = trimmed.chars().any(|ch| ch.is_ascii_digit());
    if has_digit
        && (lower.contains("version")
            || lower.starts_with("ollama ")
            || lower.starts_with("vllm ")
            || lower.starts_with("llama.cpp "))
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}
