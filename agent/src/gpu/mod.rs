use std::path::PathBuf;

use crate::collector::{self, execute, registry::RegistryEntry};
use crate::models::GpuMetrics;

/// Configuration for the GPU collector pipeline.
///
/// GPU collection only uses directory-based collectors that are registered on
/// the Server and verified by script hash before execution.
#[derive(Debug, Clone)]
pub struct CollectorConfig {
    /// Root directory for script-based collectors.
    pub collector_root: Option<PathBuf>,
    pub collector_mode: String,
    pub collector_enabled: Vec<String>,
    pub collector_disabled: Vec<String>,
    /// Server registry entries (pulled from heartbeat/config).
    pub collector_registry: Vec<RegistryEntry>,
    pub collector_timeout_secs: u64,
    pub collector_max_output_bytes: usize,
}

impl CollectorConfig {
    /// Build the directory-based collector config for the new framework.
    fn to_dir_config(&self) -> Option<collector::CollectorConfig> {
        self.collector_root
            .as_ref()
            .map(|root| collector::CollectorConfig {
                root: root.clone(),
                mode: if self.collector_mode == "auto" {
                    collector::CollectorMode::Auto
                } else {
                    collector::CollectorMode::Explicit
                },
                enabled: self.collector_enabled.clone(),
                disabled: self.collector_disabled.clone(),
                timeout_secs: self.collector_timeout_secs,
                max_output_bytes: self.collector_max_output_bytes,
            })
    }
}

/// Run all enabled script collectors and aggregate results.
///
/// - If no collector root is configured, no GPU script executes.
/// - A single failing collector never blocks the heartbeat.
pub async fn collect_gpus(config: &CollectorConfig) -> (Vec<GpuMetrics>, Vec<String>) {
    let mut gpus = Vec::new();
    let mut errors = Vec::new();

    if let Some(dir_config) = config.to_dir_config() {
        collect_via_dirs(
            &dir_config,
            &config.collector_registry,
            &mut gpus,
            &mut errors,
        )
        .await;
        return (gpus, errors);
    }

    errors.push("gpu collector root is not configured; no GPU collectors executed".to_string());
    (gpus, errors)
}

