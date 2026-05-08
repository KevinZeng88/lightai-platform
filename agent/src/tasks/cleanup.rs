use std::path::{Path, PathBuf};

use super::{has_parent_dir, CleanupModelFileResult};

pub async fn cleanup_model_file(
    path: &str,
    allowed_model_dirs: &[String],
) -> CleanupModelFileResult {
    if allowed_model_dirs.is_empty() {
        return cleanup_failure("no allowed model directory configured; refusing to delete file");
    }
    if path.trim().is_empty() || has_parent_dir(path) {
        return cleanup_failure("invalid path");
    }
    let target = Path::new(path);
    if !target.is_absolute() {
        return cleanup_failure("path must be absolute");
    }

    let allowed_dirs = match allowed_canonical_dirs(allowed_model_dirs).await {
        AllowedDirResolution::Dirs(dirs) => dirs,
        AllowedDirResolution::InvalidConfig => {
            return cleanup_failure("allowed model directory config is invalid")
        }
        AllowedDirResolution::Missing => {
            return cleanup_failure("allowed model directory does not exist")
        }
        AllowedDirResolution::Inaccessible => {
            return cleanup_failure("allowed model directory not accessible")
        }
    };
    if allowed_dirs.is_empty() {
        return cleanup_failure("allowed model directory config is invalid");
    }

    let metadata = match tokio::fs::symlink_metadata(target).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return cleanup_failure("file does not exist");
        }
        Err(error) => {
            return cleanup_failure(&format!("failed to read file info: {error}"));
        }
    };

    if metadata.file_type().is_symlink() {
        return cleanup_failure("Security risk: refusing to delete symlink");
    }
    if !metadata.is_file() {
        return cleanup_failure("refusing to delete directory or non-regular file");
    }

    let canonical_target = match tokio::fs::canonicalize(target).await {
        Ok(path) => path,
        Err(error) => {
            return cleanup_failure(&format!("failed to resolve file path: {error}"));
        }
    };
    if !allowed_dirs
        .iter()
        .any(|allowed_dir| canonical_target.starts_with(allowed_dir))
    {
        return cleanup_failure("file not within allowed model directory");
    }

    match tokio::fs::remove_file(target).await {
        Ok(()) => CleanupModelFileResult {
            cleanup_status: "deleted".to_string(),
            message: "file cleaned up".to_string(),
        },
        Err(error) => cleanup_failure(&format!("failed to delete file: {error}")),
    }
}

fn cleanup_failure(message: &str) -> CleanupModelFileResult {
    CleanupModelFileResult {
        cleanup_status: "failed".to_string(),
        message: message.to_string(),
    }
}

enum AllowedDirResolution {
    Dirs(Vec<PathBuf>),
    InvalidConfig,
    Missing,
    Inaccessible,
}

async fn allowed_canonical_dirs(allowed_model_dirs: &[String]) -> AllowedDirResolution {
    let mut dirs = Vec::new();
    let mut saw_invalid = false;
    let mut saw_missing = false;
    let mut saw_inaccessible = false;
    for dir in allowed_model_dirs {
        if dir.trim().is_empty() || has_parent_dir(dir) {
            saw_invalid = true;
            continue;
        }
        let path = Path::new(dir);
        if !path.is_absolute() {
            saw_invalid = true;
            continue;
        }
        match tokio::fs::canonicalize(path).await {
            Ok(canonical) => {
                if canonical.is_dir() {
                    dirs.push(canonical);
                } else {
                    saw_invalid = true;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => saw_missing = true,
            Err(_) => saw_inaccessible = true,
        }
    }
    if !dirs.is_empty() {
        AllowedDirResolution::Dirs(dirs)
    } else if saw_invalid {
        AllowedDirResolution::InvalidConfig
    } else if saw_missing {
        AllowedDirResolution::Missing
    } else if saw_inaccessible {
        AllowedDirResolution::Inaccessible
    } else {
        AllowedDirResolution::InvalidConfig
    }
}
