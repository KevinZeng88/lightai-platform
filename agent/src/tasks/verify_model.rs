use super::VerifyModelFileResult;

pub async fn verify_model_file(path: &str) -> VerifyModelFileResult {
    if path.trim().is_empty() || path.contains("..") {
        return failure("invalid_path", "路径非法");
    }

    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return failure("missing", "文件不存在");
        }
        Err(error) => {
            return failure("failed", &format!("读取文件信息失败：{error}"));
        }
    };
    if metadata.file_type().is_symlink() {
        return failure("security_risk", "安全风险：模型路径不能是软链接");
    }

    if metadata.is_dir() {
        return VerifyModelFileResult {
            file_status: "verified".to_string(),
            size_bytes: None,
            path_type: Some("directory".to_string()),
            message: "目录已验证".to_string(),
        };
    }

    if !metadata.is_file() {
        return failure("not_file", "路径不是普通文件或目录");
    }

    VerifyModelFileResult {
        file_status: "verified".to_string(),
        size_bytes: Some(metadata.len().min(i64::MAX as u64) as i64),
        path_type: Some("file".to_string()),
        message: "文件已验证".to_string(),
    }
}

fn failure(status: &str, message: &str) -> VerifyModelFileResult {
    VerifyModelFileResult {
        file_status: status.to_string(),
        size_bytes: None,
        path_type: None,
        message: message.to_string(),
    }
}
