use std::pin::Pin;

use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::GpuCollector;
use crate::models::GpuMetrics;

const QUERY_FIELDS: &str = "index,name,uuid,driver_version,memory.total,memory.used,utilization.gpu,temperature.gpu,power.draw";

/// NVIDIA GPU collector using `nvidia-smi`.
///
/// Runs `nvidia-smi --query-gpu=... --format=csv,noheader,nounits` and parses the
/// CSV output. Prefers UUID for `gpu_key` (`nvidia:<uuid>`), falls back to index
/// (`nvidia:<index>`) when UUID is unavailable.
pub struct NvidiaCollector;

impl GpuCollector for NvidiaCollector {
    fn name(&self) -> &'static str {
        "nvidia"
    }

    fn collect(
        &self,
        timeout_secs: u64,
        max_output_bytes: usize,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<GpuMetrics>>> + Send + '_>>
    {
        Box::pin(collect(timeout_secs, max_output_bytes))
    }
}

/// Standalone collection function maintained for direct callers and tests.
pub async fn collect(
    timeout_secs: u64,
    max_output_bytes: usize,
) -> anyhow::Result<Vec<GpuMetrics>> {
    let output = timeout(
        Duration::from_secs(timeout_secs),
        Command::new("nvidia-smi")
            .arg(format!("--query-gpu={QUERY_FIELDS}"))
            .arg("--format=csv,noheader,nounits")
            .output(),
    )
    .await??;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "nvidia-smi exited with status {}",
            output.status
        ));
    }
    if output.stdout.len() > max_output_bytes {
        return Err(anyhow::anyhow!("nvidia-smi output exceeded size limit"));
    }

    let stdout = String::from_utf8(output.stdout)?;
    parse_nvidia_smi_csv(&stdout)
}

/// Parse `nvidia-smi` CSV output into [`GpuMetrics`] list.
pub fn parse_nvidia_smi_csv(output: &str) -> anyhow::Result<Vec<GpuMetrics>> {
    let mut gpus = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if parts.len() != 9 {
            return Err(anyhow::anyhow!("unexpected nvidia-smi column count"));
        }

        let index = parse_i64(parts[0]);
        let uuid = empty_to_none(parts[2]);
        let gpu_key = uuid
            .as_ref()
            .map(|value| format!("nvidia:{value}"))
            .unwrap_or_else(|| format!("nvidia:{}", parts[0]));

        gpus.push(GpuMetrics {
            gpu_key,
            gpu_index: index,
            vendor: "nvidia".to_string(),
            name: parts[1].to_string(),
            uuid,
            driver_version: empty_to_none(parts[3]),
            memory_total_bytes: parse_mib_to_bytes(parts[4]),
            memory_used_bytes: parse_mib_to_bytes(parts[5]),
            utilization_percent: parse_f64(parts[6]),
            temperature_celsius: parse_f64(parts[7]),
            power_watts: parse_f64(parts[8]),
            collector: "nvidia".to_string(),
            raw_json: None,
        });
    }
    Ok(gpus)
}

fn empty_to_none(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("[not supported]") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_i64(value: &str) -> Option<i64> {
    value.parse().ok()
}

fn parse_f64(value: &str) -> Option<f64> {
    value.parse().ok()
}

fn parse_mib_to_bytes(value: &str) -> Option<i64> {
    value
        .parse::<i64>()
        .ok()
        .map(|mib| mib.saturating_mul(1024 * 1024))
}
