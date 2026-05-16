use std::path::PathBuf;

use lightai_agent::gateway_supervisor::{
    gateway_state_path_from_agent_state_path, GatewayProcessSpec,
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

fn valid_spec() -> GatewayProcessSpec {
    GatewayProcessSpec {
        binary_path: PathBuf::from("/opt/lightai/bin/lightai-gateway"),
        config_path: PathBuf::from("gateway.toml"),
        work_dir: PathBuf::from("/opt/lightai"),
        log_path: PathBuf::from("logs/lightai-gateway.log"),
        state_path: PathBuf::from("data/gateway-state.json"),
    }
}
