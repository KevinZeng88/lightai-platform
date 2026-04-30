pub mod custom;
pub mod nvidia;

use crate::config::Config;
use crate::models::GpuMetrics;

pub async fn collect_gpus(config: &Config) -> (Vec<GpuMetrics>, Vec<String>) {
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
