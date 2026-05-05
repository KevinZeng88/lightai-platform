use std::path::{Path, PathBuf};

use super::{has_parent_dir, CleanupModelFileResult};

pub async fn cleanup_model_file(
    path: &str,
    allowed_model_dirs: &[String],
) -> CleanupModelFileResult {
    if allowed_model_dirs.is_empty() {
        return cleanup_failure("未配置受控模型目录，拒绝删除文件");
    }
    if path.trim().is_empty() || has_parent_dir(path) {
        return cleanup_failure("路径非法");
    }
    let target = Path::new(path);
    if !target.is_absolute() {
        return cleanup_failure("路径必须是绝对路径");
    }

    let allowed_dirs = match allowed_canonical_dirs(allowed_model_dirs).await {
        AllowedDirResolution::Dirs(dirs) => dirs,
        AllowedDirResolution::InvalidConfig => return cleanup_failure("受控模型目录配置非法"),
        AllowedDirResolution::Missing => return cleanup_failure("受控模型目录不存在"),
        AllowedDirResolution::Inaccessible => return cleanup_failure("受控模型目录不可访问"),
    };
    if allowed_dirs.is_empty() {
        return cleanup_failure("受控模型目录配置非法");
    }

    let metadata = match tokio::fs::symlink_metadata(target).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return cleanup_failure("文件不存在");
        }
        Err(error) => {
            return cleanup_failure(&format!("读取文件信息失败：{error}"));
        }
    };

    if metadata.file_type().is_symlink() {
        return cleanup_failure("安全风险：拒绝删除软链接");
    }
    if !metadata.is_file() {
        return cleanup_failure("拒绝删除目录或非普通文件");
    }

    let canonical_target = match tokio::fs::canonicalize(target).await {
        Ok(path) => path,
        Err(error) => {
            return cleanup_failure(&format!("解析文件路径失败：{error}"));
        }
    };
    if !allowed_dirs
        .iter()
        .any(|allowed_dir| canonical_target.starts_with(allowed_dir))
    {
        return cleanup_failure("文件不在受控模型目录内");
    }

    match tokio::fs::remove_file(target).await {
        Ok(()) => CleanupModelFileResult {
            cleanup_status: "deleted".to_string(),
            message: "文件已清理".to_string(),
        },
        Err(error) => cleanup_failure(&format!("删除文件失败：{error}")),
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
