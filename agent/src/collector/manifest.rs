use std::path::Path;

/// Parsed `collector.toml` manifest.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CollectorManifest {
    pub id: String,
    pub vendor: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub discover: String,
    pub metrics: String,
    /// Informational SHA-256 of discover script (not a trust source).
    #[serde(default)]
    pub discover_sha256: String,
    /// Informational SHA-256 of metrics script (not a trust source).
    #[serde(default)]
    pub metrics_sha256: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_priority")]
    pub priority: u32,
}

fn default_enabled() -> bool {
    true
}

fn default_priority() -> u32 {
    100
}

pub fn parse(path: &Path) -> anyhow::Result<CollectorManifest> {
    let content = std::fs::read_to_string(path)?;
    let manifest: CollectorManifest = toml::from_str(&content)?;

    // Basic validation.
    if manifest.id.is_empty() {
        anyhow::bail!("collector id must not be empty");
    }
    if manifest.vendor.is_empty() {
        anyhow::bail!("collector vendor must not be empty");
    }
    if manifest.name.is_empty() {
        anyhow::bail!("collector name must not be empty");
    }
    if manifest.version.is_empty() {
        anyhow::bail!("collector version must not be empty");
    }
    if manifest.discover.is_empty() {
        anyhow::bail!("discover script path must not be empty");
    }
    if manifest.metrics.is_empty() {
        anyhow::bail!("metrics script path must not be empty");
    }
    if manifest.discover.contains('/') || manifest.discover.contains('\\') {
        anyhow::bail!("discover script must be a filename, not a path");
    }
    if manifest.metrics.contains('/') || manifest.metrics.contains('\\') {
        anyhow::bail!("metrics script must be a filename, not a path");
    }

    Ok(manifest)
}
