use sqlx::SqlitePool;

/// Delete historical metric samples older than `retention_days`.
///
/// Returns the total number of rows deleted (node + GPU).
/// Errors are logged; the caller should not crash the server on failure.
pub async fn cleanup_historical_metrics(
    pool: &SqlitePool,
    retention_days: u32,
) -> anyhow::Result<u64> {
    let cutoff = now_unix_secs() - (retention_days as i64) * 86400;

    let node_deleted = sqlx::query("DELETE FROM node_metric_samples WHERE sampled_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    let gpu_deleted = sqlx::query("DELETE FROM gpu_metric_samples WHERE sampled_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;

    let total =
        node_deleted.rows_affected() + gpu_deleted.rows_affected();

    tracing::info!(
        retention_days,
        cutoff,
        node_deleted = node_deleted.rows_affected(),
        gpu_deleted = gpu_deleted.rows_affected(),
        "cleaned up historical metric samples"
    );

    Ok(total)
}

pub fn spawn_cleanup_task(
    pool: SqlitePool,
    retention_days: u32,
    interval_hours: u32,
) {
    tokio::spawn(async move {
        // Run first cleanup after a short delay so the server is fully up.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;

        loop {
            match cleanup_historical_metrics(&pool, retention_days).await {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(count, "history cleanup completed");
                    }
                }
                Err(error) => {
                    tracing::error!(
                        %error,
                        "history cleanup failed; will retry at next interval"
                    );
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(
                interval_hours as u64 * 3600,
            ))
            .await;
        }
    });
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
