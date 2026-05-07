use std::collections::HashSet;

use crate::models::GpuMetrics;

/// Parsed collector status from a STATUS row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectorStatus {
    /// Script executed successfully (devices may or may not be present).
    Ok,
    /// The collector's tool is not available on this machine (e.g. nvidia-smi not found).
    NotAvailable,
    /// Script executed successfully but some data was incomplete.
    Partial,
    /// Script reported an error.
    Error,
}

impl CollectorStatus {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ok" => Some(Self::Ok),
            "not_available" => Some(Self::NotAvailable),
            "partial" => Some(Self::Partial),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// Parsed device from a DEVICE row during discovery.
#[derive(Debug, Clone)]
pub struct DiscoveryDevice {
    pub device_key: String,
    pub vendor: String,
    pub index: Option<i64>,
    pub name: String,
    pub uuid: Option<String>,
    pub pci_bus_id: Option<String>,
    pub driver_version: Option<String>,
    pub message: Option<String>,
}

/// Parsed metric from a METRIC row.
#[derive(Debug, Clone)]
pub struct MetricSample {
    pub device_key: String,
    pub memory_total_mb: Option<f64>,
    pub memory_used_mb: Option<f64>,
    pub memory_free_mb: Option<f64>,
    pub utilization_percent: Option<f64>,
    pub memory_utilization_percent: Option<f64>,
    pub temperature_c: Option<f64>,
    pub power_w: Option<f64>,
    pub health_status: String,
    pub message: Option<String>,
}

/// Result of parsing discovery output.
#[derive(Debug)]
pub struct DiscoveryResult {
    pub status: CollectorStatus,
    pub status_message: Option<String>,
    pub vendor: Option<String>,
    pub collector: Option<String>,
    pub devices: Vec<DiscoveryDevice>,
}

/// Result of parsing metrics output.
#[derive(Debug)]
pub struct MetricsResult {
    pub status: CollectorStatus,
    pub status_message: Option<String>,
    pub vendor: Option<String>,
    pub collector: Option<String>,
    pub samples: Vec<MetricSample>,
}

/// Parse discovery TSV output into a [`DiscoveryResult`].
///
/// Returns an error if the output is completely unparseable (e.g. no STATUS row,
/// invalid schema version). Individual row errors are logged but don't cause
/// the whole parse to fail.
pub fn parse_discovery(tsv: &str) -> anyhow::Result<DiscoveryResult> {
    let mut status: Option<CollectorStatus> = None;
    let mut status_message: Option<String> = None;
    let mut vendor: Option<String> = None;
    let mut collector: Option<String> = None;
    let mut devices = Vec::new();
    let mut seen_keys = HashSet::new();

    for line in tsv.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();

        match cols.first().copied() {
            Some("STATUS") => {
                if cols.len() != 6 {
                    anyhow::bail!("STATUS row must have 6 columns, got {}", cols.len());
                }
                if cols[1] != "1" {
                    anyhow::bail!("unsupported schema_version '{}' in STATUS row", cols[1]);
                }
                status =
                    Some(CollectorStatus::parse(cols[2]).ok_or_else(|| {
                        anyhow::anyhow!("unknown collector status '{}'", cols[2])
                    })?);
                vendor = empty_to_opt(cols[3]);
                collector = empty_to_opt(cols[4]);
                status_message = empty_to_opt(cols[5]);
            }
            Some("DEVICE") => {
                if cols.len() != 10 {
                    tracing::warn!(
                        "DEVICE row has {} columns (expected 10), skipping",
                        cols.len()
                    );
                    continue;
                }
                if cols[1] != "1" {
                    tracing::warn!(
                        "unsupported schema_version '{}' in DEVICE row, skipping",
                        cols[1]
                    );
                    continue;
                }
                let device_key = cols[2].trim().to_string();
                if device_key.is_empty() {
                    tracing::warn!("DEVICE row missing device_key, skipping");
                    continue;
                }
                if !seen_keys.insert(device_key.clone()) {
                    tracing::warn!("duplicate device_key '{}' in discovery output", device_key);
                    continue;
                }
                devices.push(DiscoveryDevice {
                    device_key,
                    vendor: cols[3].trim().to_string(),
                    index: parse_opt_i64(cols[4]),
                    name: cols[5].trim().to_string(),
                    uuid: empty_to_opt(cols[6]),
                    pci_bus_id: empty_to_opt(cols[7]),
                    driver_version: empty_to_opt(cols[8]),
                    message: empty_to_opt(cols[9]),
                });
            }
            Some("METRIC") => {
                // METRIC rows in discovery output are ignored (discovery shouldn't produce them).
                tracing::debug!("ignoring METRIC row in discovery output");
            }
            Some(unknown) => {
                tracing::warn!("unknown TSV record type '{}', skipping", unknown);
            }
            None => {}
        }
    }

