use std::fs;

use lightai_server::config::Config;

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

fn unique_temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "lightai-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
