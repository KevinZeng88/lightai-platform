use std::path::PathBuf;

use lightai_agent::gateway_supervisor::{
    gateway_state_path_from_agent_state_path, read_gateway_log, GatewayProcessRecord,
    GatewayProcessSpec,
};

#[test]
fn gateway_state_path_is_separate_from_managed_instance_store() {
    let path = gateway_state_path_from_agent_state_path("data/agent-state.toml");

    assert_eq!(path, PathBuf::from("data/agent-state.toml.gateway.json"));
    assert!(!path.to_string_lossy().contains("managed-instances"));
}

#[test]
fn command_spec_keeps_program_and_args_separate() {
    let spec = valid_spec();

    let command = spec.command_spec().unwrap();

    assert_eq!(
        command.program,
        PathBuf::from("/opt/lightai/bin/lightai-gateway")
    );
    assert_eq!(command.args, vec!["--config", "gateway.toml"]);
    assert_eq!(command.current_dir, PathBuf::from("/opt/lightai"));
    assert_eq!(command.log_path, PathBuf::from("logs/lightai-gateway.log"));
}

#[test]
fn rejects_empty_binary_path() {
    let mut spec = valid_spec();
    spec.binary_path = PathBuf::new();

    let error = spec.validate().unwrap_err();

    assert!(error.contains("binary_path"), "{error}");
}

#[test]
fn rejects_parent_directory_components() {
    let mut spec = valid_spec();
    spec.log_path = PathBuf::from("../gateway.log");

    let error = spec.validate().unwrap_err();

    assert!(error.contains("parent directory"), "{error}");
}

#[test]
fn rejects_root_work_dir() {
    let mut spec = valid_spec();
    spec.work_dir = PathBuf::from("/");

    let error = spec.validate().unwrap_err();

    assert!(error.contains("filesystem root"), "{error}");
}

#[test]
fn rejects_non_loopback_health_url() {
    let mut spec = valid_spec();
    spec.health_url = "http://example.com/health".to_string();

    let error = spec.validate().unwrap_err();

    assert!(
        error.contains("loopback") || error.contains("localhost"),
        "{error}"
    );
}

#[test]
fn rejects_health_url_outside_health_path() {
    let mut spec = valid_spec();
    spec.health_url = "http://127.0.0.1:18082/admin".to_string();

    let error = spec.validate().unwrap_err();

    assert!(error.contains("/health"), "{error}");
}

fn valid_spec() -> GatewayProcessSpec {
    GatewayProcessSpec {
        binary_path: PathBuf::from("/opt/lightai/bin/lightai-gateway"),
        config_path: PathBuf::from("gateway.toml"),
        work_dir: PathBuf::from("/opt/lightai"),
        log_path: PathBuf::from("logs/lightai-gateway.log"),
        state_path: PathBuf::from("data/gateway-state.json"),
        health_url: "http://127.0.0.1:18082/health".to_string(),
    }
}

#[tokio::test]
async fn read_gateway_log_sanitizes_sensitive_content() {
    let dir = unique_temp_path("gateway-log-dir");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lightai-gateway.log");
    let state_path = dir.join("gateway-state.json");
    std::fs::write(&path, "normal line\napi_key = secret\n").unwrap();
    std::fs::write(
        &state_path,
        serde_json::to_string(&GatewayProcessRecord {
            process_id: 0,
            process_start_time: None,
            health_url: "http://127.0.0.1:18082/health".to_string(),
            command: "lightai-gateway --config gateway.toml".to_string(),
            log_path: path.to_string_lossy().into_owned(),
            started_at: 1,
        })
        .unwrap(),
    )
    .unwrap();

    let result = read_gateway_log(&state_path, 1024).await;

    assert_eq!(result.gateway_status, "log_available");
    let log_tail = result.log_tail.unwrap();
    assert!(log_tail.contains("normal line"));
    assert!(log_tail.contains("[redacted"));
    assert!(!log_tail.contains("secret"));

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn read_gateway_log_ignores_untrusted_payload_path_and_uses_state() {
    let dir = unique_temp_path("gateway-log-dir");
    std::fs::create_dir_all(&dir).unwrap();
    let state_log_path = dir.join("lightai-gateway.log");
    let untrusted_log_path = dir.join("other-lightai-gateway.log");
    let state_path = dir.join("gateway-state.json");
    std::fs::write(&state_log_path, "state log\n").unwrap();
    std::fs::write(&untrusted_log_path, "untrusted log\n").unwrap();
    std::fs::write(
        &state_path,
        serde_json::to_string(&GatewayProcessRecord {
            process_id: 0,
            process_start_time: None,
            health_url: "http://127.0.0.1:18082/health".to_string(),
            command: "lightai-gateway --config gateway.toml".to_string(),
            log_path: state_log_path.to_string_lossy().into_owned(),
            started_at: 1,
        })
        .unwrap(),
    )
    .unwrap();

    let result = read_gateway_log(&state_path, 1024).await;

    assert_eq!(result.gateway_status, "log_available");
    let log_tail = result.log_tail.unwrap();
    assert!(log_tail.contains("state log"));
    assert!(!log_tail.contains("untrusted log"));

    let _ = std::fs::remove_dir_all(dir);
}

fn unique_temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
