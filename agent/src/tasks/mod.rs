use std::path::{Component, Path};
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

mod cleanup;
pub(crate) mod docker_backend;
mod logs;
pub(crate) mod ollama;
mod probe;
mod process;
mod process_command;
mod process_logs;
mod result;
mod runtime_check;
mod verify_model;

use crate::client::ServerClient;
use crate::config::Config;
use crate::heartbeat::RuntimeConfig;
use crate::managed_process;
use crate::models::{AgentConfig, AgentTaskPollRequest, AgentTaskResultRequest};
use crate::platform_log::{self, LogPolicy};
use crate::state::{self, AgentState};
pub use cleanup::cleanup_model_file;
use logs::{read_instance_log, ReadInstanceLogResult};
pub use probe::{build_test_urls, summarize_test_failures};
pub use process::{
    collect_managed_instance_reports, start_model_instance, start_model_instance_with_store,
    stop_model_instance, stop_model_instance_with_store,
};
use process::{is_custom_script, run_controlled_script_action};
pub(super) use result::{instance_failure, instance_failure_with_details};
pub use result::{
    CleanupModelFileResult, ModelInstanceTaskResult, RuntimeEnvironmentCheckResult,
    VerifyModelFileResult,
};
pub use runtime_check::check_runtime_environment;
pub(crate) use runtime_check::verify_controlled_entrypoint;
pub use verify_model::{verify_model_file, verify_model_file_with_hint};

pub async fn run(config: Config, runtime_config: Arc<RwLock<RuntimeConfig>>) {
    let client = ServerClient::new(
        config.server_url.clone(),
        config.ca_cert_path.as_deref(),
        config.insecure_skip_tls_verify,
    )
    .expect("TLS client init failed");
    loop {
        let sleep_secs = match state::load(&config.state_path).await {
            Ok(Some(agent_state)) => {
                let allowed_model_dirs = runtime_config.read().await.allowed_model_dirs.clone();
                let snapshot = runtime_config.read().await.clone();
                let current_config_version = snapshot.config_version;
                let log_policy = snapshot.log_policy;
                let managed_store_path =
                    managed_process::store_path_from_state_path(&config.state_path);
                match run_once(
                    &client,
                    &agent_state.agent_token,
                    &agent_state,
                    &allowed_model_dirs,
                    current_config_version,
                    Some(&managed_store_path),
                    &log_policy,
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
    managed_store_path: Option<&Path>,
    log_policy: &LogPolicy,
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

    let _ = platform_log::append(
        log_policy,
        "agent.log",
        "info",
        &format!("Agent starting task task_id={} kind={}", task.id, task.kind),
    )
    .await;

    let (status, result) = match task.kind.as_str() {
        "verify_model_file" => {
            let path = task
                .payload
                .get("path")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let path_type_hint = task
                .payload
                .get("path_type")
                .and_then(|value| value.as_str());
            let result = verify_model_file_with_hint(path, path_type_hint).await;
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
        "read_agent_log" => {
            let max_bytes = task
                .payload
                .get("max_bytes")
                .and_then(|value| value.as_u64())
                .unwrap_or(64 * 1024)
                .min(512 * 1024) as usize;
            let content = platform_log::read_tail(log_policy, "agent.log", max_bytes).await;
            match content {
                Ok(content) => (
                    "succeeded".to_string(),
                    serde_json::json!({
                        "log_status": "available",
                        "content": content,
                        "message": "Agent log read succeeded"
                    }),
                ),
                Err(error) => (
                    "failed".to_string(),
                    serde_json::json!({
                        "log_status": "failed",
                        "content": "",
                        "message": format!("Agent log read failed: {error}")
                    }),
                ),
            }
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
            let result = start_model_instance_with_store(&task.payload, managed_store_path).await;
            let status = if result.instance_status == "running" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "stop_model_instance" => {
            let backend = task
                .payload
                .get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = if backend == "ollama" {
                ollama::stop_ollama_instance(&task.payload).await
            } else {
                stop_model_instance_with_store(&task.payload, managed_store_path).await
            };
            let status = if result.instance_status == "stopped" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "check_model_instance" => {
            let backend = task
                .payload
                .get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = if backend == "ollama" {
                ollama::check_ollama_instance(&task.payload).await
            } else {
                test_model_instance(&task.payload).await
            };
            let status = if result.instance_status == "running" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "test_model_instance" => {
            let backend = task
                .payload
                .get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = if backend == "ollama" {
                ollama::test_ollama_instance(&task.payload).await
            } else {
                test_model_instance(&task.payload).await
            };
            let status = if result.instance_status == "running" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "read_instance_log" => {
            let backend = task
                .payload
                .get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = if backend == "ollama" {
                let rt_env_id = task
                    .payload
                    .get("runtime_environment_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let log_dir = task
                    .payload
                    .get("log_dir")
                    .and_then(|v| v.as_str())
                    .unwrap_or("logs");
                let max_bytes = 65536usize;
                match ollama::read_daemon_log(rt_env_id, log_dir, max_bytes).await {
                    Ok(content) => ReadInstanceLogResult {
                        log_status: "available".to_string(),
                        content: format!(
                            "【Ollama 使用共享 daemon，日志为 Ollama 服务日志，不是单模型独立日志】\n\n{content}"
                        ),
                        message: "read from ollama daemon log".to_string(),
                    },
                    Err(e) => ReadInstanceLogResult {
                        log_status: "failed".to_string(),
                        content: String::new(),
                        message: e,
                    },
                }
            } else {
                read_instance_log(&task.payload, managed_store_path).await
            };
            let status = if result.log_status == "available" {
                "succeeded"
            } else {
                "failed"
            };
            (status.to_string(), serde_json::to_value(result)?)
        }
        "list_ollama_models" => {
            let result = ollama::query_model_list(&task.payload).await;
            match result {
                Ok(models) => (
                    "succeeded".to_string(),
                    serde_json::to_value(serde_json::json!({
                        "models": models,
                    }))?,
                ),
                Err(e) => ("failed".to_string(), serde_json::json!({"message": e})),
            }
        }
        _ => (
            "failed".to_string(),
            serde_json::json!({
                "cleanup_status": "failed",
                "message": "unknown task type"
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

pub async fn test_model_instance(payload: &serde_json::Value) -> ModelInstanceTaskResult {
    if is_custom_script(payload) {
        return run_controlled_script_action(payload, "test", "running").await;
    }
    let Some(url) = payload
        .get("endpoint_url")
        .and_then(|value| value.as_str())
        .or_else(|| payload.get("base_url").and_then(|value| value.as_str()))
    else {
        return instance_failure("instance missing test URL");
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
        Err(error) => {
            return instance_failure(&format!("test client initialization failed: {error}"))
        }
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
                        message: format!("test succeeded: HTTP {status} {url}"),
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
            Err(error) => failures.push(format!("{url} -> request failed: {error}")),
        }
    }
    instance_failure(&summarize_test_failures(&urls, &failures))
}

fn has_parent_dir(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}