    let status = status.ok_or_else(|| anyhow::anyhow!("discovery output missing STATUS row"))?;

    Ok(DiscoveryResult {
        status,
        status_message,
        vendor,
        collector,
        devices,
    })
}

/// Parse metrics TSV output into a [`MetricsResult`].
pub fn parse_metrics(tsv: &str) -> anyhow::Result<MetricsResult> {
    let mut status: Option<CollectorStatus> = None;
    let mut status_message: Option<String> = None;
    let mut vendor: Option<String> = None;
    let mut collector: Option<String> = None;
    let mut samples = Vec::new();
    let mut seen_keys = HashSet::new();

    for line in tsv.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();

        match cols.first().copied() {
            Some("STATUS") => {
                if cols.len() != 6 {
                    anyhow::bail!("STATUS row must have 6 columns, got {}", cols.len());
                }
                if cols[1] != "1" {
                    anyhow::bail!("unsupported schema_version '{}' in STATUS row", cols[1]);
                }
                status =
                    Some(CollectorStatus::parse(cols[2]).ok_or_else(|| {
                        anyhow::anyhow!("unknown collector status '{}'", cols[2])
                    })?);
                vendor = empty_to_opt(cols[3]);
                collector = empty_to_opt(cols[4]);
                status_message = empty_to_opt(cols[5]);
            }
            Some("DEVICE") => {
                tracing::debug!("ignoring DEVICE row in metrics output");
            }
            Some("METRIC") => {
                if cols.len() != 12 {
                    tracing::warn!(
                        "METRIC row has {} columns (expected 12), skipping",
                        cols.len()
                    );
                    continue;
                }
                if cols[1] != "1" {
                    tracing::warn!(
                        "unsupported schema_version '{}' in METRIC row, skipping",
                        cols[1]
                    );
                    continue;
                }
                let device_key = cols[2].trim().to_string();
                if device_key.is_empty() {
                    tracing::warn!("METRIC row missing device_key, skipping");
                    continue;
                }
                if !seen_keys.insert(device_key.clone()) {
                    tracing::warn!("duplicate device_key '{}' in metrics output", device_key);
                    continue;
                }
                let health_status = cols[10].trim().to_string();
                if health_status.is_empty() {
                    tracing::warn!(
                        "METRIC row missing health_status for device_key '{}', skipping",
                        device_key
                    );
                    continue;
                }
                samples.push(MetricSample {
                    device_key,
                    memory_total_mb: parse_opt_f64(cols[3]),
                    memory_used_mb: parse_opt_f64(cols[4]),
                    memory_free_mb: parse_opt_f64(cols[5]),
                    utilization_percent: parse_opt_f64(cols[6]),
                    memory_utilization_percent: parse_opt_f64(cols[7]),
                    temperature_c: parse_opt_f64(cols[8]),
                    power_w: parse_opt_f64(cols[9]),
                    health_status,
                    message: empty_to_opt(cols[11]),
                });
            }
            Some(unknown) => {
                tracing::warn!("unknown TSV record type '{}', skipping", unknown);
            }
            None => {}
        }
    }

    let status = status.ok_or_else(|| anyhow::anyhow!("metrics output missing STATUS row"))?;

    Ok(MetricsResult {
        status,
        status_message,
        vendor,
        collector,
        samples,
    })
}