/// Run directory-based collectors: scan, filter, execute discover+metrics, parse TSV, merge.
async fn collect_via_dirs(
    config: &collector::CollectorConfig,
    registry: &[RegistryEntry],
    gpus: &mut Vec<GpuMetrics>,
    errors: &mut Vec<String>,
) {
    let dirs = collector::scan_collector_dirs(&config.root);

    // If registry is completely empty and we have collectors, report it once.
    if registry.is_empty() && !dirs.is_empty() {
        errors.push(
            "collector registry is empty — no collectors will execute. \
             Register collector hashes via Web '采集器登记' page."
                .to_string(),
        );
    }

    for dir in dirs {
        let status = dir.check_enabled(config, registry);
        match status {
            collector::EnableStatus::Allowed => { /* proceed */ }
            collector::EnableStatus::Disabled => {
                errors.push(format!(
                    "{} (id={}, version={}): collector disabled in Agent config",
                    dir.manifest.id, dir.manifest.id, dir.manifest.version
                ));
                continue;
            }
            collector::EnableStatus::NotListed => {
                tracing::debug!(
                    collector_id = %dir.manifest.id,
                    "collector not in explicit enabled list"
                );
                continue;
            }
            collector::EnableStatus::NotRegistered => {
                errors.push(format!(
                    "{} (id={}, version={}): collector not registered in Server registry — \
                     run 'lightai-agent collector inspect' and register via Web",
                    dir.manifest.id, dir.manifest.id, dir.manifest.version
                ));
                continue;
            }
            collector::EnableStatus::RegistryDisabled => {
                errors.push(format!(
                    "{} (id={}, version={}): collector disabled in Server registry",
                    dir.manifest.id, dir.manifest.id, dir.manifest.version
                ));
                continue;
            }
            collector::EnableStatus::DiscoverHashMismatch => {
                errors.push(format!(
                    "{}: discover.sh hash mismatch — script has changed since last registration. \
                     Re-run inspect and re-register.",
                    dir.manifest.id
                ));
                continue;
            }
            collector::EnableStatus::MetricsHashMismatch => {
                errors.push(format!(
                    "{}: metrics.sh hash mismatch — script has changed since last registration. \
                     Re-run inspect and re-register.",
                    dir.manifest.id
                ));
                continue;
            }
        }

        let collector_name = dir.manifest.id.clone();

        // Read scripts into memory.
        let discover_path = dir.dir_path.join(&dir.manifest.discover);
        let metrics_path = dir.dir_path.join(&dir.manifest.metrics);

        let discover_bytes = match std::fs::read(&discover_path) {
            Ok(b) => b,
            Err(e) => {
                errors.push(format!(
                    "{collector_name}: failed to read discover script: {e}"
                ));
                continue;
            }
        };
        let metrics_bytes = match std::fs::read(&metrics_path) {
            Ok(b) => b,
            Err(e) => {
                errors.push(format!(
                    "{collector_name}: failed to read metrics script: {e}"
                ));
                continue;
            }
        };

        // Validate hash again (TOCTOU defense — we already checked in is_enabled).
        let discover_hash = execute::sha256_hex(&discover_bytes);
        let metrics_hash = execute::sha256_hex(&metrics_bytes);

        let reg_entry = match registry
            .iter()
            .find(|r| r.id == dir.manifest.id && r.version == dir.manifest.version && r.enabled)
        {
            Some(e) => e,
            None => {
                errors.push(format!(
                    "{collector_name}: registry entry (id={}, version={}) missing or disabled",
                    dir.manifest.id, dir.manifest.version
                ));
                continue;
            }
        };
        if reg_entry.discover_sha256 != discover_hash {
            errors.push(format!(
                "{collector_name}: discover.sh hash mismatch (registry={}, local={})",
                reg_entry.discover_sha256, discover_hash
            ));
            continue;
        }
        if reg_entry.metrics_sha256 != metrics_hash {
            errors.push(format!(
                "{collector_name}: metrics.sh hash mismatch (registry={}, local={})",
                reg_entry.metrics_sha256, metrics_hash
            ));
            continue;
        }

        // Execute from memory (stdin).
        let discover_output = execute::execute_script(
            &discover_bytes,
            config.timeout_secs,
            config.max_output_bytes,
            64 * 1024,
        )
        .await;
        if discover_output.timed_out {
            errors.push(format!("{collector_name}: discover.sh timed out"));
            continue;
        }
        if discover_output.exit_code != Some(0) {
            errors.push(format!(
                "{collector_name}: discover.sh exited with {:?}: {}",
                discover_output.exit_code,
                discover_output.stderr.trim()
            ));
            continue;
        }

        let discovery = match collector::tsv::parse_discovery(&discover_output.stdout) {
            Ok(d) => d,
            Err(e) => {
                errors.push(format!("{collector_name}: discover parse error: {e}"));
                continue;
            }
        };

        // If discovery reports not_available, skip metrics.
        if matches!(
            discovery.status,
            collector::tsv::CollectorStatus::NotAvailable
        ) {
            if let Some(ref msg) = discovery.status_message {
                errors.push(format!("{collector_name}: {msg}"));
            }
            continue;
        }

        let metrics_output = execute::execute_script(
            &metrics_bytes,
            config.timeout_secs,
            config.max_output_bytes,
            64 * 1024,
        )
        .await;
        if metrics_output.timed_out {
            errors.push(format!("{collector_name}: metrics.sh timed out"));
            continue;
        }
        if metrics_output.exit_code != Some(0) {
            errors.push(format!(
                "{collector_name}: metrics.sh exited with {:?}: {}",
                metrics_output.exit_code,
                metrics_output.stderr.trim()
            ));
            continue;
        }

        let metrics = match collector::tsv::parse_metrics(&metrics_output.stdout) {
            Ok(m) => m,
            Err(e) => {
                errors.push(format!("{collector_name}: metrics parse error: {e}"));
                continue;
            }
        };

        // Merge discovery + metrics into GpuMetrics.
        let mut device_gpus =
            collector::tsv::merge_into_gpu_metrics(&discovery, &metrics, &collector_name, errors);
        gpus.append(&mut device_gpus);
    }
}
