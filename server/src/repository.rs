use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::auth;
use crate::models::{
    GpuMetricSample, GpuMetricSamplesResponse, GpuMetrics, GpuView, HeartbeatRequest,
    NodeListResponse, NodeMetricSample, NodeMetricSamplesResponse, NodeMetrics, NodeView,
    RegisterRequest, RegisterResponse,
};

const HEARTBEAT_INTERVAL_SECS: u64 = 15;
const ONLINE_THRESHOLD_SECS: i64 = 60;

pub async fn register_node(
    pool: &SqlitePool,
    request: RegisterRequest,
) -> anyhow::Result<RegisterResponse> {
    let now = now_unix_secs();
    let node_id = Uuid::new_v4().to_string();
    let token = auth::generate_agent_token();
    let token_hash = auth::hash_token(&token);
    let token_prefix = auth::token_prefix(&token);

    sqlx::query(
        r#"
        INSERT INTO nodes (
            id, name, hostname, agent_version, os, arch,
            token_hash, token_prefix, registered_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&node_id)
    .bind(request.name)
    .bind(request.hostname)
    .bind(request.agent_version)
    .bind(request.os)
    .bind(request.arch)
    .bind(token_hash)
    .bind(token_prefix)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(RegisterResponse {
        node_id,
        agent_token: token,
        heartbeat_interval_secs: HEARTBEAT_INTERVAL_SECS,
    })
}

pub async fn authenticate_node(
    pool: &SqlitePool,
    node_id: &str,
    token: &str,
) -> anyhow::Result<bool> {
    let hash: Option<String> = sqlx::query_scalar("SELECT token_hash FROM nodes WHERE id = ?")
        .bind(node_id)
        .fetch_optional(pool)
        .await?;

    Ok(hash
        .as_deref()
        .is_some_and(|expected| auth::verify_token(token, expected)))
}

pub async fn record_heartbeat(pool: &SqlitePool, request: HeartbeatRequest) -> anyhow::Result<()> {
    let now = now_unix_secs();
    let errors_json = serde_json::to_string(&request.collector_errors)?;
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        UPDATE nodes
        SET updated_at = ?, last_heartbeat_at = ?
        WHERE id = ?
        "#,
    )
    .bind(now)
    .bind(request.sampled_at)
    .bind(&request.node_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO node_status (
            node_id, cpu_usage_percent, memory_total_bytes, memory_used_bytes,
            disk_total_bytes, disk_used_bytes, collector_errors_json, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(node_id) DO UPDATE SET
            cpu_usage_percent = excluded.cpu_usage_percent,
            memory_total_bytes = excluded.memory_total_bytes,
            memory_used_bytes = excluded.memory_used_bytes,
            disk_total_bytes = excluded.disk_total_bytes,
            disk_used_bytes = excluded.disk_used_bytes,
            collector_errors_json = excluded.collector_errors_json,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&request.node_id)
    .bind(request.metrics.cpu_usage_percent)
    .bind(request.metrics.memory_total_bytes)
    .bind(request.metrics.memory_used_bytes)
    .bind(request.metrics.disk_total_bytes)
    .bind(request.metrics.disk_used_bytes)
    .bind(errors_json)
    .bind(request.sampled_at)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO node_metric_samples (
            node_id, sampled_at, cpu_usage_percent, memory_total_bytes, memory_used_bytes,
            disk_total_bytes, disk_used_bytes
        )
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&request.node_id)
    .bind(request.sampled_at)
    .bind(request.metrics.cpu_usage_percent)
    .bind(request.metrics.memory_total_bytes)
    .bind(request.metrics.memory_used_bytes)
    .bind(request.metrics.disk_total_bytes)
    .bind(request.metrics.disk_used_bytes)
    .execute(&mut *tx)
    .await?;

    for gpu in request.gpus {
        upsert_gpu_status(&mut tx, &request.node_id, request.sampled_at, &gpu).await?;
        insert_gpu_sample(&mut tx, &request.node_id, request.sampled_at, &gpu).await?;
    }

    tx.commit().await?;
    Ok(())
}

