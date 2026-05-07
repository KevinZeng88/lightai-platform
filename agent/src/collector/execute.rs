use sha2::{Digest, Sha256};
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// Compute the SHA-256 hex digest of `data`.
///
/// Implemented in-process (no external sha256sum dependency).
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Result of a collector script execution.
#[derive(Debug)]
pub struct ScriptOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

/// Execute a script from memory (already validated).
///
/// The script content is passed via stdin to `/bin/sh -s`.
/// Execution is sandboxed with:
/// - Fixed PATH
/// - Limited environment (whitelist)
/// - Fixed working directory
/// - Hard timeout
/// - Output size limits
pub async fn execute_script(
    script_bytes: &[u8],
    timeout_secs: u64,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> ScriptOutput {
    let result = timeout(
        Duration::from_secs(timeout_secs),
        run_script(script_bytes, max_stdout_bytes, max_stderr_bytes),
    )
    .await;

    match result {
        Ok(output) => output,
        Err(_elapsed) => ScriptOutput {
            stdout: String::new(),
            stderr: "collector script timed out".to_string(),
            exit_code: None,
            timed_out: true,
        },
    }
}

async fn run_script(
    script_bytes: &[u8],
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> ScriptOutput {
    let mut child = match Command::new("/bin/sh")
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("HOME", "/tmp")
        .env("USER", "lightai")
        .current_dir("/tmp")
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return ScriptOutput {
                stdout: String::new(),
                stderr: format!("failed to spawn collector script: {e}"),
                exit_code: None,
                timed_out: false,
            };
        }
    };

    // Write script content to stdin, then drop the writer to close stdin.
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        if stdin.write_all(script_bytes).await.is_err() {
            let _ = child.kill().await;
            return ScriptOutput {
                stdout: String::new(),
                stderr: "failed to write script to stdin".to_string(),
                exit_code: None,
                timed_out: false,
            };
        }
    }

    let output = match child.wait_with_output().await {
        Ok(output) => output,
        Err(e) => {
            return ScriptOutput {
                stdout: String::new(),
                stderr: format!("failed to wait for collector script: {e}"),
                exit_code: None,
                timed_out: false,
            };
        }
    };

    let stdout = truncate_bytes(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        max_stdout_bytes,
    );
    let stderr = truncate_bytes(
        String::from_utf8_lossy(&output.stderr).into_owned(),
        max_stderr_bytes,
    );

    ScriptOutput {
        stdout,
        stderr,
        exit_code: output.status.code(),
        timed_out: false,
    }
}

fn truncate_bytes(s: String, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = s[..end].to_string();
    truncated.push_str("\n[output truncated]");
    truncated
}

/// Check whether a file path points to a regular file that is not a symlink
/// and is not world-writable. Returns Ok(()) if safe, or an error otherwise.
pub fn check_file_permissions(path: &std::path::Path) -> anyhow::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        anyhow::bail!("{} is a symlink, not allowed", path.display());
    }
    if !meta.is_file() {
        anyhow::bail!("{} is not a regular file", path.display());
    }
    // Check world-writable.
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode();
    if mode & 0o002 != 0 {
        anyhow::bail!("{} is world-writable, not allowed", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_produces_known_hash() {
        let hash = sha256_hex(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn truncate_bytes_respects_char_boundary() {
        // 'é' is 2 bytes in UTF-8: h(1) + é(2) = 3 bytes for "hé".
        // max_bytes=2 falls in the middle of 'é', must truncate at byte 1.
        let s = "héllo world".to_string();
        let t = truncate_bytes(s, 2);
        assert_eq!(t, "h\n[output truncated]");
    }
}
