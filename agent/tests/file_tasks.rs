use std::fs;

use lightai_agent::tasks;

#[tokio::test]
async fn verifies_existing_regular_model_file() {
    let path = unique_temp_path("model.gguf");
    fs::write(&path, b"model").unwrap();

    let result = tasks::verify_model_file(path.to_str().unwrap()).await;

    assert_eq!(result.file_status, "verified");
    assert_eq!(result.size_bytes, Some(5));
    assert_eq!(result.message, "文件已验证");

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn reports_missing_model_file() {
    let path = unique_temp_path("missing.gguf");

    let result = tasks::verify_model_file(path.to_str().unwrap()).await;

    assert_eq!(result.file_status, "missing");
    assert_eq!(result.size_bytes, None);
    assert_eq!(result.message, "文件不存在");
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
    assert_eq!(result.message, "目录已验证");

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
    assert!(result.message.contains("版本无法自动获取"));

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
    assert_eq!(result.message, "入口文件不可执行");

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
        "params": {
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
async fn rejects_path_traversal_marker() {
    let result = tasks::verify_model_file("/models/../secret.gguf").await;

    assert_eq!(result.file_status, "invalid_path");
    assert_eq!(result.message, "路径非法");
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
    assert_eq!(result.message, "文件已清理");
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
    assert_eq!(result.message, "文件不存在");

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
    assert_eq!(result.message, "受控模型目录不存在");
}

#[tokio::test]
async fn cleanup_distinguishes_invalid_allowed_model_dir_config() {
    let target = unique_temp_path("invalid-allowed-target.gguf");

    let result =
        tasks::cleanup_model_file(target.to_str().unwrap(), &["relative/models".to_string()]).await;

    assert_eq!(result.cleanup_status, "failed");
    assert_eq!(result.message, "受控模型目录配置非法");
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
    assert_eq!(result.message, "拒绝删除目录或非普通文件");

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
    assert_eq!(result.message, "路径非法");

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
    assert_eq!(result.message, "文件不在受控模型目录内");
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
    assert_eq!(result.message, "未配置受控模型目录，拒绝删除文件");
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
