use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static GLOBAL_POLICY: OnceLock<Mutex<LogPolicy>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogPolicy {
    #[serde(default = "default_log_dir")]
    pub log_dir: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_log_max_file_bytes")]
    pub log_max_file_bytes: u64,
    #[serde(default = "default_log_retention_files")]
    pub log_retention_files: usize,
    #[serde(default = "default_log_retention_days")]
    pub log_retention_days: u64,
}

impl Default for LogPolicy {
    fn default() -> Self {
        Self {
            log_dir: "logs".to_string(),
            log_level: "info".to_string(),
            log_max_file_bytes: 10 * 1024 * 1024,
            log_retention_files: 5,
            log_retention_days: 7,
        }
    }
}

fn default_log_dir() -> String {
    LogPolicy::default().log_dir
}

fn default_log_level() -> String {
    LogPolicy::default().log_level
}

fn default_log_max_file_bytes() -> u64 {
    LogPolicy::default().log_max_file_bytes
}

fn default_log_retention_files() -> usize {
    LogPolicy::default().log_retention_files
}

fn default_log_retention_days() -> u64 {
    LogPolicy::default().log_retention_days
}

pub fn set_global(policy: LogPolicy) {
    let lock = GLOBAL_POLICY.get_or_init(|| Mutex::new(LogPolicy::default()));
    if let Ok(mut current) = lock.lock() {
        *current = policy;
    }
}

pub fn global() -> LogPolicy {
    GLOBAL_POLICY
        .get_or_init(|| Mutex::new(LogPolicy::default()))
        .lock()
        .map(|policy| policy.clone())
        .unwrap_or_default()
}

pub fn validate_policy(policy: &LogPolicy) -> anyhow::Result<()> {
    validate_log_dir(&policy.log_dir)?;
    match policy.log_level.as_str() {
        "error" | "warn" | "info" | "debug" | "trace" => {}
        _ => anyhow::bail!("log_level is invalid"),
    }
    if policy.log_max_file_bytes == 0 || policy.log_max_file_bytes > 1024 * 1024 * 1024 {
        anyhow::bail!("log_max_file_bytes must be between 1 and 1073741824");
    }
    if policy.log_retention_files == 0 || policy.log_retention_files > 100 {
        anyhow::bail!("log_retention_files must be between 1 and 100");
    }
    if policy.log_retention_days > 3650 {
        anyhow::bail!("log_retention_days must be between 0 and 3650");
    }
    Ok(())
}

pub fn validate_log_dir(value: &str) -> anyhow::Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains("..") || trimmed.contains('\0') {
        anyhow::bail!("log_dir is invalid");
    }
    let path = Path::new(trimmed);
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::RootDir
        )
    }) && trimmed == "/"
    {
        anyhow::bail!("log_dir is dangerous");
    }
    Ok(())
}

pub async fn append(
    policy: &LogPolicy,
    file_name: &str,
    level: &str,
    message: &str,
) -> anyhow::Result<()> {
    if !level_enabled(&policy.log_level, level) {
        return Ok(());
    }
    validate_policy(policy)?;
    let dir = prepare_dir(&policy.log_dir).await?;
    let file_path = safe_log_file(&dir, file_name)?;
    let line = format!(
        "{} [{}] {}\n",
        format_timestamp(),
        level.to_ascii_uppercase(),
        sanitize(message)
    );
    rotate_if_needed(&file_path, policy, line.len() as u64).await?;
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
        .await?
        .write_all(line.as_bytes())
        .await?;
    cleanup_retention(&file_path, policy).await?;
    Ok(())
}

pub async fn read_tail(
    policy: &LogPolicy,
    file_name: &str,
    max_bytes: usize,
) -> anyhow::Result<String> {
    validate_policy(policy)?;
    let dir = prepare_dir(&policy.log_dir).await?;
    let file_path = safe_log_file(&dir, file_name)?;
    let bytes = match tokio::fs::read(&file_path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let start = bytes.len().saturating_sub(max_bytes);
    Ok(sanitize(&String::from_utf8_lossy(&bytes[start..])))
}

pub fn sanitize(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "token",
                "password",
                "authorization",
                "api key",
                "api_key",
                "apikey",
                "secret",
            ]
            .iter()
            .any(|needle| lower.contains(needle))
            {
                "[redacted — sensitive log line]".to_string()
            } else {
                line.chars().take(1000).collect()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn prepare_dir(value: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(value);
    if let Ok(metadata) = tokio::fs::symlink_metadata(&path).await {
        if metadata.file_type().is_symlink() {
            anyhow::bail!("log directory must not be a symlink");
        }
        if !metadata.is_dir() {
            anyhow::bail!("log path is not a directory");
        }
    }
    tokio::fs::create_dir_all(&path).await?;
    Ok(path)
}

fn safe_log_file(dir: &Path, file_name: &str) -> anyhow::Result<PathBuf> {
    if !matches!(file_name, "server.log" | "agent.log" | "instance.log") {
        anyhow::bail!("log file is not managed by platform");
    }
    Ok(dir.join(file_name))
}

async fn rotate_if_needed(
    path: &Path,
    policy: &LogPolicy,
    incoming_bytes: u64,
) -> anyhow::Result<()> {
    let current_size = tokio::fs::metadata(path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if current_size + incoming_bytes <= policy.log_max_file_bytes {
        return Ok(());
    }
    for index in (1..=policy.log_retention_files).rev() {
        let from = rotated_path(path, index);
        let to = rotated_path(path, index + 1);
        if tokio::fs::metadata(&from).await.is_ok() {
            let _ = tokio::fs::rename(&from, &to).await;
        }
    }
    if tokio::fs::metadata(path).await.is_ok() {
        tokio::fs::rename(path, rotated_path(path, 1)).await?;
    }
    Ok(())
}

async fn cleanup_retention(path: &Path, policy: &LogPolicy) -> anyhow::Result<()> {
    for index in (policy.log_retention_files + 1)..=(policy.log_retention_files + 20) {
        let _ = tokio::fs::remove_file(rotated_path(path, index)).await;
    }
    if policy.log_retention_days == 0 {
        return Ok(());
    }
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(
            policy.log_retention_days * 86_400,
        ))
        .unwrap_or(std::time::UNIX_EPOCH);
    for index in 1..=(policy.log_retention_files + 20) {
        let rotated = rotated_path(path, index);
        if let Ok(metadata) = tokio::fs::metadata(&rotated).await {
            if metadata.modified().unwrap_or(std::time::SystemTime::now()) < cutoff {
                let _ = tokio::fs::remove_file(rotated).await;
            }
        }
    }
    Ok(())
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    let file_name = path.file_name().and_then(|v| v.to_str()).unwrap_or("log");
    path.with_file_name(format!("{file_name}.{index}"))
}

fn level_enabled(configured: &str, level: &str) -> bool {
    fn rank(level: &str) -> u8 {
        match level {
            "error" => 1,
            "warn" => 2,
            "info" => 3,
            "debug" => 4,
            "trace" => 5,
            _ => 3,
        }
    }
    rank(level) <= rank(configured)
}

fn format_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = civil_from_days(days_since_epoch as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + (era * 400);
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

use tokio::io::AsyncWriteExt;
