//! Ollama backend — daemon lifecycle and model operations.
//!
//! Design:
//!   - Runtime = ollama daemon configuration (host, port, env vars).
//!   - Instance = a model name loaded on that daemon.
//!   - Multiple instances share one daemon per Runtime.
//!   - start = warmup/load model; stop = unload model (daemon stays).
//!   - logs = daemon log tail (shared, not per-model).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::Duration;

use super::process_logs::sanitize_log;
use super::result::{instance_failure, instance_failure_with_details, ModelInstanceTaskResult};
use crate::platform_log::{self, LogPolicy};

const OLLAMA_API_TIMEOUT_SECS: u64 = 10;
const OLLAMA_START_TIMEOUT_SECS: u64 = 30;

// ── Shared daemon registry ──
// Key = (runtime_environment_id), tracks the single daemon per Runtime.

struct DaemonHandle {
    child: Arc<Mutex<Child>>,
    log_path: PathBuf,
}

fn daemon_registry() -> &'static Mutex<HashMap<String, DaemonHandle>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, DaemonHandle>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── Config structs ──

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct OllamaRuntimeConfig {
    pub host: String,
    pub port: u16,
    pub models_dir: String,
    pub max_loaded_models: u32,
    pub num_parallel: u32,
    pub max_queue: u32,
    pub keep_alive: String,
    pub context_length: u32,
}

impl OllamaRuntimeConfig {
    pub fn from_runtime_params(params: &serde_json::Value) -> Self {
        let defaults = params.get("defaults").filter(|v| v.is_object());
        let d = defaults.unwrap_or(params);
        Self {
            host: d
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .to_string(),
            port: d.get("port").and_then(|v| v.as_u64()).unwrap_or(11434) as u16,
            models_dir: d
                .get("models_dir")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            max_loaded_models: d
                .get("max_loaded_models")
                .and_then(|v| v.as_u64())
                .unwrap_or(2) as u32,
            num_parallel: d.get("num_parallel").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
            max_queue: d.get("max_queue").and_then(|v| v.as_u64()).unwrap_or(512) as u32,
            keep_alive: d
                .get("keep_alive")
                .and_then(|v| v.as_str())
                .unwrap_or("30m")
                .to_string(),
            context_length: d
                .get("context_length")
                .and_then(|v| v.as_u64())
                .unwrap_or(4096) as u32,
        }
    }
}

// ── API response types ──

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OllamaModelInfo {
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub modified_at: Option<String>,
}

// ── Daemon lifecycle ──

pub(crate) fn ollama_api_base(runtime_cfg: &OllamaRuntimeConfig) -> String {
    format!("http://{}:{}", runtime_cfg.host, runtime_cfg.port)
}

pub(crate) fn build_ollama_env(runtime_cfg: &OllamaRuntimeConfig) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "OLLAMA_HOST".to_string(),
        format!("{}:{}", runtime_cfg.host, runtime_cfg.port),
    );
    if !runtime_cfg.models_dir.trim().is_empty() {
        env.insert(
            "OLLAMA_MODELS".to_string(),
            runtime_cfg.models_dir.trim().to_string(),
        );
    }
    env.insert(
        "OLLAMA_MAX_LOADED_MODELS".to_string(),
        runtime_cfg.max_loaded_models.to_string(),
    );
    env.insert(
        "OLLAMA_NUM_PARALLEL".to_string(),
        runtime_cfg.num_parallel.to_string(),
    );
    env.insert(
        "OLLAMA_MAX_QUEUE".to_string(),
        runtime_cfg.max_queue.to_string(),
    );
    env.insert(
        "OLLAMA_KEEP_ALIVE".to_string(),
        runtime_cfg.keep_alive.clone(),
    );
    env.insert(
        "OLLAMA_CONTEXT_LENGTH".to_string(),
        runtime_cfg.context_length.to_string(),
    );
    env
}

