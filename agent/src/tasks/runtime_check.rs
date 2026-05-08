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
                return runtime_unavailable("Docker image config is invalid");
            }
            let inspect_ok = check_docker_image_exists(image).await;
            if !inspect_ok {
                return RuntimeEnvironmentCheckResult {
                    check_status: "available".to_string(),
                    version: None,
                    message: "Docker image config passed basic validation; image not present locally, will be pulled on start"
                        .to_string(),
                };
            }
            let version = detect_docker_image_version(image).await;
            let has_version = version.is_some();
            RuntimeEnvironmentCheckResult {
                check_status: "available".to_string(),
                version,
                message: if has_version {
                    "Docker image exists and passed basic validation".to_string()
                } else {
                    "Docker image passed basic validation; version unavailable (may be verified at real start)".to_string()
                },
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
                    message: format!(
                        "{backend} entrypoint available, using manual version {version}"
                    ),
                };
            }
            if let Some(version) = detect_entrypoint_version(path).await {
                return RuntimeEnvironmentCheckResult {
                    check_status: "available".to_string(),
                    version: Some(version),
                    message: format!("{backend} entrypoint available, version auto-detected"),
                };
            }
            RuntimeEnvironmentCheckResult {
                check_status: "version_unavailable".to_string(),
                version: None,
                message: format!(
                    "{backend} entrypoint available but version could not be auto-detected; --version did not return a recognizable version; version may be filled manually"
                ),
            }
        }
        _ => runtime_unavailable("deploy type not supported"),
    }
}

pub(crate) async fn verify_controlled_entrypoint(path: &str) -> RuntimeEnvironmentCheckResult {
    if path.trim().is_empty() || has_parent_dir(path) {
        return runtime_unavailable("invalid entrypoint path");
    }
    let path = Path::new(path);
    if !path.is_absolute() {
        return runtime_unavailable("entrypoint path must be absolute");
    }
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return runtime_unavailable("entrypoint file does not exist");
        }
        Err(error) => {
            return runtime_unavailable(&format!("entrypoint file not accessible: {error}"))
        }
    };
    if metadata.file_type().is_symlink() {
        return runtime_unavailable("Security risk: entrypoint must not be a symlink");
    }
    if !metadata.is_file() {
        return RuntimeEnvironmentCheckResult {
            check_status: "invalid_path".to_string(),
            version: None,
            message: "entrypoint path is not a regular file".to_string(),
        };
    }
    if !is_executable(&metadata) {
        return RuntimeEnvironmentCheckResult {
            check_status: "not_executable".to_string(),
            version: None,
            message: "entrypoint file is not executable".to_string(),
        };
    }
    RuntimeEnvironmentCheckResult {
        check_status: "available".to_string(),
        version: None,
        message: "Entrypoint file is accessible; version could not be auto-detected".to_string(),
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

async fn check_docker_image_exists(image: &str) -> bool {
    let output = timeout(
        Duration::from_secs(5),
        Command::new("docker")
            .args(["image", "inspect", image])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await;
    match output {
        Ok(Ok(output)) => output.status.success(),
        _ => false,
    }
}

async fn detect_docker_image_version(image: &str) -> Option<String> {
    let output = timeout(
        Duration::from_secs(10),
        Command::new("docker")
            .args([
                "run",
                "--rm",
                "--entrypoint",
                "python",
                image,
                "-c",
                "import importlib; m = importlib.util.find_spec('vllm'); print(m and __import__('vllm').__version__)",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|v| v.chars().take(120).collect())
}
