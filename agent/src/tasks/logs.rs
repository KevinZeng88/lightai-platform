use std::path::Path;

use super::process::{running_instance_log_tail, sanitize_log, tail_bytes};
use crate::managed_process;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ReadInstanceLogResult {
    pub(crate) log_status: String,
    pub(crate) content: String,
    pub(crate) message: String,
}

pub(crate) async fn read_instance_log(
    payload: &serde_json::Value,
    managed_store_path: Option<&Path>,
) -> ReadInstanceLogResult {
    let instance_id = payload
        .get("instance_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let max_bytes = payload
        .get("max_bytes")
        .and_then(|value| value.as_u64())
        .unwrap_or(64 * 1024)
        .min(512 * 1024) as usize;

    if let Some(tail) = running_instance_log_tail(instance_id, max_bytes).await {
        if tail.trim().is_empty() {
            return ReadInstanceLogResult {
                log_status: "available".to_string(),
                content: "实例进程正在运行，暂无日志输出".to_string(),
                message: "从内存缓冲区读取".to_string(),
            };
        }
        return ReadInstanceLogResult {
            log_status: "available".to_string(),
            content: tail,
            message: "从内存缓冲区读取".to_string(),
        };
    }

    if let Some(store_path) = managed_store_path {
        if let Ok(Some(record)) = managed_process::find(store_path, instance_id).await {
            if let Some(ref log_path) = record.log_path {
                match tokio::fs::read_to_string(log_path).await {
                    Ok(content) => {
                        let tail = tail_bytes(&content, max_bytes);
                        if tail.trim().is_empty() {
                            return ReadInstanceLogResult {
                                log_status: "available".to_string(),
                                content: "日志文件为空".to_string(),
                                message: format!("从日志文件 {} 读取", log_path),
                            };
                        }
                        return ReadInstanceLogResult {
                            log_status: "available".to_string(),
                            content: sanitize_log(&tail),
                            message: format!("从日志文件 {} 读取", log_path),
                        };
                    }
                    Err(error) => {
                        return ReadInstanceLogResult {
                            log_status: "failed".to_string(),
                            content: String::new(),
                            message: format!("读取实例日志文件失败：{error}"),
                        };
                    }
                }
            }
        }
    }

    ReadInstanceLogResult {
        log_status: "failed".to_string(),
        content: String::new(),
        message: "未找到实例日志；实例可能已停止或 Agent 已重启".to_string(),
    }
}
