use std::pin::Pin;

use serde::Deserialize;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::GpuCollector;
use crate::models::GpuMetrics;

#[derive(Debug, Deserialize)]
struct CustomOutput {
    #[serde(default)]
    gpus: Vec<CustomGpu>,
}

#[derive(Debug, Deserialize)]
struct CustomGpu {
    index: Option<i64>,
    vendor: Option<String>,
    name: String,
    uuid: Option<String>,
    memory_total_bytes: Option<i64>,
    memory_used_bytes: Option<i64>,
    utilization_percent: Option<f64>,
    temperature_celsius: Option<f64>,
    power_watts: Option<f64>,
}

/// User-supplied script collector.
///
/// Runs a user-provided script that must output JSON on stdout with the structure
/// `{ "gpus": [...] }`. Each GPU object can supply: `index`, `vendor`, `name`,
/// `uuid`, `memory_total_bytes`, `memory_used_bytes`, `utilization_percent`,
/// `temperature_celsius`, `power_watts`.
///
/// Primary use cases:
/// - Domestic / non-NVIDIA GPU adapters that don't yet have a built-in collector.
/// - Custom monitoring setups.
pub struct CustomCollector {
    script: String,
}

impl CustomCollector {
    pub fn new(script: String) -> Self {
        Self { script }
    }
}

impl GpuCollector for CustomCollector {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn collect(
        &self,
        timeout_secs: u64,
        max_output_bytes: usize,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<GpuMetrics>>> + Send + '_>>
    {
        Box::pin(collect(&self.script, timeout_secs, max_output_bytes))
    }
}

/// Standalone collection function maintained for direct callers and tests.
pub async fn collect(
    script_path: &str,
    timeout_secs: u64,
    max_output_bytes: usize,
) -> anyhow::Result<Vec<GpuMetrics>> {
    if script_path.trim().is_empty() {
        return Err(anyhow::anyhow!("custom collector script path is empty"));
    }

    let output = timeout(
        Duration::from_secs(timeout_secs),
        Command::new(script_path).output(),
    )
    .await??;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "custom collector exited with status {}",
            output.status
        ));
    }
    if output.stdout.len() > max_output_bytes {
        return Err(anyhow::anyhow!(
            "custom collector output exceeded size limit"
        ));
    }

    let stdout = String::from_utf8(output.stdout)?;
    parse_custom_output(&stdout)
}

pub fn parse_custom_output(output: &str) -> anyhow::Result<Vec<GpuMetrics>> {
    let parsed: CustomOutput = serde_json::from_str(output)?;
    Ok(parsed
        .gpus
        .into_iter()
        .map(|gpu| {
            let vendor = gpu.vendor.unwrap_or_else(|| "custom".to_string());
            let key_value = gpu
                .uuid
                .clone()
                .or_else(|| gpu.index.map(|index| index.to_string()))
                .unwrap_or_else(|| gpu.name.clone());

            GpuMetrics {
                gpu_key: format!("{vendor}:{key_value}"),
                gpu_index: gpu.index,
                vendor,
                name: gpu.name,
                uuid: gpu.uuid,
                driver_version: None,
                memory_total_bytes: gpu.memory_total_bytes,
                memory_used_bytes: gpu.memory_used_bytes,
                utilization_percent: gpu.utilization_percent,
                temperature_celsius: gpu.temperature_celsius,
                power_watts: gpu.power_watts,
                collector: "custom".to_string(),
                raw_json: None,
            }
        })
        .collect())
}