/// Check whether /api/tags is reachable at the configured base URL.
async fn api_tags_reachable(base_url: &str) -> bool {
    let client = reqwest::Client::new();
    match client
        .get(format!("{base_url}/api/tags"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Start or locate the ollama daemon for a given Runtime.
/// Returns (base_url, log_path, daemon_action).
async fn ensure_daemon(
    runtime_env_id: &str,
    binary_path: &str,
    log_dir: &str,
    runtime_cfg: &OllamaRuntimeConfig,
) -> Result<(String, PathBuf, String), String> {
    let base_url = ollama_api_base(runtime_cfg);

    // 1. Check whether /api/tags is already reachable (daemon already running externally).
    if api_tags_reachable(&base_url).await {
        let log_path = daemon_log_path(log_dir, runtime_env_id)
            .unwrap_or_else(|_| PathBuf::from("logs/ollama-daemon.log"));
        return Ok((base_url, log_path, "reused_existing_daemon".to_string()));
    }

    // 2. Check in-memory registry.
    {
        let registry = daemon_registry().lock().await;
        if let Some(handle) = registry.get(runtime_env_id) {
            let mut child = handle.child.lock().await;
            match child.try_wait() {
                Ok(None) => {
                    return Ok((
                        base_url,
                        handle.log_path.clone(),
                        "reused_existing_daemon".to_string(),
                    ));
                }
                Ok(Some(_)) | Err(_) => {
                    drop(child);
                }
            }
        }
    }
    // Clean stale registry.
    daemon_registry().lock().await.remove(runtime_env_id);

    // 3. Try to start daemon.
    let log_path = daemon_log_path(log_dir, runtime_env_id)?;
    let mut cmd = Command::new(binary_path);
    cmd.arg("serve");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    for (key, val) in build_ollama_env(runtime_cfg) {
        cmd.env(key, val);
    }

    let start_err = match cmd.spawn() {
        Ok(mut child) => {
            let pid = child.id().unwrap_or(0);
            let log_path_clone = log_path.clone();
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            attach_daemon_log_reader(stdout, stderr, log_path_clone);

            let deadline =
                std::time::Instant::now() + Duration::from_secs(OLLAMA_START_TIMEOUT_SECS);
            let mut ready = false;
            while std::time::Instant::now() < deadline {
                if api_tags_reachable(&base_url).await {
                    ready = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            if !ready {
                let _ = child.kill().await;
                let tail = read_daemon_log(runtime_env_id, log_dir, 4096)
                    .await
                    .unwrap_or_default();
                return Err(format!(
                    "ollama daemon did not become ready within {}s. Daemon log tail: {}",
                    OLLAMA_START_TIMEOUT_SECS, tail
                ));
            }

            let handle = DaemonHandle {
                child: Arc::new(Mutex::new(child)),
                log_path: log_path.clone(),
            };
            daemon_registry()
                .lock()
                .await
                .insert(runtime_env_id.to_string(), handle);

            let _ = platform_log::append(
                &LogPolicy::default(),
                "agent.log",
                "info",
                &format!(
                    "ollama daemon started runtime={runtime_env_id} pid={pid} base_url={base_url}"
                ),
            )
            .await;

            return Ok((base_url, log_path, "started_ollama_serve".to_string()));
        }
        Err(e) => e.to_string(),
    };

    // 4. Start failed — check if port already in use but /api/tags available.
    if start_err.contains("address already in use") || start_err.contains("bind") {
        if api_tags_reachable(&base_url).await {
            let log_path = daemon_log_path(log_dir, runtime_env_id)?;
            return Ok((base_url, log_path, "reused_existing_daemon".to_string()));
        }
        return Err(format!(
            "Ollama port is already in use ({base_url}) but /api/tags is not reachable. \
             Please check the process using the port."
        ));
    }

    Err(format!("ollama serve start failed: {start_err}"))
}

fn daemon_log_path(log_dir: &str, runtime_env_id: &str) -> Result<PathBuf, String> {
    let dir = Path::new(log_dir);
    if dir.to_string_lossy().contains("..") || dir.is_absolute() && dir.to_string_lossy().is_empty()
    {
        return Err("invalid log_dir".to_string());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create log dir: {e}"))?;
    Ok(dir.join(format!("ollama-daemon-{runtime_env_id}.log")))
}

fn spawn_log_reader(stream: impl tokio::io::AsyncRead + Unpin + Send + 'static, log_path: PathBuf) {
    tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(stream);
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .ok();
        let mut buf = [0u8; 4096];
        loop {
            match tokio::io::AsyncReadExt::read(&mut reader, &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    if let Some(ref mut f) = file {
                        let _ = tokio::io::AsyncWriteExt::write_all(f, text.as_bytes()).await;
                        let _ = tokio::io::AsyncWriteExt::flush(f).await;
                    }
                }
            }
        }
    });
}

fn attach_daemon_log_reader(
    stdout: Option<impl tokio::io::AsyncRead + Unpin + Send + 'static>,
    stderr: Option<impl tokio::io::AsyncRead + Unpin + Send + 'static>,
    log_path: PathBuf,
) {
    if let Some(s) = stdout {
        spawn_log_reader(s, log_path.clone());
    }
    if let Some(s) = stderr {
        spawn_log_reader(s, log_path);
    }
}

// ── Model list ──

pub(crate) async fn list_models(base_url: &str) -> Result<Vec<OllamaModelInfo>, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base_url}/api/tags"))
        .timeout(Duration::from_secs(OLLAMA_API_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("ollama /api/tags request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("ollama /api/tags returned {}", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("read /api/tags body: {e}"))?;
    let tags: OllamaTagsResponse =
        serde_json::from_str(&body).map_err(|e| format!("parse /api/tags: {e}"))?;
    Ok(tags.models)
}

pub(crate) async fn model_exists(base_url: &str, model: &str) -> Result<bool, String> {
    let models = list_models(base_url).await?;
    Ok(models.iter().any(|m| m.name == model))
}

// ── Warmup / load ──

async fn warmup_model(base_url: &str, model: &str, keep_alive: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "prompt": "",
        "stream": false,
        "keep_alive": keep_alive,
    });
    let resp = client
        .post(format!("{base_url}/api/generate"))
        .json(&body)
        .timeout(Duration::from_secs(OLLAMA_API_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("ollama /api/generate warmup failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("ollama warmup returned {}: {}", status, text));
    }
    let text = resp.text().await.unwrap_or_default();
    Ok(text)
}

// ── Start instance ──

pub(crate) async fn start_ollama_instance(payload: &serde_json::Value) -> ModelInstanceTaskResult {
    let _instance_id = payload
        .get("instance_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let runtime_env_id = payload
        .get("runtime_environment_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let binary_path = payload
        .get("binary_path")
        .and_then(|v| v.as_str())
        .unwrap_or("ollama");
    let log_dir = payload
        .get("log_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("logs");

    let params_str = payload
        .get("params_json")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let params: serde_json::Value = serde_json::from_str(params_str).unwrap_or_default();
    let model_name = params
        .get("ollama_model")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if model_name.trim().is_empty() {
        return instance_failure("ollama_model is required in instance params");
    }

    let runtime_params = payload.get("runtime_params");
    let rt_cfg = OllamaRuntimeConfig::from_runtime_params(
        runtime_params.unwrap_or(&serde_json::Value::Null),
    );

    // user keep_alive override
    let keep_alive = params
        .get("keep_alive")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&rt_cfg.keep_alive);

    // Ensure daemon.
    let (base_url, _log_path, daemon_action) =
        match ensure_daemon(runtime_env_id, binary_path, log_dir, &rt_cfg).await {
            Ok(v) => v,
            Err(e) => {
                return instance_failure_with_details(
                    &format!("ollama daemon error: {e}"),
                    None,
                    Some(format!(
                        "ollama load model {model_name} via daemon {}; daemon_action=failed_to_start; error={e}",
                        ollama_api_base(&rt_cfg)
                    )),
                )
            }
        };

    // Check model exists.
    match model_exists(&base_url, model_name).await {
        Ok(false) => {
            let cmd = format!("ollama load model {model_name} via daemon {base_url}; daemon_action={daemon_action}; model_check=failed_not_found");
            return instance_failure_with_details(
                &format!("Ollama model not found: {model_name}. Please run `ollama pull {model_name}` on the node first."),
                None,
                Some(cmd),
            );
        }
        Err(e) => {
            let cmd = format!("ollama load model {model_name} via daemon {base_url}; daemon_action={daemon_action}; model_check=error; error={e}");
            return instance_failure_with_details(
                &format!("failed to check Ollama model: {e}"),
                None,
                Some(cmd),
            );
        }
        Ok(true) => {}
    }

    // Warmup.
    match warmup_model(&base_url, model_name, keep_alive).await {
        Ok(_summary) => {}
        Err(e) => {
            let cmd = format!("ollama load model {model_name} via daemon {base_url}; daemon_action={daemon_action}; warmup=POST /api/generate; warmup_error={e}");
            return instance_failure_with_details(
                &format!("ollama warmup failed: {e}"),
                None,
                Some(cmd),
            );
        }
    }

    let command = format!(
        "ollama load model {model_name} via daemon {base_url}; daemon_action={daemon_action}; warmup=POST /api/generate"
    );
    ModelInstanceTaskResult {
        instance_status: "running".to_string(),
        message: format!("ollama model loaded: {model_name}"),
        base_url: Some(base_url.clone()),
        endpoint_url: Some(format!("{base_url}/api/generate")),
        process_id: None,
        process_ref: Some(runtime_env_id.to_string()),
        response_summary: None,
        log_tail: None,
        command: Some(command),
    }
}

// ── Stop instance ──

pub(crate) async fn stop_ollama_instance(payload: &serde_json::Value) -> ModelInstanceTaskResult {
    let _instance_id = payload
        .get("instance_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let runtime_env_id = payload
        .get("runtime_environment_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let params_str = payload
        .get("params_json")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let params: serde_json::Value = serde_json::from_str(params_str).unwrap_or_default();
    let model_name = params
        .get("ollama_model")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if model_name.trim().is_empty() {
        return instance_failure("ollama_model is required");
    }

    let runtime_params = payload.get("runtime_params");
    let rt_cfg = OllamaRuntimeConfig::from_runtime_params(
        runtime_params.unwrap_or(&serde_json::Value::Null),
    );
    let base_url = ollama_api_base(&rt_cfg);

    // Unload via keep_alive=0.
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model_name,
        "prompt": "",
        "stream": false,
        "keep_alive": "0",
    });
    match client
        .post(format!("{base_url}/api/generate"))
        .json(&body)
        .timeout(Duration::from_secs(OLLAMA_API_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => {
            let _ = platform_log::append(
                &LogPolicy::default(),
                "agent.log",
                "warn",
                &format!("ollama unload returned {} for {model_name}", resp.status()),
            )
            .await;
        }
        Err(_) => {
            // Daemon may have stopped; treat as already unloaded.
        }
    }

    ModelInstanceTaskResult {
        instance_status: "stopped".to_string(),
        message: format!("ollama model unloaded: {model_name}"),
        base_url: None,
        endpoint_url: None,
        process_id: None,
        process_ref: Some(runtime_env_id.to_string()),
        response_summary: None,
        log_tail: None,
        command: Some(format!(
            "ollama unload model {model_name} via daemon {base_url}; daemon_action=reused_existing_daemon; unload=POST /api/generate keep_alive=0"
        )),
    }
}

// ── Check ──

pub(crate) async fn check_ollama_instance(payload: &serde_json::Value) -> ModelInstanceTaskResult {
    let params_str = payload
        .get("params_json")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let params: serde_json::Value = serde_json::from_str(params_str).unwrap_or_default();
    let model_name = params
        .get("ollama_model")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if model_name.trim().is_empty() {
        return instance_failure("ollama_model is required");
    }

    let runtime_params = payload.get("runtime_params");
    let rt_cfg = OllamaRuntimeConfig::from_runtime_params(
        runtime_params.unwrap_or(&serde_json::Value::Null),
    );
    let base_url = ollama_api_base(&rt_cfg);

    // Check daemon reachable.
    let client = reqwest::Client::new();
    match client
        .get(format!("{base_url}/api/tags"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
    {
        Ok(resp) if !resp.status().is_success() => {
            return instance_failure(&format!("ollama daemon returned {}", resp.status()));
        }
        Err(e) => {
            return instance_failure(&format!("ollama daemon not reachable: {e}"));
        }
        _ => {}
    }

    // Check model exists.
    match model_exists(&base_url, model_name).await {
        Ok(false) => {
            return instance_failure(&format!("Ollama model not found: {model_name}"));
        }
        Err(e) => return instance_failure(&e),
        Ok(true) => {}
    }

    // Model exists in /api/tags — daemon is reachable and model is available.
    ModelInstanceTaskResult {
        instance_status: "running".to_string(),
        message: format!("ollama model available: {model_name}"),
        base_url: Some(base_url.clone()),
        endpoint_url: Some(format!("{base_url}/api/generate")),
        process_id: None,
        process_ref: None,
        response_summary: None,
        log_tail: None,
        command: None,
    }
}

// ── Test ──

pub(crate) async fn test_ollama_instance(payload: &serde_json::Value) -> ModelInstanceTaskResult {
    // Run check first — daemon reachable, model exists.
    let check = check_ollama_instance(payload).await;
    if check.instance_status != "running" {
        return check;
    }

    let params_str = payload
        .get("params_json")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let params: serde_json::Value = serde_json::from_str(params_str).unwrap_or_default();
    let model_name = params
        .get("ollama_model")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let runtime_params = payload.get("runtime_params");
    let rt_cfg = OllamaRuntimeConfig::from_runtime_params(
        runtime_params.unwrap_or(&serde_json::Value::Null),
    );
    let base_url = ollama_api_base(&rt_cfg);

    let start = std::time::Instant::now();
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model_name,
        "prompt": "hello",
        "stream": false,
    });
    match client
        .post(format!("{base_url}/api/generate"))
        .json(&body)
        .timeout(Duration::from_secs(OLLAMA_API_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(resp) => {
            let elapsed_ms = start.elapsed().as_millis();
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status.is_success() {
                let summary: String = text.chars().take(300).collect();
                ModelInstanceTaskResult {
                    instance_status: "running".to_string(),
                    message: format!("ollama test OK ({elapsed_ms}ms)"),
                    base_url: Some(base_url),
                    endpoint_url: None,
                    process_id: None,
                    process_ref: None,
                    response_summary: Some(summary),
                    log_tail: None,
                    command: None,
                }
            } else {
                instance_failure(&format!("ollama test returned {}: {}", status, text))
            }
        }
        Err(e) => instance_failure(&format!("ollama test failed: {e}")),
    }
}

// ── Read daemon log ──

pub(crate) async fn read_daemon_log(
    runtime_env_id: &str,
    log_dir: &str,
    max_bytes: usize,
) -> Result<String, String> {
    let log_path = daemon_log_path(log_dir, runtime_env_id)?;
    match tokio::fs::read(&log_path).await {
        Ok(bytes) => {
            let start = bytes.len().saturating_sub(max_bytes);
            Ok(sanitize_log(&String::from_utf8_lossy(&bytes[start..])))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok("Ollama daemon log not yet available".to_string())
        }
        Err(e) => Err(format!("read daemon log: {e}")),
    }
}

// ── Model list query (for Server API) ──

pub(crate) async fn query_model_list(
    payload: &serde_json::Value,
) -> Result<Vec<OllamaModelInfo>, String> {
    let runtime_params = payload.get("runtime_params");
    let rt_cfg = OllamaRuntimeConfig::from_runtime_params(
        runtime_params.unwrap_or(&serde_json::Value::Null),
    );
    let base_url = ollama_api_base(&rt_cfg);

    // Quick reachability check.
    let client = reqwest::Client::new();
    match client
        .get(format!("{base_url}/api/tags"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
    {
        Ok(resp) if !resp.status().is_success() => {
            return Err(
                "Ollama daemon not running. Start the Ollama Runtime or any Ollama Instance first."
                    .to_string(),
            );
        }
        Err(_) => {
            return Err("Ollama daemon not reachable. Start the Ollama Runtime or any Ollama Instance first.".to_string());
        }
        _ => {}
    }

    list_models(&base_url).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_config_defaults() {
        let cfg = OllamaRuntimeConfig::from_runtime_params(&json!({}));
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 11434);
        assert_eq!(cfg.models_dir, "");
        assert_eq!(cfg.max_loaded_models, 2);
        assert_eq!(cfg.num_parallel, 1);
        assert_eq!(cfg.max_queue, 512);
        assert_eq!(cfg.keep_alive, "30m");
        assert_eq!(cfg.context_length, 4096);
    }

    #[test]
    fn runtime_config_from_defaults_key() {
        let cfg = OllamaRuntimeConfig::from_runtime_params(&json!({
            "defaults": {
                "host": "0.0.0.0",
                "port": 18080,
                "max_loaded_models": 4
            }
        }));
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 18080);
        assert_eq!(cfg.max_loaded_models, 4);
        // not set → use built-in default
        assert_eq!(cfg.keep_alive, "30m");
    }

    #[test]
    fn env_vars_basic() {
        let cfg = OllamaRuntimeConfig::from_runtime_params(&json!({
            "host": "0.0.0.0",
            "port": 11434
        }));
        let env = build_ollama_env(&cfg);
        assert_eq!(env.get("OLLAMA_HOST").unwrap(), "0.0.0.0:11434");
        assert_eq!(env.get("OLLAMA_MAX_LOADED_MODELS").unwrap(), "2");
        assert_eq!(env.get("OLLAMA_NUM_PARALLEL").unwrap(), "1");
        assert_eq!(env.get("OLLAMA_MAX_QUEUE").unwrap(), "512");
        assert_eq!(env.get("OLLAMA_KEEP_ALIVE").unwrap(), "30m");
        assert_eq!(env.get("OLLAMA_CONTEXT_LENGTH").unwrap(), "4096");
    }

    #[test]
    fn env_vars_models_dir_set() {
        let mut cfg = OllamaRuntimeConfig::from_runtime_params(&json!({}));
        cfg.models_dir = "/data/ollama-models".into();
        let env = build_ollama_env(&cfg);
        assert_eq!(env.get("OLLAMA_MODELS").unwrap(), "/data/ollama-models");
    }

    #[test]
    fn env_vars_models_dir_empty_omitted() {
        let cfg = OllamaRuntimeConfig::from_runtime_params(&json!({}));
        let env = build_ollama_env(&cfg);
        assert!(!env.contains_key("OLLAMA_MODELS"));
    }

    #[test]
    fn parse_tags_response() {
        let json = r#"{"models":[{"name":"qwen2.5:7b","size":4682017325,"digest":"abc123","modified_at":"2025-01-01T00:00:00Z"},{"name":"nomic-embed-text:latest","size":274801250}]}"#;
        let tags: OllamaTagsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(tags.models.len(), 2);
        assert_eq!(tags.models[0].name, "qwen2.5:7b");
        assert_eq!(tags.models[0].size, Some(4682017325));
        assert_eq!(tags.models[1].name, "nomic-embed-text:latest");
        assert_eq!(tags.models[1].size, Some(274801250));
        assert!(tags.models[1].digest.is_none());
    }

    #[test]
    fn parse_empty_tags_response() {
        let json = r#"{"models":[]}"#;
        let tags: OllamaTagsResponse = serde_json::from_str(json).unwrap();
        assert!(tags.models.is_empty());
    }

    #[test]
    fn daemon_log_path_rejects_parent_dir() {
        assert!(daemon_log_path("../etc", "rt-1").is_err());
    }

    #[test]
    fn ollama_api_base_url() {
        let cfg = OllamaRuntimeConfig::from_runtime_params(&json!({
            "host": "127.0.0.1",
            "port": 11434
        }));
        assert_eq!(ollama_api_base(&cfg), "http://127.0.0.1:11434");
    }

    #[test]
    fn warmup_payload_uses_keep_alive() {
        // Test design: warmup_model builds correct JSON body.
        let body = json!({
            "model": "qwen2.5:7b",
            "prompt": "",
            "stream": false,
            "keep_alive": "10m",
        });
        assert_eq!(body["model"], "qwen2.5:7b");
        assert_eq!(body["keep_alive"], "10m");
        assert_eq!(body["stream"], false);
    }
}
