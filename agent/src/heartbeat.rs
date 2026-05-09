use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

use crate::client::ServerClient;
use crate::config::Config;
use crate::gpu;
use crate::managed_process;
use crate::metrics::MetricsCollector;
use crate::models::{AgentConfig, HeartbeatRequest, RegisterRequest};
use crate::platform_log::{self, LogPolicy};
use crate::state::{self, AgentState};

/// Guard: "managed store recovery" log fires at most once per process lifetime.
static MANAGED_STORE_RECOVERY_LOGGED: AtomicBool = AtomicBool::new(false);

pub async fn run(config: Config, runtime_config: Arc<RwLock<RuntimeConfig>>) {
    let mut metrics_collector = MetricsCollector::new();

    loop {
        let snapshot = runtime_config.read().await.clone();
        let sleep_secs = match run_once(&config, &snapshot, &mut metrics_collector).await {
            Ok((next_config, new_registry, fetched_registry)) => {
                let mut runtime = runtime_config.write().await;
                runtime.apply_server_config(next_config);
                // Prefer the dedicated fetch result (freshest), fall back to
                // heartbeat response registry.
                let merged_registry =
                    if let Some(fetched) = fetched_registry.filter(|r| !r.is_empty()) {
                        fetched
                    } else if let Some(hr) = new_registry.filter(|r| !r.is_empty()) {
                        hr
                    } else {
                        runtime.collector_registry.clone()
                    };
                runtime.collector_registry = merged_registry;
                runtime.heartbeat_interval_secs
            }
            Err(error) => {
                tracing::warn!(%error, "heartbeat cycle failed");
                snapshot.heartbeat_interval_secs
            }
        };
        sleep(Duration::from_secs(sleep_secs)).await;
    }
}

