use std::fs;

use lightai_agent::platform_log::{self, LogPolicy};
use lightai_agent::tasks;

#[tokio::test]
async fn verifies_existing_regular_model_file() {
    let path = unique_temp_path("model.gguf");
    fs::write(&path, b"model").unwrap();

    let result = tasks::verify_model_file(path.to_str().unwrap()).await;

    assert_eq!(result.file_status, "verified");
    assert_eq!(result.size_bytes, Some(5));
    assert_eq!(result.message, "file verified");

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn reports_missing_model_file() {
    let path = unique_temp_path("missing.gguf");

    let result = tasks::verify_model_file(path.to_str().unwrap()).await;

    assert_eq!(result.file_status, "missing");
    assert_eq!(result.size_bytes, None);
    assert_eq!(result.message, "path does not exist");
}

#[tokio::test]
async fn verifies_existing_model_directory() {
    let path = unique_temp_path("hf-model-dir");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("config.json"), b"{}").unwrap();

    let result = tasks::verify_model_file(path.to_str().unwrap()).await;

    assert_eq!(result.file_status, "verified");
    assert_eq!(result.path_type.as_deref(), Some("directory"));
    assert_eq!(result.size_bytes, None);
    assert_eq!(result.message, "directory verified");

    let _ = fs::remove_file(path.join("config.json"));
    let _ = fs::remove_dir(path);
}

#[tokio::test]
async fn runtime_check_reports_version_unavailable_separately() {
    let path = unique_temp_path("llama-server");
    fs::write(&path, b"#!/bin/sh\nexit 2\n").unwrap();
    make_executable(&path);

    let result = tasks::check_runtime_environment(&serde_json::json!({
        "backend": "llama_cpp",
        "deploy_type": "binary",
        "binary_path": path.to_str().unwrap()
    }))
    .await;

    assert_eq!(result.check_status, "version_unavailable");
    assert!(result.message.contains("version could not"));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn runtime_check_reports_not_executable_entrypoint() {
    let path = unique_temp_path("not-executable");
    fs::write(&path, b"binary").unwrap();

    let result = tasks::check_runtime_environment(&serde_json::json!({
        "backend": "llama_cpp",
        "deploy_type": "binary",
        "binary_path": path.to_str().unwrap()
    }))
    .await;

    assert_eq!(result.check_status, "not_executable");
    assert_eq!(result.message, "entrypoint file is not executable");

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn script_instance_starts_with_argv_and_can_be_stopped() {
    let script = unique_temp_path("run-script");
    fs::write(&script, b"#!/bin/sh\nsleep 30\n").unwrap();
    make_executable(&script);
    let model = unique_temp_path("script-model.gguf");
    fs::write(&model, b"model").unwrap();

    let payload = serde_json::json!({
        "instance_id": "instance-script",
        "backend": "custom",
        "deploy_type": "script",
        "binary_path": script.to_str().unwrap(),
        "model_path": model.to_str().unwrap(),
        "params_json": {
            "host": "127.0.0.1",
            "port": 19091,
            "extra_args": ["--flag", "value"]
        }
    });
    let started = tasks::start_model_instance(&payload).await;

    assert_eq!(started.instance_status, "running");
    assert_eq!(started.base_url.as_deref(), Some("http://127.0.0.1:19091"));
    assert!(started.process_id.is_some());

    let stopped = tasks::stop_model_instance(&serde_json::json!({
        "instance_id": "instance-script"
    }))
    .await;

    assert_eq!(stopped.instance_status, "stopped");

    let _ = fs::remove_file(model);
    let _ = fs::remove_file(script);
}

#[tokio::test]
async fn managed_instance_start_persists_process_reference_for_restart_recovery() {
    let script = unique_temp_path("managed-run-script");
    fs::write(&script, b"#!/bin/sh\nsleep 30\n").unwrap();
    make_executable(&script);
    let model = unique_temp_path("managed-model.gguf");
    fs::write(&model, b"model").unwrap();
    let store_path = unique_temp_path("managed-processes.json");

    let payload = serde_json::json!({
        "instance_id": "instance-managed",
        "backend": "custom",
        "deploy_type": "script",
        "binary_path": script.to_str().unwrap(),
        "model_path": model.to_str().unwrap(),
        "params_json": {
            "host": "127.0.0.1",
            "port": 19094
        }
    });
    let started = tasks::start_model_instance_with_store(&payload, Some(&store_path)).await;

    assert_eq!(started.instance_status, "running");
    assert_eq!(started.process_ref.as_deref(), Some("instance-managed"));
    assert!(store_path.exists());

    let reports = tasks::collect_managed_instance_reports(Some(&store_path)).await;
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].instance_id, "instance-managed");
    assert_eq!(reports[0].status, "running");
    assert_eq!(
        reports[0].base_url.as_deref(),
        Some("http://127.0.0.1:19094")
    );
    assert!(reports[0].message.contains("still running"));

    let stopped = tasks::stop_model_instance_with_store(
        &serde_json::json!({
            "instance_id": "instance-managed"
        }),
        Some(&store_path),
    )
    .await;
    assert_eq!(stopped.instance_status, "stopped");

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(model);
    let _ = fs::remove_file(script);
}

