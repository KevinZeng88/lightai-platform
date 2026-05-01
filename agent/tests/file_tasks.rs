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
async fn rejects_directory_as_model_file() {
    let path = unique_temp_path("model-dir");
    fs::create_dir_all(&path).unwrap();

    let result = tasks::verify_model_file(path.to_str().unwrap()).await;

    assert_eq!(result.file_status, "not_file");
    assert_eq!(result.message, "路径不是普通文件");

    let _ = fs::remove_dir(path);
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
