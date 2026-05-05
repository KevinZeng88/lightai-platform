use std::process::Stdio;

use serde::Deserialize;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::result::{instance_failure, instance_failure_with_details, ModelInstanceTaskResult};
use crate::managed_process::ManagedProcessRecord;
use crate::platform_log::{self, LogPolicy};

const DOCKER_RUN_TIMEOUT_SECS: u64 = 60;
const DOCKER_STOP_TIMEOUT_SECS: u64 = 30;
const DOCKER_INSPECT_TIMEOUT_SECS: u64 = 5;
const DOCKER_LOGS_TAIL_BYTES: usize = 8192;

// ── Docker / vLLM parameter structs ──

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct DockerInstanceParams {
    pub image: String,
    pub container_name: String,
    pub gpu: String,
    pub ipc: String,
    pub ports: Vec<DockerPortMapping>,
    pub volumes: Vec<DockerVolumeMapping>,
    pub env: std::collections::HashMap<String, String>,
    pub extra_docker_args: Vec<String>,
}

impl Default for DockerInstanceParams {
    fn default() -> Self {
        Self {
            image: String::new(),
            container_name: String::new(),
            gpu: "all".to_string(),
            ipc: String::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            env: std::collections::HashMap::new(),
            extra_docker_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct VllmParams {
    pub model: String,
    pub served_model_name: String,
    pub host: String,
    pub port: u16,
    pub gpu_memory_utilization: Option<f64>,
    pub max_model_len: Option<u32>,
    pub max_num_seqs: Option<u32>,
    pub extra_vllm_args: Vec<String>,
}

impl Default for VllmParams {
    fn default() -> Self {
        Self {
            model: String::new(),
            served_model_name: String::new(),
            host: "0.0.0.0".to_string(),
            port: 8000,
            gpu_memory_utilization: None,
            max_model_len: None,
            max_num_seqs: None,
            extra_vllm_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct DockerPayload {
    pub docker: DockerInstanceParams,
    pub vllm: VllmParams,
}

// ── Three-layer config: model + runtime + instance ──

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct DockerRuntimeConfig {
    pub backend: String,
    pub deploy_type: String,
    pub image: String,
    pub entrypoint: String,
    pub gpu: String,
    pub ipc: String,
    pub container_port: u16,
    pub cache_host_path: String,
    pub cache_container_path: String,
    #[serde(alias = "vllm_defaults")]
    pub defaults: BackendDefaults,
    pub extra_docker_args: Vec<String>,
    #[serde(alias = "extra_vllm_args")]
    pub extra_backend_args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct BackendDefaults {
    pub host: String,
    pub port: u16,
    pub gpu_memory_utilization: Option<f64>,
    pub max_model_len: Option<u32>,
    pub max_num_seqs: Option<u32>,
    pub ctx_size: Option<u32>,
    pub n_gpu_layers: Option<i64>,
}

impl Default for BackendDefaults {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8000,
            gpu_memory_utilization: Some(0.5),
            max_model_len: Some(4096),
            max_num_seqs: Some(8),
            ctx_size: Some(4096),
            n_gpu_layers: Some(-1),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct DockerInstanceOverrides {
    pub container_name: String,
    pub host_port: u16,
    pub model_container_path: String,
    pub served_model_name: String,
    pub gpu_memory_utilization: Option<f64>,
    pub max_model_len: Option<u32>,
    pub max_num_seqs: Option<u32>,
    pub gpu: Option<String>,
    pub container_port: Option<u16>,
    pub extra_docker_args: Vec<String>,
    #[serde(alias = "extra_vllm_args")]
    pub extra_backend_args: Vec<String>,
}

pub(crate) fn merge_docker_config(
    model_host_path: &str,
    model_name: &str,
    runtime: Option<&DockerRuntimeConfig>,
    overrides: Option<&DockerInstanceOverrides>,
) -> Result<DockerPayload, String> {
    let rt = runtime.cloned().unwrap_or_default();
    let ov = overrides.cloned().unwrap_or_default();

    if rt.image.trim().is_empty() {
        return Err("Docker 运行环境缺少镜像配置".to_string());
    }
    if model_host_path.trim().is_empty() {
        return Err("模型文件路径未配置".to_string());
    }

    let container_model = if ov.model_container_path.trim().is_empty() {
        let model_dir = std::path::Path::new(model_host_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("model");
        format!("/models/{model_dir}")
    } else {
        ov.model_container_path.trim().to_string()
    };

    let served_name = if ov.served_model_name.trim().is_empty() {
        if model_name.trim().is_empty() {
            return Err("served_model_name 未配置".to_string());
        }
        model_name.to_string()
    } else {
        ov.served_model_name.trim().to_string()
    };

    let host_port = if ov.host_port > 0 {
        ov.host_port
    } else {
        rt.container_port + 10000
    };

    let mut docker = DockerInstanceParams {
        image: rt.image.clone(),
        container_name: ov.container_name.clone(),
        gpu: ov.gpu.clone().unwrap_or(rt.gpu.clone()),
        ipc: rt.ipc.clone(),
        ports: vec![DockerPortMapping {
            host: host_port,
            container: rt.container_port,
        }],
        volumes: Vec::new(),
        extra_docker_args: {
            let mut args = rt.extra_docker_args.clone();
            args.extend(ov.extra_docker_args.clone());
            args
        },
        ..Default::default()
    };

    if !rt.cache_host_path.trim().is_empty() && !rt.cache_container_path.trim().is_empty() {
        docker.volumes.push(DockerVolumeMapping {
            host: rt.cache_host_path.trim().to_string(),
            container: rt.cache_container_path.trim().to_string(),
            readonly: false,
        });
    }

    docker.volumes.push(DockerVolumeMapping {
        host: model_host_path.trim().to_string(),
        container: container_model.clone(),
        readonly: true,
    });

    let vllm = VllmParams {
        model: container_model,
        served_model_name: served_name,
        host: rt.defaults.host.clone(),
        port: rt.defaults.port,
        gpu_memory_utilization: ov
            .gpu_memory_utilization
            .or(rt.defaults.gpu_memory_utilization),
        max_model_len: ov.max_model_len.or(rt.defaults.max_model_len),
        max_num_seqs: ov.max_num_seqs.or(rt.defaults.max_num_seqs),
        extra_vllm_args: {
            let mut args = rt.extra_backend_args.clone();
            args.extend(ov.extra_backend_args.clone());
            args
        },
    };

    Ok(DockerPayload { docker, vllm })
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DockerPortMapping {
    pub host: u16,
    pub container: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DockerVolumeMapping {
    pub host: String,
    pub container: String,
    #[serde(default)]
    pub readonly: bool,
}

// ── Payload parsing ──

pub(crate) fn parse_docker_payload(payload: &serde_json::Value) -> Result<DockerPayload, String> {
    let params = payload
        .get("params")
        .or_else(|| payload.get("params_json"))
        .unwrap_or(&serde_json::Value::Null);
    let parsed: serde_json::Value = if let Some(value) = params.as_str() {
        serde_json::from_str(value).unwrap_or(serde_json::Value::Null)
    } else {
        params.clone()
    };
    serde_json::from_value::<DockerPayload>(parsed).map_err(|e| format!("Docker 参数解析失败：{e}"))
}

// ── Docker run arg construction ──

pub(crate) fn build_docker_run_args(payload: &DockerPayload) -> Result<Vec<String>, String> {
    let docker = &payload.docker;
    if docker.image.trim().is_empty() {
        return Err("Docker 镜像未配置".to_string());
    }
    let mut args = vec!["run".to_string()];
    if !docker.container_name.trim().is_empty() {
        args.push("--name".to_string());
        args.push(docker.container_name.trim().to_string());
    }
    if !docker.gpu.trim().is_empty() {
        args.push("--gpus".to_string());
        args.push(docker.gpu.trim().to_string());
    }
    if !docker.ipc.trim().is_empty() {
        args.push("--ipc".to_string());
        args.push(docker.ipc.trim().to_string());
    }
    for port in &docker.ports {
        args.push("-p".to_string());
        args.push(format!("{}:{}", port.host, port.container));
    }
    for volume in &docker.volumes {
        let mut vol = format!("{}:{}", volume.host.trim(), volume.container.trim());
        if volume.readonly {
            vol.push_str(":ro");
        }
        args.push("-v".to_string());
        args.push(vol);
    }
    for (key, value) in &docker.env {
        args.push("-e".to_string());
        args.push(format!("{key}={value}"));
    }
    for extra in &docker.extra_docker_args {
        validate_docker_arg(extra)?;
        args.push(extra.to_string());
    }
    args.push("--detach".to_string());
    args.push(docker.image.trim().to_string());
    Ok(args)
}

pub(crate) fn build_vllm_args(payload: &DockerPayload) -> Vec<String> {
    let vllm = &payload.vllm;
    let mut args = Vec::new();
    if !vllm.model.trim().is_empty() {
        args.push("--model".to_string());
        args.push(vllm.model.trim().to_string());
    }
    if !vllm.served_model_name.trim().is_empty() {
        args.push("--served-model-name".to_string());
        args.push(vllm.served_model_name.trim().to_string());
    }
    if !vllm.host.trim().is_empty() {
        args.push("--host".to_string());
        args.push(vllm.host.trim().to_string());
    }
    if vllm.port > 0 {
        args.push("--port".to_string());
        args.push(vllm.port.to_string());
    }
    if let Some(gmu) = vllm.gpu_memory_utilization {
        args.push("--gpu-memory-utilization".to_string());
        args.push(format!("{gmu}"));
    }
    if let Some(ml) = vllm.max_model_len {
        args.push("--max-model-len".to_string());
        args.push(ml.to_string());
    }
    if let Some(ns) = vllm.max_num_seqs {
        args.push("--max-num-seqs".to_string());
        args.push(ns.to_string());
    }
    for extra in &vllm.extra_vllm_args {
        args.push(extra.to_string());
    }
    args
}

fn validate_docker_arg(arg: &str) -> Result<(), String> {
    if arg.trim().is_empty() {
        return Err("Docker 高级参数不能为空".to_string());
    }
    if arg.len() > 512 || arg.chars().any(|ch| ch.is_control()) {
        return Err("Docker 高级参数包含非法字符".to_string());
    }
    Ok(())
}

pub(crate) fn docker_command_summary(
    image: &str,
    docker_args: &[String],
    vllm_args: &[String],
) -> String {
    let mut parts = vec!["docker".to_string()];
    parts.extend(docker_args.iter().map(|a| sanitize_docker_arg(a)));
    parts.push(image.to_string());
    if !vllm_args.is_empty() {
        parts.extend(vllm_args.iter().map(|a| sanitize_docker_arg(a)));
    }
    serde_json::to_string(&parts).unwrap_or_else(|_| "[\"docker\"]".to_string())
}

fn sanitize_docker_arg(arg: &str) -> String {
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

// ── Docker start ──

pub(crate) async fn start_docker_container(
    instance_id: &str,
    payload: &DockerPayload,
) -> ModelInstanceTaskResult {
    let docker_args = match build_docker_run_args(payload) {
        Ok(args) => args,
        Err(message) => return instance_failure(&message),
    };
    let vllm_args = build_vllm_args(payload);
    let image = payload.docker.image.trim().to_string();
    let command_summary = docker_command_summary(&image, &docker_args, &vllm_args);

    let mut all_args = docker_args.clone();
    all_args.extend(vllm_args.clone());

    let mut cmd = Command::new("docker");
    cmd.args(&all_args);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = match timeout(Duration::from_secs(DOCKER_RUN_TIMEOUT_SECS), cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return instance_failure_with_details(
                &format!("Docker run 失败：{error}"),
                None,
                Some(command_summary),
            );
        }
        Err(_) => {
            return instance_failure_with_details("Docker run 超时", None, Some(command_summary));
        }
    };

    let stderr_text = String::from_utf8_lossy(&output.stderr);
    let stdout_text = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        let detail = stderr_text
            .lines()
            .next()
            .unwrap_or("Docker run 失败")
            .chars()
            .take(300)
            .collect::<String>();
        return instance_failure_with_details(
            &format!("Docker run 失败：{detail}"),
            Some(stderr_text.chars().take(4096).collect()),
            Some(command_summary),
        );
    }

    let container_id = stdout_text.trim().to_string();

    let _ = platform_log::append(
        &LogPolicy::default(),
        "agent.log",
        "info",
        &format!(
            "Docker 容器已启动 instance_id={instance_id} container_id={container_id} container_name={}",
            payload.docker.container_name,
        ),
    )
    .await;

    let host_port = payload.docker.ports.first().map(|p| p.host).unwrap_or(8000);
    let base_url = format!("http://127.0.0.1:{host_port}");

    ModelInstanceTaskResult {
        instance_status: "running".to_string(),
        message: format!("Docker 容器已启动 container_id={container_id}"),
        base_url: Some(base_url.clone()),
        endpoint_url: Some(base_url),
        process_id: None,
        process_ref: Some(container_id.clone()),
        response_summary: None,
        log_tail: None,
        command: Some(command_summary),
    }
}

// ── Docker managed record ──

pub(crate) fn create_docker_managed_record(
    instance_id: &str,
    container_id: &str,
    container_name: &str,
    _payload: &DockerPayload,
    base_url: &str,
    command_summary: &str,
    started_at: i64,
) -> ManagedProcessRecord {
    ManagedProcessRecord {
        instance_id: instance_id.to_string(),
        process_id: 0,
        process_start_time: None,
        base_url: Some(base_url.to_string()),
        endpoint_url: Some(base_url.to_string()),
        command: Some(command_summary.to_string()),
        log_path: None,
        started_at,
        container_id: Some(container_id.to_string()),
        container_name: Some(container_name.to_string()),
        deploy_type: Some("docker".to_string()),
    }
}

// ── Docker stop ──

pub(crate) async fn stop_docker_container(record: &ManagedProcessRecord) -> Result<(), String> {
    let container_ref = record
        .container_id
        .as_deref()
        .or(record.container_name.as_deref())
        .ok_or_else(|| "缺少容器 ID 或名称，无法停止".to_string())?;

    let output = timeout(
        Duration::from_secs(DOCKER_STOP_TIMEOUT_SECS),
        Command::new("docker")
            .args(["stop", container_ref])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| "Docker stop 超时".to_string())?
    .map_err(|e| format!("Docker stop 失败：{e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Docker stop 失败：{}", stderr.trim()));
    }

    Ok(())
}

// ── Docker inspect / check ──

pub(crate) async fn inspect_docker_container(
    container_ref: &str,
) -> Result<DockerContainerStatus, String> {
    let output = timeout(
        Duration::from_secs(DOCKER_INSPECT_TIMEOUT_SECS),
        Command::new("docker")
            .args(["inspect", container_ref])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| "Docker inspect 超时".to_string())?
    .map_err(|e| format!("Docker inspect 失败：{e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No such object") {
            return Ok(DockerContainerStatus {
                running: false,
                message: "容器不存在".to_string(),
                exit_code: None,
            });
        }
        return Err(format!("Docker inspect 失败：{}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).map_err(|e| format!("Docker inspect 输出解析失败：{e}"))?;
    let container = parsed
        .first()
        .ok_or_else(|| "Docker inspect 返回空数组".to_string())?;
    let running = container["State"]["Running"].as_bool().unwrap_or(false);
    let exit_code = container["State"]["ExitCode"].as_i64();
    let error_msg = container["State"]["Error"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let message = if running {
        "容器正在运行".to_string()
    } else if let Some(ref error) = error_msg {
        format!("容器已退出，错误信息：{error}")
    } else {
        format!(
            "容器已退出，退出码：{}",
            exit_code.map_or("未知".to_string(), |c| c.to_string())
        )
    };
    Ok(DockerContainerStatus {
        running,
        message,
        exit_code,
    })
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct DockerContainerStatus {
    pub running: bool,
    pub message: String,
    pub exit_code: Option<i64>,
}

pub(crate) async fn read_docker_logs(
    container_ref: &str,
    tail_bytes: usize,
) -> Result<String, String> {
    let tail_lines = format!("{}", tail_bytes.min(DOCKER_LOGS_TAIL_BYTES));
    let output = timeout(
        Duration::from_secs(10),
        Command::new("docker")
            .args(["logs", "--tail", &tail_lines, container_ref])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| "Docker logs 超时".to_string())?
    .map_err(|e| format!("Docker logs 失败：{e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    let start = combined.len().saturating_sub(tail_bytes);
    Ok(super::process_logs::sanitize_log(&combined[start..]))
}

pub(crate) async fn check_docker_record(
    record: &ManagedProcessRecord,
) -> super::process::DockerCheckResult {
    let container_ref = match record
        .container_id
        .as_deref()
        .or(record.container_name.as_deref())
    {
        Some(r) => r,
        None => {
            return super::process::DockerCheckResult {
                is_running: false,
                message: "缺少容器 ID 或名称".to_string(),
            }
        }
    };
    match inspect_docker_container(container_ref).await {
        Ok(status) => super::process::DockerCheckResult {
            is_running: status.running,
            message: status.message,
        },
        Err(error) => super::process::DockerCheckResult {
            is_running: false,
            message: format!("Docker 容器检查失败：{error}"),
        },
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_minimal_docker_payload() {
        let payload = json!({
            "params_json": json!({
                "docker": {
                    "image": "vllm/vllm-openai:latest",
                    "container_name": "test-container"
                },
                "vllm": {
                    "model": "/models/test",
                    "served_model_name": "test-model"
                }
            }).to_string()
        });
        let parsed = parse_docker_payload(&payload).unwrap();
        assert_eq!(parsed.docker.image, "vllm/vllm-openai:latest");
        assert_eq!(parsed.docker.container_name, "test-container");
        assert_eq!(parsed.vllm.model, "/models/test");
        assert_eq!(parsed.vllm.served_model_name, "test-model");
    }

    #[test]
    fn parses_docker_payload_with_defaults_for_missing_fields() {
        let payload = json!({
            "params": json!({
                "docker": {"image": "nginx:latest"},
                "vllm": {}
            })
        });
        let parsed = parse_docker_payload(&payload).unwrap();
        assert_eq!(parsed.docker.gpu, "all");
        assert!(parsed.docker.ipc.is_empty());
        assert!(parsed.docker.ports.is_empty());
    }

    #[test]
    fn build_docker_run_args_with_ports_volumes_and_gpu() {
        let payload = DockerPayload {
            docker: DockerInstanceParams {
                image: "vllm/vllm-openai:latest".to_string(),
                container_name: "test-qwen".to_string(),
                gpu: "all".to_string(),
                ipc: "host".to_string(),
                ports: vec![DockerPortMapping {
                    host: 18000,
                    container: 8000,
                }],
                volumes: vec![DockerVolumeMapping {
                    host: "/data/models".to_string(),
                    container: "/models".to_string(),
                    readonly: true,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let args = build_docker_run_args(&payload).unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("--name test-qwen"));
        assert!(joined.contains("--gpus all"));
        assert!(joined.contains("--ipc host"));
        assert!(joined.contains("-p 18000:8000"));
        assert!(joined.contains("-v /data/models:/models:ro"));
        assert!(joined.contains("--detach"));
        assert!(
            !joined.contains("--rm"),
            "default args must not include --rm"
        );
        assert!(joined.contains("vllm/vllm-openai:latest"));
    }

    #[test]
    fn build_docker_run_args_no_rm_default() {
        let payload = DockerPayload {
            docker: DockerInstanceParams {
                image: "test:latest".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let args = build_docker_run_args(&payload).unwrap();
        let joined = args.join(" ");
        assert!(
            !joined.contains("--rm"),
            "default docker run must not include --rm"
        );
    }

    #[test]
    fn build_docker_run_args_with_env_vars() {
        let mut env_map = std::collections::HashMap::new();
        env_map.insert("HF_HOME".to_string(), "/cache/hf".to_string());
        let payload = DockerPayload {
            docker: DockerInstanceParams {
                image: "test:latest".to_string(),
                env: env_map,
                ..Default::default()
            },
            ..Default::default()
        };
        let args = build_docker_run_args(&payload).unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-e HF_HOME=/cache/hf"));
    }

    #[test]
    fn build_docker_run_args_rejects_empty_image() {
        let result = build_docker_run_args(&DockerPayload::default());
        assert!(result.is_err());
    }

    #[test]
    fn build_vllm_args_with_full_config() {
        let payload = DockerPayload {
            vllm: VllmParams {
                model: "/models/qwen3".to_string(),
                served_model_name: "qwen3-0.6b".to_string(),
                host: "0.0.0.0".to_string(),
                port: 8000,
                gpu_memory_utilization: Some(0.5),
                max_model_len: Some(4096),
                max_num_seqs: Some(8),
                ..Default::default()
            },
            ..Default::default()
        };
        let args = build_vllm_args(&payload);
        let joined = args.join(" ");
        assert!(joined.contains("--model /models/qwen3"));
        assert!(joined.contains("--served-model-name qwen3-0.6b"));
        assert!(joined.contains("--gpu-memory-utilization 0.5"));
        assert!(joined.contains("--max-model-len 4096"));
        assert!(joined.contains("--max-num-seqs 8"));
    }

    #[test]
    fn build_vllm_args_omits_optional_fields_when_none() {
        let payload = DockerPayload {
            vllm: VllmParams {
                model: "/models/test".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let args = build_vllm_args(&payload);
        let joined = args.join(" ");
        assert!(!joined.contains("max-model-len"));
        assert!(!joined.contains("max-num-seqs"));
        assert!(!joined.contains("gpu-memory-utilization"));
    }

    #[test]
    fn docker_command_summary_checks() {
        let summary = docker_command_summary(
            "vllm/vllm-openai:latest",
            &[
                "run".to_string(),
                "--rm".to_string(),
                "--detach".to_string(),
            ],
            &["--model".to_string(), "/models/test".to_string()],
        );
        assert!(summary.contains("docker"));
        assert!(summary.contains("vllm/vllm-openai:latest"));
    }

    #[test]
    fn inspect_parses_running_container_json() {
        let output = json!([{
            "State": {"Running": true, "ExitCode": 0, "Error": ""}
        }]);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&output.to_string()).unwrap();
        let container = &parsed[0];
        assert!(container["State"]["Running"].as_bool().unwrap());
    }

    #[test]
    fn inspect_parses_exited_container_json() {
        let output = json!([{
            "State": {"Running": false, "ExitCode": 1, "Error": "OOM killed"}
        }]);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&output.to_string()).unwrap();
        let container = &parsed[0];
        assert!(!container["State"]["Running"].as_bool().unwrap());
        assert_eq!(container["State"]["Error"].as_str().unwrap(), "OOM killed");
    }

    #[test]
    fn detect_container_not_found_from_stderr() {
        let stderr = "Error: No such object: container";
        assert!(stderr.contains("No such object"));
    }

    // ── Three-layer merge tests ──

    fn make_runtime() -> DockerRuntimeConfig {
        DockerRuntimeConfig {
            image: "vllm/vllm-openai:latest".to_string(),
            gpu: "all".to_string(),
            ipc: "host".to_string(),
            container_port: 8000,
            cache_host_path: "/data/vllm-cache".to_string(),
            cache_container_path: "/root/.cache/huggingface".to_string(),
            defaults: BackendDefaults::default(),
            ..Default::default()
        }
    }

    fn make_overrides() -> DockerInstanceOverrides {
        DockerInstanceOverrides {
            container_name: "test-qwen".to_string(),
            host_port: 18000,
            model_container_path: String::new(),
            served_model_name: String::new(),
            ..Default::default()
        }
    }

    #[test]
    fn merge_basic_config_produces_valid_docker_args() {
        let merged = merge_docker_config(
            "/data/models/qwen3-0.6b",
            "qwen3-0.6b",
            Some(&make_runtime()),
            Some(&make_overrides()),
        )
        .unwrap();

        let args = build_docker_run_args(&merged).unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("--gpus all"));
        assert!(joined.contains("--name test-qwen"));
        assert!(joined.contains("-p 18000:8000"));
        assert!(joined.contains("-v /data/models/qwen3-0.6b:/models/qwen3-0.6b:ro"));
        assert!(joined.contains("vllm/vllm-openai:latest"));
        assert!(!joined.contains("--rm"), "merge should not add --rm");
    }

    #[test]
    fn merge_uses_model_name_as_served_model_name_when_not_overridden() {
        let merged = merge_docker_config(
            "/data/models/qwen3-0.6b",
            "qwen3-0.6b",
            Some(&make_runtime()),
            Some(&make_overrides()),
        )
        .unwrap();
        assert_eq!(merged.vllm.served_model_name, "qwen3-0.6b");
    }

    #[test]
    fn merge_instance_override_prioritized_over_runtime_default() {
        let mut ov = make_overrides();
        ov.gpu_memory_utilization = Some(0.3);
        ov.max_model_len = Some(2048);

        let merged = merge_docker_config(
            "/data/models/test",
            "test-model",
            Some(&make_runtime()),
            Some(&ov),
        )
        .unwrap();
        assert_eq!(merged.vllm.gpu_memory_utilization, Some(0.3));
        assert_eq!(merged.vllm.max_model_len, Some(2048));
        // max_num_seqs falls back to runtime default
        assert_eq!(merged.vllm.max_num_seqs, Some(8));
    }

    #[test]
    fn merge_falls_back_to_runtime_defaults_when_no_overrides() {
        let merged = merge_docker_config(
            "/data/models/test",
            "test-model",
            Some(&make_runtime()),
            None,
        )
        .unwrap();
        assert_eq!(merged.vllm.max_num_seqs, Some(8));
        assert_eq!(merged.vllm.gpu_memory_utilization, Some(0.5));
    }

    #[test]
    fn merge_rejects_empty_image() {
        let mut rt = make_runtime();
        rt.image = String::new();
        let result = merge_docker_config("/data/models/test", "test", Some(&rt), None);
        assert!(result.is_err());
    }

    #[test]
    fn merge_computes_container_model_path_from_host_path() {
        let merged = merge_docker_config(
            "/data/models/qwen3-0.6b",
            "qwen3-0.6b",
            Some(&make_runtime()),
            Some(&DockerInstanceOverrides::default()),
        )
        .unwrap();
        assert_eq!(merged.vllm.model, "/models/qwen3-0.6b");
    }

    #[test]
    fn merge_instance_override_container_path_takes_priority() {
        let mut ov = make_overrides();
        ov.model_container_path = "/custom/path".to_string();
        let merged = merge_docker_config(
            "/data/models/qwen3-0.6b",
            "qwen",
            Some(&make_runtime()),
            Some(&ov),
        )
        .unwrap();
        assert_eq!(merged.vllm.model, "/custom/path");
    }

    #[test]
    fn old_full_docker_payload_still_works() {
        let payload = DockerPayload {
            docker: DockerInstanceParams {
                image: "vllm/vllm-openai:latest".to_string(),
                container_name: "legacy-test".to_string(),
                gpu: "all".to_string(),
                ipc: "host".to_string(),
                ports: vec![DockerPortMapping {
                    host: 18000,
                    container: 8000,
                }],
                volumes: vec![DockerVolumeMapping {
                    host: "/data/models/test".to_string(),
                    container: "/models/test".to_string(),
                    readonly: true,
                }],
                ..Default::default()
            },
            vllm: VllmParams {
                model: "/models/test".to_string(),
                served_model_name: "test-model".to_string(),
                host: "0.0.0.0".to_string(),
                port: 8000,
                ..Default::default()
            },
        };
        let args = build_docker_run_args(&payload).unwrap();
        assert!(args.contains(&"--name".to_string()));
        assert!(args.contains(&"legacy-test".to_string()));
    }

    #[test]
    fn merge_extra_args_combine_runtime_and_instance() {
        let mut rt = make_runtime();
        rt.extra_docker_args = vec!["--shm-size=2g".to_string()];
        let mut ov = make_overrides();
        ov.extra_docker_args = vec!["--ulimit".to_string(), "nofile=65536".to_string()];
        let merged =
            merge_docker_config("/data/models/test", "test", Some(&rt), Some(&ov)).unwrap();
        let args = build_docker_run_args(&merged).unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("--shm-size=2g"));
        assert!(joined.contains("--ulimit"));
        assert!(joined.contains("nofile=65536"));
    }
}
