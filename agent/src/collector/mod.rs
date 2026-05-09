pub mod execute;
pub mod inspect;
pub mod manifest;
pub mod registry;
pub mod tsv;

use std::path::{Path, PathBuf};

use manifest::CollectorManifest;
use registry::RegistryEntry;

/// A complete local collector directory with parsed manifest and script hashes.
#[derive(Debug, Clone)]
pub struct CollectorDir {
    pub dir_path: PathBuf,
    pub manifest: CollectorManifest,
    pub discover_sha256: String,
    pub metrics_sha256: String,
}

/// Agent-side collector configuration.
#[derive(Debug, Clone)]
pub struct CollectorConfig {
    pub root: PathBuf,
    pub mode: CollectorMode,
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/opt/lightai/collectors/gpu"),
            mode: CollectorMode::Explicit,
            enabled: Vec::new(),
            disabled: Vec::new(),
            timeout_secs: 5,
            max_output_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectorMode {
    /// Only execute collectors listed in `enabled`.
    Explicit,
    /// Scan all collectors in root, subject to `disabled` and registry validation.
    Auto,
}

impl CollectorMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "explicit" => Some(Self::Explicit),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// Scan `root` for valid collector directories.
///
/// Skips hidden dirs (starting with `.`), temp dirs (ending with `~` or `.tmp`),
/// and backup dirs (ending with `.bak`).
/// Directories missing `collector.toml`, `discover.sh`, or `metrics.sh` are skipped
/// and logged as warnings.
pub fn scan_collector_dirs(root: &Path) -> Vec<CollectorDir> {
    let mut dirs = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return dirs,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir_name.starts_with('.')
            || dir_name.ends_with('~')
            || dir_name.ends_with(".tmp")
            || dir_name.ends_with(".bak")
        {
            continue;
        }

        match CollectorDir::from_dir(&path) {
            Ok(dir) => dirs.push(dir),
            Err(e) => {
                tracing::warn!(
                    collector_dir = %path.display(),
                    error = %e,
                    "skipping invalid collector directory"
                );
            }
        }
    }

    dirs.sort_by(|a, b| {
        b.manifest
            .priority
            .cmp(&a.manifest.priority)
            .then_with(|| a.manifest.id.cmp(&b.manifest.id))
    });
    dirs
}

/// Result of checking whether a collector is allowed to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnableStatus {
    /// Collector is allowed to execute.
    Allowed,
    /// Collector is in the disabled list.
    Disabled,
    /// Collector not in the explicit enabled list.
    NotListed,
    /// No matching registry entry for this id+version.
    NotRegistered,
    /// Registry entry exists but is disabled.
    RegistryDisabled,
    /// Discover script hash does not match registry.
    DiscoverHashMismatch,
    /// Metrics script hash does not match registry.
    MetricsHashMismatch,
}

impl CollectorDir {
    pub fn from_dir(dir_path: &Path) -> anyhow::Result<Self> {
        let manifest_path = dir_path.join("collector.toml");
        if !manifest_path.is_file() {
            anyhow::bail!("missing collector.toml in {}", dir_path.display());
        }
        let manifest = manifest::parse(&manifest_path)?;

        let discover_path = dir_path.join(&manifest.discover);
        if !discover_path.is_file() {
            anyhow::bail!(
                "missing discover script '{}' in {}",
                manifest.discover,
                dir_path.display()
            );
        }
        let metrics_path = dir_path.join(&manifest.metrics);
        if !metrics_path.is_file() {
            anyhow::bail!(
                "missing metrics script '{}' in {}",
                manifest.metrics,
                dir_path.display()
            );
        }

        let discover_bytes = std::fs::read(&discover_path)?;
        let metrics_bytes = std::fs::read(&metrics_path)?;
        let discover_sha256 = execute::sha256_hex(&discover_bytes);
        let metrics_sha256 = execute::sha256_hex(&metrics_bytes);

        Ok(Self {
            dir_path: dir_path.to_path_buf(),
            manifest,
            discover_sha256,
            metrics_sha256,
        })
    }

    /// Check if this collector should be enabled given the Agent config and registry.
    ///
    /// Returns `EnableStatus::Allowed` only when ALL of these hold:
    /// 1. Not in the disabled list.
    /// 2. In the explicit enabled list (explicit mode) or any (auto mode).
    /// 3. A matching, enabled registry entry exists (id + version).
    /// 4. Both discover and metrics script hashes match the registry.
    ///
    /// This is fail-closed: a missing or empty registry means no collector executes.
    pub fn check_enabled(
        &self,
        config: &CollectorConfig,
        registry: &[RegistryEntry],
    ) -> EnableStatus {
        let id = &self.manifest.id;

        // Disabled list takes priority.
        if config.disabled.iter().any(|d| d == id) {
            return EnableStatus::Disabled;
        }

        // In explicit mode, only execute listed collectors.
        if config.mode == CollectorMode::Explicit && !config.enabled.iter().any(|e| e == id) {
            return EnableStatus::NotListed;
        }

        // Must have a matching, enabled registry entry (id + version).
        let entry = match registry
            .iter()
            .find(|r| r.id == *id && r.version == self.manifest.version)
        {
            Some(e) => e,
            None => return EnableStatus::NotRegistered,
        };

        if !entry.enabled {
            return EnableStatus::RegistryDisabled;
        }

        if entry.discover_sha256 != self.discover_sha256 {
            return EnableStatus::DiscoverHashMismatch;
        }
        if entry.metrics_sha256 != self.metrics_sha256 {
            return EnableStatus::MetricsHashMismatch;
        }

        EnableStatus::Allowed
    }
}