async fn run_once(
    config: &Config,
    runtime_config: &RuntimeConfig,
    metrics_collector: &mut MetricsCollector,
) -> anyhow::Result<(
    Option<AgentConfig>,
    Option<Vec<crate::collector::registry::RegistryEntry>>,
    Option<Vec<crate::collector::registry::RegistryEntry>>,
)> {
    let client = ServerClient::new(config.server_url.clone(), config.ca_cert_path.as_deref(), config.insecure_skip_tls_verify)?;
    let mut next_config = None;
    // Recover only persisted managed process records from the managed store.
    // Do not scan externally started processes or recover non-persisted registrations.
    // Each record validated via /proc/{pid}/stat start_time to prevent PID reuse false positives.
    let mut agent_state = match state::load(&config.state_path).await? {
        Some(state) => {
            // Log managed store recovery at most once per process lifetime.
            if !MANAGED_STORE_RECOVERY_LOGGED.swap(true, Ordering::Relaxed) {
                let store_path = managed_process::store_path_from_state_path(&config.state_path);
                if let Ok(records) = managed_process::load(&store_path).await {
                    if !records.is_empty() {
                        let _ = platform_log::append(
                            &runtime_config.log_policy,
                            "lightai-agent.log",
                            "info",
                            &format!(
                                "Agent recovered {} managed instance record(s) after startup",
                                records.len()
                            ),
                        )
                        .await;
                    }
                }
            }
            state
        }
        None => {
            let registered = register(&client, config).await?;
            next_config = Some(registered.agent_config.clone());
            registered.state
        }
    };

    // ── Fetch collector registry from Server before GPU collection ──
    // The registry from the heartbeat response arrives too late for the same
    // cycle (chicken-and-egg).  This dedicated fetch guarantees an up-to-date
    // registry before every GPU collection.
    let mut fetched_registry: Vec<crate::collector::registry::RegistryEntry> =
        runtime_config.collector_registry.clone();
    if runtime_config.collector_root.is_some() {
        match client
            .fetch_collector_registry(&agent_state.agent_token)
            .await
        {
            Ok(entries) => {
                log_registry_fetch_details(&runtime_config.log_policy, &entries).await;
                fetched_registry = entries;
            }
            Err(e) => {
                let _ = platform_log::append(
                    &runtime_config.log_policy,
                    "lightai-agent.log",
                    "error",
                    &format!("GPU collector registry fetch failed: {e}"),
                )
                .await;
                // Fall back to cached registry; if empty, GPU collection will
                // report "registry empty".
            }
        }
    }

    // Build collector config with the freshly-fetched registry.
    let mut collector_cfg = runtime_config.to_collector_config();
    collector_cfg.collector_registry = fetched_registry.clone();
    let (gpus, collector_errors) = gpu::collect_gpus(&collector_cfg).await;
    let collector_status = gpu::compute_collector_status(
        runtime_config.collector_root.as_deref(),
        &collector_errors,
        &gpus,
    );

    // Enhance collector_errors with registry match details when relevant.
    let collector_errors = enhance_collector_errors(collector_errors, &fetched_registry);

    let managed_store_path = managed_process::store_path_from_state_path(&config.state_path);
    let managed_instances = managed_process::reports(Some(&managed_store_path)).await;

    // Log first-probe GPU diagnostic once per process.
    log_first_gpu_probe(
        &runtime_config.log_policy,
        collector_status,
        &gpus,
        &collector_errors,
    )
    .await;

    let running_count = managed_instances
        .iter()
        .filter(|r| r.status == "running")
        .count();
    let failed_count = managed_instances
        .iter()
        .filter(|r| r.status == "failed")
        .count();
    if running_count > 0 || failed_count > 0 {
        let _ = platform_log::append(
            &runtime_config.log_policy,
            "lightai-agent.log",
            "debug",
            &format!("Agent heartbeat reporting managed instance status: running={running_count}, failed={failed_count}"),
        )
        .await;
        if failed_count > 0 {
            let failed_ids = managed_instances
                .iter()
                .filter(|r| r.status == "failed")
                .map(|r| format!("{}（{}）", r.instance_id, r.message))
                .collect::<Vec<_>>()
                .join("，");
            let _ = platform_log::append(
                &runtime_config.log_policy,
                "lightai-agent.log",
                "warn",
                &format!("Managed instance process exited: {failed_ids}"),
            )
            .await;
        }
    }
    let request = HeartbeatRequest {
        node_id: agent_state.node_id.clone(),
        sampled_at: now_unix_secs(),
        metrics: metrics_collector.collect(),
        gpus,
        collector_errors,
        collector_status: collector_status.as_str().to_string(),
        agent_config: runtime_config.to_agent_config(),
        managed_instances,
    };

    let mut new_registry: Option<Vec<crate::collector::registry::RegistryEntry>> = None;
    match client.heartbeat(&agent_state.agent_token, &request).await {
        Ok(response) => {
            if !response.collector_registry.is_empty() {
                new_registry = Some(response.collector_registry);
            }
            if let Some(ref agent_config) = response.agent_config {
                if agent_config.config_version
                    > runtime_config
                        .last_config_updated_at
                        .map_or(0, |_| runtime_config.config_version)
                {
                    let _ = platform_log::append(
                        &runtime_config.log_policy,
                        "lightai-agent.log",
                        "info",
                        &format!(
                            "Agent config updated config_version={}",
                            agent_config.config_version
                        ),
                    )
                    .await;
                }
            }
            Ok((
                response.agent_config.or(next_config),
                new_registry,
                Some(fetched_registry),
            ))
        }
        Err(error) if is_unauthorized(&error) => {
            let _ = platform_log::append(
                &runtime_config.log_policy,
                "lightai-agent.log",
                "warn",
                "Agent token expired, re-registering",
            )
            .await;
            let registered = register(&client, config).await?;
            next_config = Some(registered.agent_config.clone());
            agent_state = registered.state;
            let request = HeartbeatRequest {
                node_id: agent_state.node_id.clone(),
                ..request
            };
            let response = client.heartbeat(&agent_state.agent_token, &request).await?;
            let registry = if response.collector_registry.is_empty() {
                None
            } else {
                Some(response.collector_registry)
            };
            Ok((
                response.agent_config.or(next_config),
                registry,
                Some(fetched_registry),
            ))
        }
        Err(error) => {
            let _ = platform_log::append(
                &runtime_config.log_policy,
                "lightai-agent.log",
                "error",
                &format!("Heartbeat failed: {error}"),
            )
            .await;
            Err(error)
        }
    }
}

struct RegisteredAgent {
    state: AgentState,
    agent_config: AgentConfig,
}

async fn register(client: &ServerClient, config: &Config) -> anyhow::Result<RegisteredAgent> {
    let response = client
        .register(&RegisterRequest {
            name: config.node_name.clone(),
            hostname: std::env::var("HOSTNAME").unwrap_or_else(|_| config.node_name.clone()),
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        })
        .await?;

    let state = AgentState {
        node_id: response.node_id.clone(),
        agent_token: response.agent_token.clone(),
    };
    state::save(&config.state_path, &state).await?;
    let _ = platform_log::append(
        &LogPolicy::default(),
        "lightai-agent.log",
        "info",
        &format!("Agent registered successfully node_id={}", response.node_id),
    )
    .await;
    Ok(RegisteredAgent {
        state,
        agent_config: response.agent_config.unwrap_or_else(|| AgentConfig {
            config_version: 0,
            heartbeat_interval_secs: response.heartbeat_interval_secs,
            ..AgentConfig::default()
        }),
    })
}