/// Merge discovery and metrics results into [`GpuMetrics`].
///
/// - Discovery provides device identity fields.
/// - Metrics provides runtime metric fields.
/// - Device keys that appear in metrics but not discovery are still included,
///   but marked with a warning via `collector_errors`.
pub fn merge_into_gpu_metrics(
    discovery: &DiscoveryResult,
    metrics: &MetricsResult,
    collector_name: &str,
    errors: &mut Vec<String>,
) -> Vec<GpuMetrics> {
    let mut gpus = Vec::new();
    let mut matched_metric_keys = HashSet::new();

    for device in &discovery.devices {
        let metric = metrics
            .samples
            .iter()
            .find(|m| m.device_key == device.device_key);

        if let Some(m) = metric {
            matched_metric_keys.insert(m.device_key.clone());
        }

        gpus.push(GpuMetrics {
            gpu_key: device.device_key.clone(),
            gpu_index: device.index,
            vendor: device.vendor.clone(),
            name: device.name.clone(),
            uuid: device.uuid.clone(),
            driver_version: device.driver_version.clone(),
            memory_total_bytes: metric
                .and_then(|m| m.memory_total_mb)
                .map(|mb| (mb * 1024.0 * 1024.0) as i64),
            memory_used_bytes: metric
                .and_then(|m| m.memory_used_mb)
                .map(|mb| (mb * 1024.0 * 1024.0) as i64),
            utilization_percent: metric.and_then(|m| m.utilization_percent),
            temperature_celsius: metric.and_then(|m| m.temperature_c),
            power_watts: metric.and_then(|m| m.power_w),
            collector: collector_name.to_string(),
            raw_json: None,
        });
    }

    // Metrics-only devices (no discovery match).
    for m in &metrics.samples {
        if !matched_metric_keys.contains(&m.device_key) {
            errors.push(format!(
                "device_key '{}' found in metrics but not in discovery for collector '{}'",
                m.device_key, collector_name
            ));
            gpus.push(GpuMetrics {
                gpu_key: m.device_key.clone(),
                gpu_index: None,
                vendor: metrics
                    .vendor
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                name: m.device_key.clone(),
                uuid: None,
                driver_version: None,
                memory_total_bytes: m.memory_total_mb.map(|mb| (mb * 1024.0 * 1024.0) as i64),
                memory_used_bytes: m.memory_used_mb.map(|mb| (mb * 1024.0 * 1024.0) as i64),
                utilization_percent: m.utilization_percent,
                temperature_celsius: m.temperature_c,
                power_watts: m.power_w,
                collector: collector_name.to_string(),
                raw_json: None,
            });
        }
    }

    gpus
}

fn empty_to_opt(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_opt_i64(s: &str) -> Option<i64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse().ok()
}

