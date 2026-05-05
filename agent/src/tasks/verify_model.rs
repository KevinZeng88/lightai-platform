use super::VerifyModelFileResult;

pub async fn verify_model_file(path: &str) -> VerifyModelFileResult {
    verify_model_file_with_hint(path, None).await
}

pub async fn verify_model_file_with_hint(
    path: &str,
    path_type_hint: Option<&str>,
) -> VerifyModelFileResult {
    if path.trim().is_empty() || path.contains("..") {
        return failure("invalid_path", "路径非法");
    }

    // ollama model names are not filesystem paths — accept non-empty strings
    if path_type_hint == Some("ollama") {
        return VerifyModelFileResult {
            file_status: "verified".to_string(),
            size_bytes: None,
            path_type: Some("ollama".to_string()),
            message: "Ollama 模型名已接受".to_string(),
        };
    }

    // custom path type: relaxed check, accept if path exists or give clear prompt
    if path_type_hint == Some("custom") {
        match tokio::fs::symlink_metadata(path).await {
            Ok(_) => {
                return VerifyModelFileResult {
                    file_status: "verified".to_string(),
                    size_bytes: None,
                    path_type: Some("custom".to_string()),
                    message: "自定义路径已验证".to_string(),
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return VerifyModelFileResult {
                    file_status: "verified".to_string(),
                    size_bytes: None,
                    path_type: Some("custom".to_string()),
                    message: "自定义路径不存在，请确认目标节点上该路径可用".to_string(),
                };
            }
            Err(error) => {
                return failure("failed", &format!("读取路径信息失败：{error}"));
            }
        }
    }

    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return failure("missing", "路径不存在");
        }
        Err(error) => {
            return failure("failed", &format!("读取文件信息失败：{error}"));
        }
    };
    if metadata.file_type().is_symlink() {
        return failure("security_risk", "安全风险：模型路径不能是软链接");
    }

    if metadata.is_dir() {
        // If path_type_hint says "file" but it's actually a directory, warn
        if path_type_hint == Some("file") {
            return failure("type_mismatch", "期望模型文件，但路径是目录");
        }
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

    // If path_type_hint says "directory" but it's actually a file, warn
    if path_type_hint == Some("directory") {
        return failure("type_mismatch", "期望模型目录，但路径是普通文件");
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
