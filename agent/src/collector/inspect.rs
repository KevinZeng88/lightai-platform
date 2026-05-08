use serde::Serialize;
use std::path::Path;

use super::execute::{check_file_permissions, sha256_hex};
use super::manifest;

/// Output of `lightai-agent collector inspect <dir>`.
///
/// This JSON can be copy-pasted into the Web UI for registry registration.
#[derive(Debug, Serialize)]
pub struct InspectOutput {
    pub id: String,
    pub vendor: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub discover: String,
    pub metrics: String,
    pub discover_sha256: String,
    pub metrics_sha256: String,
    /// Non-fatal warnings (e.g. toml hash empty or mismatched).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Inspect a collector directory and produce the registry-ready JSON output.
///
/// Checks:
/// - collector.toml exists and is valid
/// - discover.sh and metrics.sh exist (as named in manifest)
/// - Files are not symlinks and not world-writable
/// - Computes SHA-256 in-process (no external sha256sum dependency)
///
/// Does NOT register, approve, or execute any script.
pub fn inspect(dir_path: &Path) -> anyhow::Result<InspectOutput> {
    if !dir_path.is_dir() {
        anyhow::bail!("{} is not a directory", dir_path.display());
    }

    let manifest_path = dir_path.join("collector.toml");
    if !manifest_path.is_file() {
        anyhow::bail!("collector.toml not found in {}", dir_path.display());
    }
    let manifest = manifest::parse(&manifest_path)?;

    let discover_path = dir_path.join(&manifest.discover);
    if !discover_path.is_file() {
        anyhow::bail!(
            "discover script '{}' not found in {}",
            manifest.discover,
            dir_path.display()
        );
    }
    check_file_permissions(&discover_path)?;
    let metrics_path = dir_path.join(&manifest.metrics);
    if !metrics_path.is_file() {
        anyhow::bail!(
            "metrics script '{}' not found in {}",
            manifest.metrics,
            dir_path.display()
        );
    }
    check_file_permissions(&metrics_path)?;

    // Also check the collector dir itself for world-writability.
    let dir_meta = std::fs::symlink_metadata(dir_path)?;
    use std::os::unix::fs::PermissionsExt;
    let dir_mode = dir_meta.permissions().mode();
    if dir_mode & 0o002 != 0 {
        anyhow::bail!(
            "collector directory {} is world-writable, not allowed",
            dir_path.display()
        );
    }

    let discover_bytes = std::fs::read(&discover_path)?;
    let metrics_bytes = std::fs::read(&metrics_path)?;
    let discover_sha256 = sha256_hex(&discover_bytes);
    let metrics_sha256 = sha256_hex(&metrics_bytes);

    let mut warnings = Vec::new();

    // Only warn if a non-empty toml hash does not match the computed one.
    // Empty hash fields are normal (template mode); no warning.
    if !manifest.discover_sha256.is_empty() && manifest.discover_sha256 != discover_sha256 {
        warnings.push(format!(
            "collector.toml: discover_sha256 mismatch — toml has '{}', actual is '{}'. \
             Update toml or re-run inspect.",
            manifest.discover_sha256, discover_sha256
        ));
    }

    if !manifest.metrics_sha256.is_empty() && manifest.metrics_sha256 != metrics_sha256 {
        warnings.push(format!(
            "collector.toml: metrics_sha256 mismatch — toml has '{}', actual is '{}'. \
             Update toml or re-run inspect.",
            manifest.metrics_sha256, metrics_sha256
        ));
    }

    if !warnings.is_empty() {
        for w in &warnings {
            eprintln!("WARNING: {w}");
        }
    }

    Ok(InspectOutput {
        id: manifest.id,
        vendor: manifest.vendor,
        name: manifest.name,
        version: manifest.version,
        description: manifest.description,
        discover: manifest.discover,
        metrics: manifest.metrics,
        discover_sha256,
        metrics_sha256,
        warnings,
    })
}