pub fn next_interval_secs(
    config_interval_secs: u64,
    registration_interval_secs: Option<u64>,
) -> u64 {
    registration_interval_secs.unwrap_or(config_interval_secs)
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub config_version: i64,
    pub heartbeat_interval_secs: u64,
    pub metrics_sample_interval_secs: u64,
    pub command_timeout_secs: u64,
    pub environment_check_timeout_secs: u64,
    pub allowed_model_dirs: Vec<String>,
    pub collector_timeout_secs: u64,
    pub collector_max_output_bytes: usize,
    pub collector_root: Option<String>,
    pub collector_mode: String,
    pub collector_enabled: Vec<String>,
    pub collector_disabled: Vec<String>,
    pub collector_registry: Vec<crate::collector::registry::RegistryEntry>,
    pub log_policy: LogPolicy,
    pub last_config_updated_at: Option<i64>,
}

impl RuntimeConfig {
    pub fn default_effective() -> Self {
        Self {
            config_version: 0,
            heartbeat_interval_secs: 15,
            metrics_sample_interval_secs: 15,
            command_timeout_secs: 5,
            environment_check_timeout_secs: 5,
            allowed_model_dirs: Vec::new(),
            collector_timeout_secs: 5,
            collector_max_output_bytes: 1024 * 1024,
            collector_root: None,
            collector_mode: "explicit".to_string(),
            collector_enabled: Vec::new(),
            collector_disabled: Vec::new(),
            collector_registry: Vec::new(),
            log_policy: LogPolicy::default(),
            last_config_updated_at: None,
        }
    }

    pub fn from_config(config: &Config) -> Self {
        let mut cfg = Self::default_effective();
        cfg.collector_root = config.collector_root.clone();
        cfg.collector_mode = config.collector_mode.clone();
        cfg.collector_enabled = config.collector_enabled.clone();
        cfg.collector_disabled = config.collector_disabled.clone();
        cfg
    }

    pub fn apply_server_config(&mut self, config: Option<AgentConfig>) {
        let Some(config) = config else {
            return;
        };
        if config.config_version >= self.config_version {
            self.config_version = config.config_version;
            self.heartbeat_interval_secs = config.heartbeat_interval_secs;
            self.metrics_sample_interval_secs = config.metrics_sample_interval_secs;
            self.command_timeout_secs = config.command_timeout_secs;
            self.environment_check_timeout_secs = config.environment_check_timeout_secs;
            self.allowed_model_dirs = config.allowed_model_dirs;
            self.collector_timeout_secs = config.collector_timeout_secs;
            self.collector_max_output_bytes = config.collector_max_output_bytes;
            self.log_policy = config.log_policy;
            self.last_config_updated_at = config.last_config_updated_at;
        }
    }

    pub fn to_agent_config(&self) -> AgentConfig {
        AgentConfig {
            config_version: self.config_version,
            heartbeat_interval_secs: self.heartbeat_interval_secs,
            metrics_sample_interval_secs: self.metrics_sample_interval_secs,
            task_poll_interval_secs: 15,
            config_refresh_interval_secs: 60,
            command_timeout_secs: self.command_timeout_secs,
            environment_check_timeout_secs: self.environment_check_timeout_secs,
            allowed_model_dirs: self.allowed_model_dirs.clone(),
            collector_timeout_secs: self.collector_timeout_secs,
            collector_max_output_bytes: self.collector_max_output_bytes,
            log_policy: self.log_policy.clone(),
            last_config_updated_at: self.last_config_updated_at,
        }
    }

    pub fn to_collector_config(&self) -> gpu::CollectorConfig {
        gpu::CollectorConfig {
            collector_root: self.collector_root.as_ref().map(std::path::PathBuf::from),
            collector_mode: self.collector_mode.clone(),
            collector_enabled: self.collector_enabled.clone(),
            collector_disabled: self.collector_disabled.clone(),
            collector_registry: self.collector_registry.clone(),
            collector_timeout_secs: self.collector_timeout_secs,
            collector_max_output_bytes: self.collector_max_output_bytes,
        }
    }
}

