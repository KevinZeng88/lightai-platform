use lightai_server::{db, history_cleanup};
use sqlx::SqlitePool;

fn temp_config_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("lightai-test-{name}-{}", std::process::id()))
}

async fn setup() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    pool
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn hours_ago(hours: i64) -> i64 {
    now() - hours * 3600
}

#[tokio::test]
async fn default_7day_retention_deletes_old_samples_keeps_recent() {
    let pool = setup().await;

    // Insert a node first (FK constraint).
    sqlx::query("INSERT INTO nodes (id, name, hostname, token_hash, token_prefix, registered_at, updated_at) VALUES ('n1', 'n1', 'h1', 'h', 'p', 1, 1)")
        .execute(&pool).await.unwrap();

    // Insert samples: 10 days old (should be deleted) and 1 day old (should stay).
    sqlx::query("INSERT INTO node_metric_samples (node_id, sampled_at, cpu_usage_percent) VALUES ('n1', ?, 50.0)")
        .bind(hours_ago(240))
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO node_metric_samples (node_id, sampled_at, cpu_usage_percent) VALUES ('n1', ?, 80.0)")
        .bind(hours_ago(24))
        .execute(&pool).await.unwrap();

    sqlx::query("INSERT INTO gpu_metric_samples (node_id, gpu_key, sampled_at, vendor, memory_total_bytes) VALUES ('n1', 'nvidia:gpu-0', ?, 'nvidia', 8192)")
        .bind(hours_ago(240))
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO gpu_metric_samples (node_id, gpu_key, sampled_at, vendor, memory_total_bytes) VALUES ('n1', 'nvidia:gpu-0', ?, 'nvidia', 8192)")
        .bind(hours_ago(24))
        .execute(&pool).await.unwrap();

    history_cleanup::cleanup_historical_metrics(&pool, 7)
        .await
        .unwrap();

    let node_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_metric_samples")
        .fetch_one(&pool)
        .await
        .unwrap();
    let gpu_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gpu_metric_samples")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(
        node_count, 1,
        "only the 1-day-old node sample should remain"
    );
    assert_eq!(gpu_count, 1, "only the 1-day-old GPU sample should remain");
}

#[tokio::test]
async fn custom_retention_days_respected() {
    let pool = setup().await;

    sqlx::query("INSERT INTO nodes (id, name, hostname, token_hash, token_prefix, registered_at, updated_at) VALUES ('n1', 'n1', 'h1', 'h', 'p', 1, 1)")
        .execute(&pool).await.unwrap();

    // 2 days old — should survive 3-day retention, be deleted by 1-day retention.
    sqlx::query("INSERT INTO node_metric_samples (node_id, sampled_at, cpu_usage_percent) VALUES ('n1', ?, 50.0)")
        .bind(hours_ago(48))
        .execute(&pool).await.unwrap();

    // Retention = 3 days: sample is 2 days old → should survive.
    history_cleanup::cleanup_historical_metrics(&pool, 3)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_metric_samples")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "2-day-old sample should survive 3-day retention");

    // Retention = 1 day: sample is 2 days old → should be deleted.
    history_cleanup::cleanup_historical_metrics(&pool, 1)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_metric_samples")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "2-day-old sample should be deleted by 1-day retention"
    );
}

#[tokio::test]
async fn cleanup_does_not_touch_state_tables() {
    let pool = setup().await;

    sqlx::query("INSERT INTO nodes (id, name, hostname, token_hash, token_prefix, registered_at, updated_at) VALUES ('n1', 'n1', 'h1', 'h', 'p', 1, 1)")
        .execute(&pool).await.unwrap();

    // Populate state tables.
    sqlx::query(
        "INSERT INTO node_status (node_id, cpu_usage_percent, updated_at) VALUES ('n1', 90.0, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO gpu_status (node_id, gpu_key, vendor, name, collector, updated_at) VALUES ('n1', 'nvidia:gpu0', 'nvidia', 'A100', 'nvidia', 1)")
        .execute(&pool).await.unwrap();

    // Insert an old sample to trigger deletion.
    sqlx::query("INSERT INTO node_metric_samples (node_id, sampled_at, cpu_usage_percent) VALUES ('n1', ?, 50.0)")
        .bind(hours_ago(240))
        .execute(&pool).await.unwrap();

    history_cleanup::cleanup_historical_metrics(&pool, 7)
        .await
        .unwrap();

    let status_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_status")
        .fetch_one(&pool)
        .await
        .unwrap();
    let gpu_status_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gpu_status")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(status_count, 1, "node_status must not be touched");
    assert_eq!(gpu_status_count, 1, "gpu_status must not be touched");
}

#[tokio::test]
async fn config_default_retention_is_7_days() {
    let config = lightai_server::config::Config::default();
    assert_eq!(config.metrics_retention_days, 7);
    assert_eq!(config.history_cleanup_interval_hours, 6);
}

#[tokio::test]
async fn config_rejects_retention_below_1() {
    let path = temp_config_path("bad-retention.toml");
    std::fs::write(
        &path,
        r#"[metrics]
retention_days = 0
"#,
    )
    .unwrap();
    let result = lightai_server::config::Config::from_file(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err());
}

#[tokio::test]
async fn config_parses_custom_retention_and_interval() {
    let path = temp_config_path("custom-metrics.toml");
    std::fs::write(
        &path,
        r#"[metrics]
retention_days = 30
cleanup_interval_hours = 12
"#,
    )
    .unwrap();
    let config = lightai_server::config::Config::from_file(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(config.metrics_retention_days, 30);
    assert_eq!(config.history_cleanup_interval_hours, 12);
}

#[tokio::test]
async fn empty_metrics_section_uses_defaults() {
    let path = temp_config_path("empty-metrics.toml");
    std::fs::write(
        &path,
        r#"[server]
listen_addr = "127.0.0.1:18080"
"#,
    )
    .unwrap();
    let config = lightai_server::config::Config::from_file(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(config.metrics_retention_days, 7);
    assert_eq!(config.history_cleanup_interval_hours, 6);
}