#[tokio::test]
async fn stop_refuses_unknown_instance_after_agent_restart() {
    let store_path = unique_temp_path("empty-managed-processes.json");

    let stopped = tasks::stop_model_instance_with_store(
        &serde_json::json!({
            "instance_id": "unknown-instance"
        }),
        Some(&store_path),
    )
    .await;

    assert_eq!(stopped.instance_status, "failed");
    assert!(stopped
        .message
        .contains("no platform managed process record found"));
    assert!(stopped.message.contains("refusing to stop"));

    let _ = fs::remove_file(store_path);
}

#[tokio::test]
async fn platform_log_rotates_and_sanitizes_sensitive_lines() {
    let log_dir = unique_temp_path("agent-platform-logs");
    let policy = LogPolicy {
        log_dir: log_dir.to_string_lossy().to_string(),
        log_level: "debug".to_string(),
        log_max_file_bytes: 80,
        log_retention_files: 2,
        log_retention_days: 7,
    };

    platform_log::append(&policy, "agent.log", "info", "first line")
        .await
        .unwrap();
    platform_log::append(&policy, "agent.log", "info", "password=secret")
        .await
        .unwrap();
    platform_log::append(
        &policy,
        "agent.log",
        "info",
        "third line that forces rotation",
    )
    .await
    .unwrap();

    let content = platform_log::read_tail(&policy, "agent.log", 4096)
        .await
        .unwrap();
    assert!(!content.contains("password=secret"));
    assert!(content.contains("third line"));
    assert!(log_dir.join("agent.log.1").exists());

    let _ = fs::remove_file(log_dir.join("agent.log"));
    let _ = fs::remove_file(log_dir.join("agent.log.1"));
    let _ = fs::remove_dir(log_dir);
}

#[tokio::test]
async fn failed_instance_start_returns_stderr_and_command_summary() {
    let script = unique_temp_path("failing-llama-server");
    fs::write(
        &script,
        b"#!/bin/sh\necho 'main: exiting due to HTTP server error' >&2\nexit 1\n",
    )
    .unwrap();
    make_executable(&script);
    let model = unique_temp_path("failing-model.gguf");
    fs::write(&model, b"model").unwrap();

    let result = tasks::start_model_instance(&serde_json::json!({
        "instance_id": "instance-fail",
        "backend": "llama_cpp",
        "deploy_type": "binary",
        "binary_path": script.to_str().unwrap(),
        "model_path": model.to_str().unwrap(),
        "params_json": {
            "host": "127.0.0.1",
            "port": 19092
        }
    }))
    .await;

    assert_eq!(result.instance_status, "failed");
    assert!(result
        .message
        .contains("main: exiting due to HTTP server error"));
    assert!(result
        .log_tail
        .as_deref()
        .unwrap()
        .contains("main: exiting due to HTTP server error"));
    assert!(result
        .command
        .as_deref()
        .unwrap()
        .contains("failing-llama-server"));
    assert!(!result.command.as_deref().unwrap().contains("sh -c"));

    let _ = fs::remove_file(model);
    let _ = fs::remove_file(script);
}

