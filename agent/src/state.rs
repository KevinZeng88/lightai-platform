use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentState {
    pub node_id: String,
    pub agent_token: String,
}

pub async fn load(path: &str) -> anyhow::Result<Option<AgentState>> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Ok(Some(toml::from_str(&content)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub async fn save(path: &str, state: &AgentState) -> anyhow::Result<()> {
    let path = std::path::Path::new(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = toml::to_string_pretty(state)?;
    write_private(path, content.as_bytes())?;
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, content: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, content: &[u8]) -> anyhow::Result<()> {
    // Windows ACL hardening is intentionally deferred for Stage 2.
    std::fs::write(path, content)?;
    Ok(())
}
