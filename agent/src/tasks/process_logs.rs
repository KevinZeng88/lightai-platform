use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use super::has_parent_dir;

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
                "[redacted — sensitive log line]".to_string()
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

pub(crate) async fn log_tail(log_buffer: &Arc<Mutex<String>>) -> Option<String> {
    let value = log_buffer.lock().await.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub(crate) async fn log_tail_with_path(
    log_buffer: &Arc<Mutex<String>>,
    log_path: Option<&str>,
) -> Option<String> {
    match (log_path, log_tail(log_buffer).await) {
        (Some(path), Some(tail)) => Some(format!("log file: {path}\n{tail}")),
        (Some(path), None) => Some(format!("log file: {path}")),
        (None, tail) => tail,
    }
}

pub(crate) fn trim_log_buffer(buffer: &mut String) {
    const MAX_LOG_BYTES: usize = 8192;
    if buffer.len() > MAX_LOG_BYTES {
        let start = buffer.len() - MAX_LOG_BYTES;
        *buffer = buffer[start..].to_string();
    }
}

pub(crate) fn combined_output_log(stdout: &[u8], stderr: &[u8]) -> Option<String> {
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

pub(crate) fn first_log_line(log_tail: Option<&str>) -> Option<String> {
    log_tail?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && *line != "stdout:" && *line != "stderr:")
        .map(|line| line.chars().take(220).collect())
}

pub(crate) async fn controlled_log_path(
    log_dir: &str,
    instance_id: &str,
) -> Result<PathBuf, String> {
    if log_dir.trim().is_empty() || has_parent_dir(log_dir) {
        return Err("invalid log directory path".to_string());
    }
    let dir = Path::new(log_dir);
    if !dir.is_absolute() {
        return Err("log directory must be an absolute path".to_string());
    }
    if let Ok(metadata) = tokio::fs::symlink_metadata(dir).await {
        if metadata.file_type().is_symlink() {
            return Err("log directory must not be a symlink".to_string());
        }
        if !metadata.is_dir() {
            return Err("log directory is not a directory".to_string());
        }
    }
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| format!("failed to create log directory: {error}"))?;
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