#[tokio::test]
async fn failed_instance_start_writes_stderr_to_controlled_log_dir() {
    let script = unique_temp_path("failing-with-log-dir");
    fs::write(
        &script,
        b"#!/bin/sh\necho 'HTTP server error: port already in use' >&2\nexit 1\n",
    )
    .unwrap();
    make_executable(&script);
    let model = unique_temp_path("logged-model.gguf");
    fs::write(&model, b"model").unwrap();
    let log_dir = unique_temp_path("instance-logs");

    let result = tasks::start_model_instance(&serde_json::json!({
        "instance_id": "instance-log-file",
        "backend": "llama_cpp",
        "deploy_type": "binary",
        "binary_path": script.to_str().unwrap(),
        "model_path": model.to_str().unwrap(),
        "log_dir": log_dir.to_str().unwrap(),
        "params_json": {
            "host": "127.0.0.1",
            "port": 19093
        }
    }))
    .await;

    assert_eq!(result.instance_status, "failed");
    assert!(result
        .log_tail
        .as_deref()
        .unwrap()
        .contains("port already in use"));
    let log_file = log_dir.join("instance-log-file.log");
    let content = fs::read_to_string(&log_file).unwrap();
    assert!(content.contains("stderr:"));
    assert!(content.contains("port already in use"));

    let _ = fs::remove_file(log_file);
    let _ = fs::remove_dir(log_dir);
    let _ = fs::remove_file(model);
    let _ = fs::remove_file(script);
}

#[test]
fn llama_cpp_test_probe_urls_prioritize_openai_models_endpoint() {
    let urls = tasks::build_test_urls("llama_cpp", "http://127.0.0.1:18080").unwrap();

    assert_eq!(urls[0], "http://127.0.0.1:18080/v1/models");
    assert!(urls.contains(&"http://127.0.0.1:18080/health".to_string()));
}

#[test]
fn model_test_404_summary_explains_missing_compatible_endpoint() {
    let urls = tasks::build_test_urls("llama_cpp", "http://127.0.0.1:18080").unwrap();
    let failures = urls
        .iter()
        .map(|url| format!("{url} -> HTTP 404 Not Found"))
        .collect::<Vec<_>>();

    let message = tasks::summarize_test_failures(&urls, &failures);

    assert!(message.contains("No available test endpoint found"));
    assert!(message.contains("/v1/models"));
    assert!(message.contains("endpoint_url"));
}

#[tokio::test]
async fn llama_cpp_version_detection_ignores_cuda_initialization_noise() {
    let script = unique_temp_path("noisy-llama-server");
    fs::write(
        &script,
        b"#!/bin/sh\necho 'ggml_cuda_init: found 1 CUDA devices'\necho 'main: build = 4000'\n",
    )
    .unwrap();
    make_executable(&script);

    let result = tasks::check_runtime_environment(&serde_json::json!({
        "backend": "llama_cpp",
        "deploy_type": "binary",
        "binary_path": script.to_str().unwrap()
    }))
    .await;

    assert_eq!(result.check_status, "version_unavailable");
    assert!(result.message.contains("version could not"));

    let _ = fs::remove_file(script);
}

#[tokio::test]
async fn rejects_path_traversal_marker() {
    let result = tasks::verify_model_file("/models/../secret.gguf").await;

    assert_eq!(result.file_status, "invalid_path");
    assert_eq!(result.message, "invalid path");
}

#[tokio::test]
async fn deletes_existing_file_inside_allowed_model_dir() {
    let dir = unique_temp_path("allowed-dir");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("model.gguf");
    fs::write(&path, b"model").unwrap();

    let result =
        tasks::cleanup_model_file(path.to_str().unwrap(), &[dir.to_string_lossy().to_string()])
            .await;

    assert_eq!(result.cleanup_status, "deleted");
    assert_eq!(result.message, "file cleaned up");
    assert!(!path.exists());

    let _ = fs::remove_dir(dir);
}