fn parse_opt_f64(s: &str) -> Option<f64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> String {
        s.replace("\\t", "\t").replace("\\n", "\n")
    }

    #[test]
    fn parses_discovery_with_devices() {
        let tsv = ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\nDEVICE\\t1\\tnvidia:GPU-abc\\tnvidia\\t0\\tNVIDIA A10\\tGPU-abc\\t0000:3B:00.0\\t535.129.03\\t\\n");
        let result = parse_discovery(&tsv).unwrap();
        assert_eq!(result.status, CollectorStatus::Ok);
        assert_eq!(result.devices.len(), 1);
        assert_eq!(result.devices[0].device_key, "nvidia:GPU-abc");
        assert_eq!(result.devices[0].vendor, "nvidia");
        assert_eq!(result.devices[0].index, Some(0));
        assert_eq!(result.devices[0].name, "NVIDIA A10");
        assert_eq!(result.devices[0].uuid, Some("GPU-abc".to_string()));
        assert_eq!(
            result.devices[0].pci_bus_id,
            Some("0000:3B:00.0".to_string())
        );
        assert_eq!(
            result.devices[0].driver_version,
            Some("535.129.03".to_string())
        );
    }

    #[test]
    fn parses_discovery_no_devices() {
        let tsv = ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\n");
        let result = parse_discovery(&tsv).unwrap();
        assert_eq!(result.status, CollectorStatus::Ok);
        assert!(result.devices.is_empty());
    }

    #[test]
    fn parses_discovery_not_available() {
        let tsv = ts("STATUS\\t1\\tnot_available\\tnvidia\\tnvidia-r535\\tnvidia-smi not found\\n");
        let result = parse_discovery(&tsv).unwrap();
        assert_eq!(result.status, CollectorStatus::NotAvailable);
        assert_eq!(
            result.status_message,
            Some("nvidia-smi not found".to_string())
        );
    }

    #[test]
    fn parses_discovery_error() {
        let tsv = ts("STATUS\\t1\\terror\\tnvidia\\tnvidia-r535\\tnvidia-smi query failed\\n");
        let result = parse_discovery(&tsv).unwrap();
        assert_eq!(result.status, CollectorStatus::Error);
    }

    #[test]
    fn discovery_missing_status_is_error() {
        let tsv =
            ts("DEVICE\\t1\\tnvidia:GPU-abc\\tnvidia\\t0\\tA10\\tGPU-abc\\t0000:3B\\t535\\t\\n");
        assert!(parse_discovery(&tsv).is_err());
    }

    #[test]
    fn discovery_rejects_bad_schema_version() {
        let tsv = ts("STATUS\\t2\\tok\\tnvidia\\tnvidia-r535\\t\\n");
        assert!(parse_discovery(&tsv).is_err());
    }

    #[test]
    fn discovery_skips_duplicate_device_key() {
        let tsv = ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\nDEVICE\\t1\\tnvidia:GPU-abc\\tnvidia\\t0\\tA10\\tGPU-abc\\t0000:3B\\t535\\t\\nDEVICE\\t1\\tnvidia:GPU-abc\\tnvidia\\t1\\tA10\\tGPU-abc\\t0000:3B\\t535\\t\\n");
        let result = parse_discovery(&tsv).unwrap();
        assert_eq!(result.devices.len(), 1);
    }

    #[test]
    fn discovery_skips_device_missing_key() {
        let tsv =
            ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\nDEVICE\\t1\\t\\t\\t\\t\\t\\t\\t\\t\\n");
        let result = parse_discovery(&tsv).unwrap();
        assert!(result.devices.is_empty());
    }

    #[test]
    fn parses_metrics_full() {
        let tsv = ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\nMETRIC\\t1\\tnvidia:GPU-abc\\t81920\\t12345\\t69575\\t72\\t18\\t61\\t285.4\\tok\\t\\n");
        let result = parse_metrics(&tsv).unwrap();
        assert_eq!(result.status, CollectorStatus::Ok);
        assert_eq!(result.samples.len(), 1);
        let m = &result.samples[0];
        assert_eq!(m.device_key, "nvidia:GPU-abc");
        assert_eq!(m.memory_total_mb, Some(81920.0));
        assert_eq!(m.memory_used_mb, Some(12345.0));
        assert_eq!(m.utilization_percent, Some(72.0));
        assert_eq!(m.memory_utilization_percent, Some(18.0));
        assert_eq!(m.temperature_c, Some(61.0));
        assert_eq!(m.power_w, Some(285.4));
        assert_eq!(m.health_status, "ok");
    }

    #[test]
    fn metrics_missing_health_status_skips_row() {
        let tsv = ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\nMETRIC\\t1\\tnvidia:GPU-abc\\t81920\\t12345\\t69575\\t72\\t18\\t61\\t285.4\\t\\t\\n");
        let result = parse_metrics(&tsv).unwrap();
        assert!(result.samples.is_empty());
    }

    #[test]
    fn metrics_partial_status() {
        let tsv = ts("STATUS\\t1\\tpartial\\tmetax\\tmetax-c500\\tsome fields unavailable\\n");
        let result = parse_metrics(&tsv).unwrap();
        assert_eq!(result.status, CollectorStatus::Partial);
    }

    #[test]
    fn merge_combines_discovery_and_metrics() {
        let discovery = parse_discovery(&ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\nDEVICE\\t1\\tnvidia:GPU-abc\\tnvidia\\t0\\tA10\\tGPU-abc\\t0000:3B:00.0\\t535.129.03\\t\\n")).unwrap();
        let metrics = parse_metrics(&ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\nMETRIC\\t1\\tnvidia:GPU-abc\\t81920\\t12345\\t69575\\t72\\t18\\t61\\t285.4\\tok\\t\\n")).unwrap();
        let mut errors = Vec::new();
        let gpus = merge_into_gpu_metrics(&discovery, &metrics, "nvidia-r535", &mut errors);

        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].gpu_key, "nvidia:GPU-abc");
        assert_eq!(gpus[0].vendor, "nvidia");
        assert_eq!(gpus[0].name, "A10");
        assert_eq!(gpus[0].memory_total_bytes, Some(81920 * 1024 * 1024));
        assert_eq!(gpus[0].memory_used_bytes, Some(12345 * 1024 * 1024));
        assert_eq!(gpus[0].utilization_percent, Some(72.0));
        assert_eq!(gpus[0].temperature_celsius, Some(61.0));
        assert_eq!(gpus[0].power_watts, Some(285.4));
        assert!(errors.is_empty());
    }

    #[test]
    fn merge_reports_metrics_only_device() {
        let discovery =
            parse_discovery(&ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\n")).unwrap();
        let metrics = parse_metrics(&ts("STATUS\\t1\\tok\\tnvidia\\tnvidia-r535\\t\\nMETRIC\\t1\\tnvidia:GPU-xyz\\t81920\\t0\\t81920\\t0\\t0\\t30\\t0\\tok\\t\\n")).unwrap();
        let mut errors = Vec::new();
        let gpus = merge_into_gpu_metrics(&discovery, &metrics, "nvidia-r535", &mut errors);

        assert_eq!(gpus.len(), 1);
        assert!(errors.len() == 1);
        assert!(errors[0].contains("metrics but not in discovery"));
    }
}
