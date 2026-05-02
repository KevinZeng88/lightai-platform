use std::fs;

use lightai_server::config::Config;
use lightai_server::platform_log::{self, LogPolicy};

#[test]
fn loads_server_config_from_toml_file() {
    let path = unique_temp_path("server-config.toml");
    fs::write(
        &path,
        r#"
[server]
listen_addr = "127.0.0.1:18080"

[database]
url = "sqlite://data/test.db"

[metrics]
retention_days = 14
"#,
    )
    .unwrap();

    let config = Config::from_file(&path).unwrap();

    assert_eq!(config.listen_addr, "127.0.0.1:18080");
    assert_eq!(config.database_url, "sqlite://data/test.db");
    assert_eq!(config.metrics_retention_days, 14);

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn server_platform_log_uses_controlled_files_and_filters_sensitive_lines() {
    let dir = std::env::temp_dir().join(format!("lightai-server-log-test-{}", std::process::id()));
    let policy = LogPolicy {
        log_dir: dir.to_string_lossy().to_string(),
        log_level: "info".to_string(),
        log_max_file_bytes: 1024,
        log_retention_files: 2,
        log_retention_days: 7,
    };

    platform_log::append(&policy, "server.log", "info", "正常日志")
        .await
        .unwrap();
    platform_log::append(&policy, "server.log", "info", "authorization: bearer token")
        .await
        .unwrap();
    let content = platform_log::read_tail(&policy, "server.log", 4096)
        .await
        .unwrap();

    assert!(content.contains("正常日志"));
    assert!(!content.contains("bearer token"));
    assert!(platform_log::read_tail(&policy, "../secret", 128)
        .await
        .is_err());

    let _ = fs::remove_file(dir.join("server.log"));
    let _ = fs::remove_dir(dir);
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