#[tokio::test]
async fn cleanup_rejects_missing_file_and_keeps_recordable_error() {
    let dir = unique_temp_path("allowed-missing");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("missing.gguf");

    let result =
        tasks::cleanup_model_file(path.to_str().unwrap(), &[dir.to_string_lossy().to_string()])
            .await;

    assert_eq!(result.cleanup_status, "failed");
    assert_eq!(result.message, "file does not exist");

    let _ = fs::remove_dir(dir);
}

#[tokio::test]
async fn cleanup_distinguishes_missing_allowed_model_dir() {
    let missing_dir = unique_temp_path("missing-allowed-dir");
    let target = missing_dir.join("model.gguf");

    let result = tasks::cleanup_model_file(
        target.to_str().unwrap(),
        &[missing_dir.to_string_lossy().to_string()],
    )
    .await;

    assert_eq!(result.cleanup_status, "failed");
    assert_eq!(result.message, "allowed model directory does not exist");
}

#[tokio::test]
async fn cleanup_distinguishes_invalid_allowed_model_dir_config() {
    let target = unique_temp_path("invalid-allowed-target.gguf");

    let result =
        tasks::cleanup_model_file(target.to_str().unwrap(), &["relative/models".to_string()]).await;

    assert_eq!(result.cleanup_status, "failed");
    assert_eq!(result.message, "allowed model directory config is invalid");
}

#[tokio::test]
async fn cleanup_rejects_directory() {
    let dir = unique_temp_path("allowed-directory");
    let model_dir = dir.join("model-dir");
    fs::create_dir_all(&model_dir).unwrap();

    let result = tasks::cleanup_model_file(
        model_dir.to_str().unwrap(),
        &[dir.to_string_lossy().to_string()],
    )
    .await;

    assert_eq!(result.cleanup_status, "failed");
    assert_eq!(
        result.message,
        "refusing to delete directory or non-regular file"
    );

    let _ = fs::remove_dir(model_dir);
    let _ = fs::remove_dir(dir);
}

#[tokio::test]
async fn cleanup_rejects_path_traversal() {
    let dir = unique_temp_path("allowed-traversal");
    fs::create_dir_all(&dir).unwrap();

    let result = tasks::cleanup_model_file(
        "/models/../secret.gguf",
        &[dir.to_string_lossy().to_string()],
    )
    .await;

    assert_eq!(result.cleanup_status, "failed");
    assert_eq!(result.message, "invalid path");

    let _ = fs::remove_dir(dir);
}

#[tokio::test]
async fn cleanup_rejects_file_outside_allowed_model_dirs() {
    let allowed = unique_temp_path("allowed-outside");
    let outside = unique_temp_path("outside-model.gguf");
    fs::create_dir_all(&allowed).unwrap();
    fs::write(&outside, b"model").unwrap();

    let result = tasks::cleanup_model_file(
        outside.to_str().unwrap(),
        &[allowed.to_string_lossy().to_string()],
    )
    .await;

    assert_eq!(result.cleanup_status, "failed");
    assert_eq!(result.message, "file not within allowed model directory");
    assert!(outside.exists());

    let _ = fs::remove_file(outside);
    let _ = fs::remove_dir(allowed);
}

#[tokio::test]
async fn cleanup_rejects_without_allowed_model_dirs() {
    let path = unique_temp_path("no-allowed.gguf");
    fs::write(&path, b"model").unwrap();

    let result = tasks::cleanup_model_file(path.to_str().unwrap(), &[]).await;

    assert_eq!(result.cleanup_status, "failed");
    assert_eq!(
        result.message,
        "no allowed model directory configured; refusing to delete file"
    );
    assert!(path.exists());

    let _ = fs::remove_file(path);
}

fn unique_temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "lightai-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &std::path::Path) {}
