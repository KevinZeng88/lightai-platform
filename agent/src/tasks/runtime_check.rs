use std::path::Path;

use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::has_parent_dir;
use super::result::{runtime_unavailable, RuntimeEnvironmentCheckResult};

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

pub(crate) async fn verify_controlled_entrypoint(path: &str) -> RuntimeEnvironmentCheckResult {
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

async fn detect_entrypoint_version(path: &str) -> Option<String> {
    let output = timeout(
        Duration::from_secs(3),
        Command::new(path)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .output(),
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
