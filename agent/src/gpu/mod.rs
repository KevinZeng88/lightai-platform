pub mod custom;
pub mod nvidia;

use crate::models::GpuMetrics;

#[derive(Debug, Clone)]
pub struct CollectorConfig {
    pub nvidia_collector_enabled: bool,
    pub custom_collector_script: Option<String>,
    pub collector_timeout_secs: u64,
    pub collector_max_output_bytes: usize,
}

pub async fn collect_gpus(config: &CollectorConfig) -> (Vec<GpuMetrics>, Vec<String>) {
    let mut gpus = Vec::new();
    let mut errors = Vec::new();

    if config.nvidia_collector_enabled {
        match nvidia::collect(
            config.collector_timeout_secs,
            config.collector_max_output_bytes,
        )
        .await
        {
            Ok(mut collected) => gpus.append(&mut collected),
            Err(error) => errors.push(format!("nvidia collector failed: {error}")),
        }
    }

    if let Some(script) = &config.custom_collector_script {
        match custom::collect(
            script,
            config.collector_timeout_secs,
            config.collector_max_output_bytes,
        )
        .await
        {
            Ok(mut collected) => gpus.append(&mut collected),
            Err(error) => errors.push(format!("custom collector failed: {error}")),
        }
    }

    (gpus, errors)
}