async fn upsert_gpu_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    node_id: &str,
    sampled_at: i64,
    gpu: &GpuMetrics,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO gpu_status (
            node_id, gpu_key, gpu_index, vendor, name, uuid, driver_version,
            memory_total_bytes, memory_used_bytes, utilization_percent,
            temperature_celsius, power_watts, collector, raw_json, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(node_id, gpu_key) DO UPDATE SET
            gpu_index = excluded.gpu_index,
            vendor = excluded.vendor,
            name = excluded.name,
            uuid = excluded.uuid,
            driver_version = excluded.driver_version,
            memory_total_bytes = excluded.memory_total_bytes,
            memory_used_bytes = excluded.memory_used_bytes,
            utilization_percent = excluded.utilization_percent,
            temperature_celsius = excluded.temperature_celsius,
            power_watts = excluded.power_watts,
            collector = excluded.collector,
            raw_json = excluded.raw_json,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(node_id)
    .bind(&gpu.gpu_key)
    .bind(gpu.gpu_index)
    .bind(&gpu.vendor)
    .bind(&gpu.name)
    .bind(&gpu.uuid)
    .bind(&gpu.driver_version)
    .bind(gpu.memory_total_bytes)
    .bind(gpu.memory_used_bytes)
    .bind(gpu.utilization_percent)
    .bind(gpu.temperature_celsius)
    .bind(gpu.power_watts)
    .bind(&gpu.collector)
    .bind(&gpu.raw_json)
    .bind(sampled_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_gpu_sample(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    node_id: &str,
    sampled_at: i64,
    gpu: &GpuMetrics,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO gpu_metric_samples (
            node_id, gpu_key, sampled_at, vendor, memory_total_bytes,
            memory_used_bytes, utilization_percent, temperature_celsius, power_watts
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(node_id)
    .bind(&gpu.gpu_key)
    .bind(sampled_at)
    .bind(&gpu.vendor)
    .bind(gpu.memory_total_bytes)
    .bind(gpu.memory_used_bytes)
    .bind(gpu.utilization_percent)
    .bind(gpu.temperature_celsius)
    .bind(gpu.power_watts)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn list_nodes(pool: &SqlitePool) -> anyhow::Result<NodeListResponse> {
    let node_rows = sqlx::query(
        r#"
        SELECT n.id, n.name, n.hostname, n.agent_version, n.os, n.arch,
               n.registered_at, n.updated_at, n.last_heartbeat_at,
               s.cpu_usage_percent, s.memory_total_bytes, s.memory_used_bytes,
               s.disk_total_bytes, s.disk_used_bytes
        FROM nodes n
        LEFT JOIN node_status s ON s.node_id = n.id
        ORDER BY n.name
        "#,
    )
    .fetch_all(pool)
    .await?;

    let now = now_unix_secs();
    let mut nodes = Vec::with_capacity(node_rows.len());
    for row in node_rows {
        let node_id: String = row.get("id");
        let last_heartbeat_at: Option<i64> = row.get("last_heartbeat_at");
        let gpus = list_gpus(pool, &node_id).await?;
        let metrics = row
            .try_get::<Option<f64>, _>("cpu_usage_percent")
            .ok()
            .flatten()
            .map(|cpu_usage_percent| NodeMetrics {
                cpu_usage_percent: Some(cpu_usage_percent),
                memory_total_bytes: row.get("memory_total_bytes"),
                memory_used_bytes: row.get("memory_used_bytes"),
                disk_total_bytes: row.get("disk_total_bytes"),
                disk_used_bytes: row.get("disk_used_bytes"),
            });

        let status = match last_heartbeat_at {
            Some(last_seen) if now - last_seen <= ONLINE_THRESHOLD_SECS => "online",
            Some(_) => "offline",
            None => "registered",
        }
        .to_string();

        nodes.push(NodeView {
            id: node_id,
            name: row.get("name"),
            hostname: row.get("hostname"),
            agent_version: row.get("agent_version"),
            os: row.get("os"),
            arch: row.get("arch"),
            status,
            registered_at: row.get("registered_at"),
            updated_at: row.get("updated_at"),
            last_heartbeat_at,
            metrics,
            gpus,
        });
    }

    Ok(NodeListResponse { nodes })
}

async fn list_gpus(pool: &SqlitePool, node_id: &str) -> anyhow::Result<Vec<GpuView>> {
    let rows = sqlx::query(
        r#"
        SELECT gpu_key, gpu_index, vendor, name, uuid, driver_version,
               memory_total_bytes, memory_used_bytes, utilization_percent,
               temperature_celsius, power_watts, collector, updated_at
        FROM gpu_status
        WHERE node_id = ?
        ORDER BY gpu_index, gpu_key
        "#,
    )
    .bind(node_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| GpuView {
            gpu_key: row.get("gpu_key"),
            gpu_index: row.get("gpu_index"),
            vendor: row.get("vendor"),
            name: row.get("name"),
            uuid: row.get("uuid"),
            driver_version: row.get("driver_version"),
            memory_total_bytes: row.get("memory_total_bytes"),
            memory_used_bytes: row.get("memory_used_bytes"),
            utilization_percent: row.get("utilization_percent"),
            temperature_celsius: row.get("temperature_celsius"),
            power_watts: row.get("power_watts"),
            collector: row.get("collector"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

pub async fn node_metric_samples(
    pool: &SqlitePool,
    node_id: &str,
    from: i64,
    to: i64,
) -> anyhow::Result<NodeMetricSamplesResponse> {
    let rows = sqlx::query(
        r#"
        SELECT sampled_at, cpu_usage_percent, memory_total_bytes, memory_used_bytes,
               disk_total_bytes, disk_used_bytes
        FROM node_metric_samples
        WHERE node_id = ? AND sampled_at >= ? AND sampled_at <= ?
        ORDER BY sampled_at
        "#,
    )
    .bind(node_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    let samples: Vec<NodeMetricSample> = rows
        .into_iter()
        .map(|row| NodeMetricSample {
            sampled_at: row.get("sampled_at"),
            cpu_usage_percent: row.get("cpu_usage_percent"),
            memory_total_bytes: row.get("memory_total_bytes"),
            memory_used_bytes: row.get("memory_used_bytes"),
            disk_total_bytes: row.get("disk_total_bytes"),
            disk_used_bytes: row.get("disk_used_bytes"),
        })
        .collect();
    let (actual_from, actual_to) = actual_range(samples.iter().map(|sample| sample.sampled_at));

    Ok(NodeMetricSamplesResponse {
        node_id: node_id.to_string(),
        requested_from: from,
        requested_to: to,
        actual_from,
        actual_to,
        sample_count: samples.len(),
        samples,
    })
}

pub async fn gpu_metric_samples(
    pool: &SqlitePool,
    node_id: &str,
    gpu_key: &str,
    from: i64,
    to: i64,
) -> anyhow::Result<GpuMetricSamplesResponse> {
    let rows = sqlx::query(
        r#"
        SELECT sampled_at, vendor, memory_total_bytes, memory_used_bytes,
               utilization_percent, temperature_celsius, power_watts
        FROM gpu_metric_samples
        WHERE node_id = ? AND gpu_key = ? AND sampled_at >= ? AND sampled_at <= ?
        ORDER BY sampled_at
        "#,
    )
    .bind(node_id)
    .bind(gpu_key)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    let samples: Vec<GpuMetricSample> = rows
        .into_iter()
        .map(|row| GpuMetricSample {
            sampled_at: row.get("sampled_at"),
            vendor: row.get("vendor"),
            memory_total_bytes: row.get("memory_total_bytes"),
            memory_used_bytes: row.get("memory_used_bytes"),
            utilization_percent: row.get("utilization_percent"),
            temperature_celsius: row.get("temperature_celsius"),
            power_watts: row.get("power_watts"),
        })
        .collect();
    let (actual_from, actual_to) = actual_range(samples.iter().map(|sample| sample.sampled_at));

    Ok(GpuMetricSamplesResponse {
        node_id: node_id.to_string(),
        gpu_key: gpu_key.to_string(),
        requested_from: from,
        requested_to: to,
        actual_from,
        actual_to,
        sample_count: samples.len(),
        samples,
    })
}

fn actual_range(mut sampled_at_values: impl Iterator<Item = i64>) -> (Option<i64>, Option<i64>) {
    let Some(first) = sampled_at_values.next() else {
        return (None, None);
    };

    let mut min = first;
    let mut max = first;
    for sampled_at in sampled_at_values {
        min = min.min(sampled_at);
        max = max.max(sampled_at);
    }

    (Some(min), Some(max))
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