fn is_unauthorized(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<reqwest::Error>()
        .and_then(reqwest::Error::status)
        .is_some_and(|status| status == reqwest::StatusCode::UNAUTHORIZED)
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Guard: first GPU probe log fires at most once per process lifetime.
static FIRST_GPU_PROBE_LOGGED: AtomicBool = AtomicBool::new(false);

async fn log_first_gpu_probe(
    log_policy: &LogPolicy,
    status: gpu::CollectorStatus,
    gpus: &[crate::models::GpuMetrics],
    errors: &[String],
) {
    if FIRST_GPU_PROBE_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }

    match status {
        gpu::CollectorStatus::NoCollectorConfigured => {
            let _ = platform_log::append(
                log_policy,
                "lightai-agent.log",
                "warn",
                "GPU probe: collector_root not configured. GPU collection will not run. Configure [gpu_collectors] and register via Web, then restart Agent.",
            )
            .await;
        }
        gpu::CollectorStatus::CollectorConfiguredButFailed => {
            let summary = errors.join("; ");
            let _ = platform_log::append(
                log_policy,
                "lightai-agent.log",
                "error",
                &format!(
                    "GPU probe: collector execution failed ({} error(s)). Summary: {summary}",
                    errors.len(),
                ),
            )
            .await;
        }
        gpu::CollectorStatus::CollectorOkNoDevices => {
            let _ = platform_log::append(
                log_policy,
                "lightai-agent.log",
                "warn",
                "GPU probe: collector ran successfully but no GPU devices found (GPUs discovered: 0).",
            )
            .await;
        }
        gpu::CollectorStatus::CollectorOkDevicesFound => {
            let mut summary = String::new();
            for gpu in gpus {
                summary.push_str(&format!(
                    "\n  gpu_key={} vendor={} name={} uuid={:?} memory={:?}MB",
                    gpu.gpu_key,
                    gpu.vendor,
                    gpu.name,
                    gpu.uuid,
                    gpu.memory_total_bytes
                        .map(|b| (b / 1_048_576).to_string())
                        .unwrap_or_else(|| "?".to_string()),
                ));
            }
            let _ = platform_log::append(
                log_policy,
                "lightai-agent.log",
                "info",
                &format!("GPU probe: {} GPU(s) discovered{}", gpus.len(), summary,),
            )
            .await;
        }
    }
}

/// Log detailed registry fetch info once per process (on the first successful fetch).
static REGISTRY_FETCH_LOGGED: AtomicBool = AtomicBool::new(false);

async fn log_registry_fetch_details(
    log_policy: &LogPolicy,
    entries: &[crate::collector::registry::RegistryEntry],
) {
    if REGISTRY_FETCH_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }

    if entries.is_empty() {
        let _ = platform_log::append(
            log_policy,
            "lightai-agent.log",
            "warn",
            "GPU collector registry fetch: entries=0. No collectors registered on Server.\
             Register collectors via the Web collector registry page; Agent will auto-fetch.",
        )
        .await;
        return;
    }

    let _ = platform_log::append(
        log_policy,
        "lightai-agent.log",
        "info",
        &format!(
            "GPU collector registry fetch: entries={}, endpoint=/api/agent/collector-registry",
            entries.len(),
        ),
    )
    .await;

    for entry in entries {
        let _ = platform_log::append(
            log_policy,
            "lightai-agent.log",
            "debug",
            &format!(
                "  registry entry: id={}, version={}, enabled={}, \
                 discover_sha256={}, metrics_sha256={}",
                entry.id,
                entry.version,
                entry.enabled,
                &entry.discover_sha256[..entry.discover_sha256.len().min(16)],
                &entry.metrics_sha256[..entry.metrics_sha256.len().min(16)],
            ),
        )
        .await;
    }
}

/// Enhance collector_errors to distinguish "registry empty" from
/// "registry loaded but no match" and "hash mismatch" details.
fn enhance_collector_errors(
    mut errors: Vec<String>,
    registry: &[crate::collector::registry::RegistryEntry],
) -> Vec<String> {
    if registry.is_empty() {
        // Replace the generic "registry is empty" message with a more actionable one.
        if let Some(pos) = errors.iter().position(|e| e.contains("registry is empty")) {
            errors[pos] =
                "collector registry fetch: entries=0 (no collectors registered on Server).\
                 Register collectors via the Web collector registry page."
                    .to_string();
        }
    }
    errors
}
