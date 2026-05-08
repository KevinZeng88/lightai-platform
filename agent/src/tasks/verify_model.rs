use super::VerifyModelFileResult;

pub async fn verify_model_file(path: &str) -> VerifyModelFileResult {
    verify_model_file_with_hint(path, None).await
}

pub async fn verify_model_file_with_hint(
    path: &str,
    path_type_hint: Option<&str>,
) -> VerifyModelFileResult {
    if path.trim().is_empty() || path.contains("..") {
        return failure("invalid_path", "invalid path");
    }

    // ollama model names are not filesystem paths — accept non-empty strings
    if path_type_hint == Some("ollama") {
        return VerifyModelFileResult {
            file_status: "verified".to_string(),
            size_bytes: None,
            path_type: Some("ollama".to_string()),
            message: "Ollama model name accepted".to_string(),
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
                    message: "custom path verified".to_string(),
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return VerifyModelFileResult {
                    file_status: "verified".to_string(),
                    size_bytes: None,
                    path_type: Some("custom".to_string()),
                    message: "Custom path does not exist; verify the path is available on the target node".to_string(),
                };
            }
            Err(error) => {
                return failure("failed", &format!("failed to read path info: {error}"));
            }
        }
    }

    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return failure("missing", "path does not exist");
        }
        Err(error) => {
            return failure("failed", &format!("failed to read file info: {error}"));
        }
    };
    if metadata.file_type().is_symlink() {
        return failure(
            "security_risk",
            "Security risk: model path must not be a symlink",
        );
    }

    if metadata.is_dir() {
        // If path_type_hint says "file" but it's actually a directory, warn
        if path_type_hint == Some("file") {
            return failure(
                "type_mismatch",
                "Expected a model file, but path is a directory",
            );
        }
        return VerifyModelFileResult {
            file_status: "verified".to_string(),
            size_bytes: None,
            path_type: Some("directory".to_string()),
            message: "directory verified".to_string(),
        };
    }

    if !metadata.is_file() {
        return failure("not_file", "path is not a regular file or directory");
    }

    // If path_type_hint says "directory" but it's actually a file, warn
    if path_type_hint == Some("directory") {
        return failure(
            "type_mismatch",
            "Expected a model directory, but path is a regular file",
        );
    }

    VerifyModelFileResult {
        file_status: "verified".to_string(),
        size_bytes: Some(metadata.len().min(i64::MAX as u64) as i64),
        path_type: Some("file".to_string()),
        message: "file verified".to_string(),
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
