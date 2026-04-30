use std::fs;

use lightai_agent::config::Config;
use lightai_agent::heartbeat;
use lightai_agent::metrics::MetricsCollector;
use lightai_agent::state::{self, AgentState};

#[test]
fn loads_agent_config_from_toml_file() {
    let path = unique_temp_path("agent-config.toml");
    fs::write(
        &path,
        r#"
[agent]
listen_addr = "127.0.0.1:18081"
server_url = "http://127.0.0.1:18080"
node_name = "gpu-node-test"
heartbeat_interval_secs = 30
state_path = "data/test-agent-state.toml"

[collectors.nvidia]
enabled = false

[collectors.custom]
script_path = "/opt/lightai/custom-gpu"
timeout_secs = 9
max_output_bytes = 2048
"#,
    )
    .unwrap();

    let config = Config::from_file(&path).unwrap();

    assert_eq!(config.listen_addr, "127.0.0.1:18081");
    assert_eq!(config.server_url, "http://127.0.0.1:18080");
    assert_eq!(config.node_name, "gpu-node-test");
    assert_eq!(config.heartbeat_interval_secs, 30);
    assert_eq!(config.state_path, "data/test-agent-state.toml");
    assert!(!config.nvidia_collector_enabled);
    assert_eq!(
        config.custom_collector_script.as_deref(),
        Some("/opt/lightai/custom-gpu")
    );
    assert_eq!(config.collector_timeout_secs, 9);
    assert_eq!(config.collector_max_output_bytes, 2048);

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn saves_agent_state_with_private_permissions_on_unix() {
    let path = unique_temp_path("agent-state.toml");
    let state = AgentState {
        node_id: "node-1".to_string(),
        agent_token: "secret-token".to_string(),
    };

    state::save(path.to_str().unwrap(), &state).await.unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    let _ = fs::remove_file(path);
}

#[test]
fn metrics_collector_reuses_system_state() {
    let mut collector = MetricsCollector::new();

    let first = collector.collect();
    let second = collector.collect();

    assert!(first.cpu_usage_percent.is_some());
    assert!(second.cpu_usage_percent.is_some());
}

#[test]
fn registration_interval_overrides_config_interval_for_next_sleep() {
    assert_eq!(heartbeat::next_interval_secs(15, Some(30)), 30);
    assert_eq!(heartbeat::next_interval_secs(15, None), 15);
}

fn unique_temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "lightai-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
