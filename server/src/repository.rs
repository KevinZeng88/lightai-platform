use std::collections::HashSet;

use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::auth;
use crate::models::{
    AgentConfig, AgentConfigPoliciesResponse, AgentConfigPolicy, AgentConfigPolicyView,
    AuditEventView, AuditListResponse, AuditQuery, GpuMetricSample, GpuMetricSamplesResponse,
    GpuMetrics, GpuView, HeartbeatRequest, NodeListResponse, NodeMetricSample,
    NodeMetricSamplesResponse, NodeMetrics, NodeView, RegisterRequest, RegisterResponse,
};
use crate::platform_log::LogPolicy;

const HEARTBEAT_INTERVAL_SECS: u64 = 15;
pub const ONLINE_THRESHOLD_SECS: i64 = 60;
const AGENT_CONFIG_VERSION: i64 = 1;

pub struct AuditRecord<'a> {
    pub operation_type: &'a str,
    pub target_type: &'a str,
    pub target_id: Option<&'a str>,
    pub node_id: Option<&'a str>,
    pub instance_id: Option<&'a str>,
    pub result: &'a str,
    pub error_message: Option<&'a str>,
    pub detail_json: Option<String>,
    pub actor_type: Option<&'a str>,
    pub source: Option<&'a str>,
}

pub async fn record_audit(pool: &SqlitePool, record: AuditRecord<'_>) -> anyhow::Result<()> {
    let now = now_unix_secs();
    let actor_type = record.actor_type.unwrap_or("system");
    let source = record.source.unwrap_or("local");
    sqlx::query(
        r#"
        INSERT INTO audit_events (
            id, occurred_at, actor_type, actor_id, actor_group_id, operation_type,
            target_type, target_id, node_id, instance_id, result, error_message, source, detail_json
        )
        VALUES (?, ?, ?, 'local', NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(now)
    .bind(actor_type)
    .bind(record.operation_type)
    .bind(record.target_type)
    .bind(record.target_id)
    .bind(record.node_id)
    .bind(record.instance_id)
    .bind(record.result)
    .bind(record.error_message.map(crate::platform_log::sanitize))
    .bind(source)
    .bind(
        record
            .detail_json
            .map(|value| crate::platform_log::sanitize(&value)),
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_frontend_error(
    pool: &SqlitePool,
    message: &str,
    stack: Option<&str>,
    url: Option<&str>,
    occurred_at: i64,
) -> anyhow::Result<()> {
    let detail = serde_json::json!({
        "message": crate::platform_log::sanitize(message),
        "stack": stack.map(|s| s.chars().take(1024).collect::<String>()),
        "url": url.map(|u| u.chars().take(512).collect::<String>()),
        "client_ts": occurred_at,
    });
    record_audit(
        pool,
        AuditRecord {
            operation_type: "frontend_error",
            target_type: "frontend",
            target_id: None,
            node_id: None,
            instance_id: None,
            result: "failed",
            error_message: Some(message),
            detail_json: Some(detail.to_string()),
            actor_type: Some("frontend"),
            source: Some("frontend"),
        },
    )
    .await
}

pub async fn list_audit_events(
    pool: &SqlitePool,
    query: AuditQuery,
) -> anyhow::Result<AuditListResponse> {
    let from = query.from.unwrap_or(0);
    let to = query.to.unwrap_or_else(now_unix_secs);
    let rows = sqlx::query(
        r#"
        SELECT *
        FROM audit_events
        WHERE occurred_at >= ? AND occurred_at <= ?
          AND (? IS NULL OR operation_type = ?)
          AND (? IS NULL OR target_type = ?)
          AND (? IS NULL OR target_id = ?)
          AND (? IS NULL OR node_id = ?)
          AND (? IS NULL OR instance_id = ?)
          AND (? IS NULL OR actor_type = ?)
          AND (? IS NULL OR result = ?)
        ORDER BY occurred_at DESC
        LIMIT 500
        "#,
    )
    .bind(from)
    .bind(to)
    .bind(query.operation_type.as_deref())
    .bind(query.operation_type.as_deref())
    .bind(query.target_type.as_deref())
    .bind(query.target_type.as_deref())
    .bind(query.target_id.as_deref())
    .bind(query.target_id.as_deref())
    .bind(query.node_id.as_deref())
    .bind(query.node_id.as_deref())
    .bind(query.instance_id.as_deref())
    .bind(query.instance_id.as_deref())
    .bind(query.actor_type.as_deref())
    .bind(query.actor_type.as_deref())
    .bind(query.result.as_deref())
    .bind(query.result.as_deref())
    .fetch_all(pool)
    .await?;
    Ok(AuditListResponse {
        events: rows
            .into_iter()
            .map(|row| AuditEventView {
                id: row.get("id"),
                occurred_at: row.get("occurred_at"),
                actor_type: row.get("actor_type"),
                actor_id: row.get("actor_id"),
                actor_group_id: row.get("actor_group_id"),
                operation_type: row.get("operation_type"),
                target_type: row.get("target_type"),
                target_id: row.get("target_id"),
                node_id: row.get("node_id"),
                instance_id: row.get("instance_id"),
                result: row.get("result"),
                error_message: row.get("error_message"),
                source: row.get("source"),
                detail_json: row.get("detail_json"),
            })
            .collect(),
    })
}

pub async fn server_log_policy(pool: &SqlitePool) -> anyhow::Result<LogPolicy> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT value_json FROM platform_settings WHERE key = 'server_log_policy'",
    )
    .fetch_optional(pool)
    .await?;
    Ok(value
        .as_deref()
        .and_then(|json| serde_json::from_str::<LogPolicy>(json).ok())
        .unwrap_or_else(crate::platform_log::global))
}

pub async fn update_server_log_policy(
    pool: &SqlitePool,
    policy: LogPolicy,
) -> anyhow::Result<LogPolicy> {
    crate::platform_log::validate_policy(&policy)?;
    let now = now_unix_secs();
    sqlx::query(
        r#"
        INSERT INTO platform_settings (key, value_json, updated_at)
        VALUES ('server_log_policy', ?, ?)
        ON CONFLICT(key) DO UPDATE SET
            value_json = excluded.value_json,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(serde_json::to_string(&policy)?)
    .bind(now)
    .execute(pool)
    .await?;
    crate::platform_log::set_global(policy.clone());
    Ok(policy)
}

pub async fn register_node(
    pool: &SqlitePool,
    request: RegisterRequest,
) -> anyhow::Result<RegisterResponse> {
    let now = now_unix_secs();
    let token = auth::generate_agent_token();
    let token_hash = auth::hash_token(&token);
    let token_prefix = auth::token_prefix(&token);

    // 最多重试一次：处理 same name + same hostname 并发注册导致的 UNIQUE 冲突。
    // 第一次尝试命中 UNIQUE(name) 或 UNIQUE(hostname) 时，重试会通过 SELECT
    // 发现已有记录，走复用 node_id 路径。
    for attempt in 0..2 {
        match try_register(pool, &request, now, &token, &token_hash, &token_prefix).await {
            Ok(response) => return Ok(response),
            Err(error) if attempt == 0 && is_unique_violation(&error) => {
                continue; // 并发冲突 → 重试一次
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

async fn try_register(
    pool: &SqlitePool,
    request: &RegisterRequest,
    now: i64,
    token: &str,
    token_hash: &str,
    token_prefix: &str,
) -> anyhow::Result<RegisterResponse> {
    // Agent 身份识别规则（当前阶段）：
    // 1. 不同 Agent 不允许使用相同 name — UNIQUE(name) 保障
    // 2. 不同 Agent 不允许使用相同 hostname — UNIQUE(hostname) 保障
    // 3. same name + same hostname → 复用 node_id，更新 token
    // 4. same name + different hostname → 拒绝
    // 5. different name + same hostname → 拒绝
    // 6. different name + different hostname → 创建新节点
    //
    // 事务保证检查与写入原子性；UNIQUE(name)、UNIQUE(hostname) 作为并发兜底。
    // ON CONFLICT(id) 仅用于"同节点重注册"场景（id 不变，更新 token）。
    let mut tx = pool.begin().await?;

    let name_conflict: Option<String> =
        sqlx::query_scalar("SELECT hostname FROM nodes WHERE name = ? AND hostname != ? LIMIT 1")
            .bind(&request.name)
            .bind(&request.hostname)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(conflict_host) = name_conflict {
        anyhow::bail!(
            "节点名称 '{}' 已被主机 '{}' 使用；相同名称不允许用于不同主机。请修改 Agent 配置中的 node_name",
            request.name,
            conflict_host
        );
    }

    let hostname_conflict: Option<String> =
        sqlx::query_scalar("SELECT name FROM nodes WHERE hostname = ? AND name != ? LIMIT 1")
            .bind(&request.hostname)
            .bind(&request.name)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(conflict_name) = hostname_conflict {
        anyhow::bail!(
            "主机 '{}' 已被节点 '{}' 使用；相同主机不允许用于不同名称。请修改 Agent 配置中的 hostname 或 node_name",
            request.hostname,
            conflict_name
        );
    }

    let node_id: String =
        sqlx::query_scalar("SELECT id FROM nodes WHERE name = ? AND hostname = ? LIMIT 1")
            .bind(&request.name)
            .bind(&request.hostname)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_else(|| Uuid::new_v4().to_string());

    sqlx::query(
        r#"
        INSERT INTO nodes (
            id, name, hostname, agent_version, os, arch,
            token_hash, token_prefix, registered_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            hostname = excluded.hostname,
            agent_version = excluded.agent_version,
            os = excluded.os,
            arch = excluded.arch,
            token_hash = excluded.token_hash,
            token_prefix = excluded.token_prefix,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&node_id)
    .bind(request.name.as_str())
    .bind(request.hostname.as_str())
    .bind(request.agent_version.as_deref())
    .bind(request.os.as_deref())
    .bind(request.arch.as_deref())
    .bind(token_hash)
    .bind(token_prefix)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(RegisterResponse {
        node_id: node_id.clone(),
        agent_token: token.to_string(),
        heartbeat_interval_secs: HEARTBEAT_INTERVAL_SECS,
        agent_config: effective_agent_config(pool, &node_id).await?,
    })
}

/// 判断是否为 SQLite UNIQUE 约束冲突错误。
/// 用于 register_node 并发重试：same name + same hostname 并发注册时，
/// 第一个 INSERT 成功，第二个触发 UNIQUE 冲突，重试后可复用已有记录。
fn is_unique_violation(error: &anyhow::Error) -> bool {
    error.to_string().contains("UNIQUE constraint failed")
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
    let agent_config = request.agent_config.clone();
    let managed_instances = request.managed_instances.clone();
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
            disk_total_bytes, disk_used_bytes, collector_errors_json, updated_at,
            agent_config_version, heartbeat_interval_secs, metrics_sample_interval_secs,
            task_poll_interval_secs, config_refresh_interval_secs, command_timeout_secs,
            environment_check_timeout_secs, allowed_model_dirs_json, nvidia_collector_enabled,
            custom_collector_script, collector_timeout_secs, collector_max_output_bytes,
            last_config_updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(node_id) DO UPDATE SET
            cpu_usage_percent = excluded.cpu_usage_percent,
            memory_total_bytes = excluded.memory_total_bytes,
            memory_used_bytes = excluded.memory_used_bytes,
            disk_total_bytes = excluded.disk_total_bytes,
            disk_used_bytes = excluded.disk_used_bytes,
            collector_errors_json = excluded.collector_errors_json,
            updated_at = excluded.updated_at,
            agent_config_version = excluded.agent_config_version,
            heartbeat_interval_secs = excluded.heartbeat_interval_secs,
            metrics_sample_interval_secs = excluded.metrics_sample_interval_secs,
            task_poll_interval_secs = excluded.task_poll_interval_secs,
            config_refresh_interval_secs = excluded.config_refresh_interval_secs,
            command_timeout_secs = excluded.command_timeout_secs,
            environment_check_timeout_secs = excluded.environment_check_timeout_secs,
            allowed_model_dirs_json = excluded.allowed_model_dirs_json,
            nvidia_collector_enabled = excluded.nvidia_collector_enabled,
            custom_collector_script = excluded.custom_collector_script,
            collector_timeout_secs = excluded.collector_timeout_secs,
            collector_max_output_bytes = excluded.collector_max_output_bytes,
            last_config_updated_at = excluded.last_config_updated_at
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
    .bind(agent_config.as_ref().map(|config| config.config_version))
    .bind(
        agent_config
            .as_ref()
            .map(|config| config.heartbeat_interval_secs as i64),
    )
    .bind(
        agent_config
            .as_ref()
            .map(|config| config.metrics_sample_interval_secs as i64),
    )
    .bind(
        agent_config
            .as_ref()
            .map(|config| config.task_poll_interval_secs as i64),
    )
    .bind(
        agent_config
            .as_ref()
            .map(|config| config.config_refresh_interval_secs as i64),
    )
    .bind(
        agent_config
            .as_ref()
            .map(|config| config.command_timeout_secs as i64),
    )
    .bind(
        agent_config
            .as_ref()
            .map(|config| config.environment_check_timeout_secs as i64),
    )
    .bind(
        agent_config
            .as_ref()
            .map(|config| serde_json::to_string(&config.allowed_model_dirs))
            .transpose()?,
    )
    .bind(
        agent_config
            .as_ref()
            .map(|config| config.nvidia_collector_enabled as i64),
    )
    .bind(
        agent_config
            .as_ref()
            .and_then(|config| config.custom_collector_script.clone()),
    )
    .bind(
        agent_config
            .as_ref()
            .map(|config| config.collector_timeout_secs as i64),
    )
    .bind(
        agent_config
            .as_ref()
            .map(|config| config.collector_max_output_bytes as i64),
    )
    .bind(agent_config.and_then(|config| config.last_config_updated_at))
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
    reconcile_managed_instances(&mut tx, &request.node_id, &managed_instances, now).await?;

    tx.commit().await?;
    Ok(())
}

async fn reconcile_managed_instances(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    node_id: &str,
    reports: &[crate::models::ManagedInstanceReport],
    now: i64,
) -> anyhow::Result<()> {
    let mut reported_ids = HashSet::new();
    for report in reports {
        reported_ids.insert(report.instance_id.clone());
        let status = match report.status.as_str() {
            "running" => "running",
            "stopped" => "stopped",
            "failed" => "failed",
            _ => "failed",
        };
        let log_tail = report
            .log_path
            .as_deref()
            .map(|path| format!("日志文件：{path}"));
        // last_error 仅写入失败/异常原因；running 实例不写任何信息（清空旧错误）
        let last_error: Option<&str> = if status == "running" {
            None
        } else {
            Some(&report.message)
        };
        sqlx::query(
            r#"
            UPDATE model_instances
            SET status = ?, process_id = ?, process_ref = ?, base_url = COALESCE(?, base_url),
                endpoint_url = COALESCE(?, endpoint_url), command = COALESCE(?, command),
                log_tail = COALESCE(?, log_tail), last_checked_at = ?, last_error = ?, updated_at = ?
            WHERE id = ? AND node_id = ? AND deploy_type = 'local'
            "#,
        )
        .bind(status)
        .bind(if status == "running" { report.process_id } else { None })
        .bind(if status == "running" {
            report.process_ref.as_deref()
        } else {
            None
        })
        .bind(report.base_url.as_deref())
        .bind(report.endpoint_url.as_deref())
        .bind(report.command.as_deref())
        .bind(log_tail.as_deref())
        .bind(now)
        .bind(last_error)
        .bind(now)
        .bind(&report.instance_id)
        .bind(node_id)
        .execute(&mut **tx)
        .await?;
    }

    let rows = sqlx::query(
        r#"
        SELECT id
        FROM model_instances
        WHERE node_id = ? AND deploy_type = 'local'
          AND status IN ('starting', 'running', 'stopping')
        "#,
    )
    .bind(node_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in rows {
        let id: String = row.get("id");
        if reported_ids.contains(&id) {
            continue;
        }
        sqlx::query(
            r#"
            UPDATE model_instances
            SET status = 'failed', process_id = NULL, process_ref = NULL, last_checked_at = ?,
                last_error = 'Agent 未上报该实例受管进程状态，进程可能已退出或被外部终止',
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    }

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
               s.disk_total_bytes, s.disk_used_bytes,
               s.agent_config_version, s.heartbeat_interval_secs,
               s.metrics_sample_interval_secs, s.task_poll_interval_secs,
               s.config_refresh_interval_secs, s.command_timeout_secs,
               s.environment_check_timeout_secs, s.allowed_model_dirs_json,
               s.nvidia_collector_enabled, s.custom_collector_script,
               s.collector_timeout_secs, s.collector_max_output_bytes,
               s.last_config_updated_at
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
        let agent_config = row
            .try_get::<Option<i64>, _>("agent_config_version")
            .ok()
            .flatten()
            .map(|config_version| AgentConfig {
                config_version,
                heartbeat_interval_secs: row
                    .get::<Option<i64>, _>("heartbeat_interval_secs")
                    .unwrap_or(HEARTBEAT_INTERVAL_SECS as i64)
                    as u64,
                metrics_sample_interval_secs: row
                    .get::<Option<i64>, _>("metrics_sample_interval_secs")
                    .unwrap_or(HEARTBEAT_INTERVAL_SECS as i64)
                    as u64,
                task_poll_interval_secs: row
                    .get::<Option<i64>, _>("task_poll_interval_secs")
                    .unwrap_or(HEARTBEAT_INTERVAL_SECS as i64)
                    as u64,
                config_refresh_interval_secs: row
                    .get::<Option<i64>, _>("config_refresh_interval_secs")
                    .unwrap_or(60) as u64,
                command_timeout_secs: row
                    .get::<Option<i64>, _>("command_timeout_secs")
                    .unwrap_or(5) as u64,
                environment_check_timeout_secs: row
                    .get::<Option<i64>, _>("environment_check_timeout_secs")
                    .unwrap_or(5) as u64,
                allowed_model_dirs: row
                    .get::<Option<String>, _>("allowed_model_dirs_json")
                    .and_then(|value| serde_json::from_str(&value).ok())
                    .unwrap_or_default(),
                nvidia_collector_enabled: row
                    .get::<Option<i64>, _>("nvidia_collector_enabled")
                    .map(|value| value != 0)
                    .unwrap_or(true),
                custom_collector_script: row.get("custom_collector_script"),
                collector_timeout_secs: row
                    .get::<Option<i64>, _>("collector_timeout_secs")
                    .unwrap_or(5) as u64,
                collector_max_output_bytes: row
                    .get::<Option<i64>, _>("collector_max_output_bytes")
                    .unwrap_or(1024 * 1024) as usize,
                log_policy: LogPolicy::default(),
                last_config_updated_at: row.get("last_config_updated_at"),
            });
        let effective_agent_config = effective_agent_config(pool, &node_id).await?;
        let config_sync_status = match &agent_config {
            Some(current) if current.config_version == effective_agent_config.config_version => {
                "synced"
            }
            Some(_) => "out_of_sync",
            None => "pending",
        }
        .to_string();

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
            agent_config,
            effective_agent_config,
            config_sync_status,
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

pub async fn list_agent_config_policies(
    pool: &SqlitePool,
) -> anyhow::Result<AgentConfigPoliciesResponse> {
    let global = agent_config_policy_view(pool, "global", None).await?;
    let rows = sqlx::query("SELECT id FROM nodes ORDER BY name")
        .fetch_all(pool)
        .await?;
    let mut nodes = Vec::new();
    for row in rows {
        let node_id: String = row.get("id");
        nodes.push(agent_config_policy_view(pool, "node", Some(&node_id)).await?);
    }
    Ok(AgentConfigPoliciesResponse { global, nodes })
}

pub async fn global_agent_config_policy(
    pool: &SqlitePool,
) -> anyhow::Result<AgentConfigPolicyView> {
    agent_config_policy_view(pool, "global", None).await
}

pub async fn node_agent_config_policy(
    pool: &SqlitePool,
    node_id: &str,
) -> anyhow::Result<AgentConfigPolicyView> {
    agent_config_policy_view(pool, "node", Some(node_id)).await
}

pub async fn update_global_agent_config_policy(
    pool: &SqlitePool,
    policy: AgentConfigPolicy,
) -> anyhow::Result<AgentConfigPolicyView> {
    save_agent_config_policy(pool, "global", "", policy).await?;
    global_agent_config_policy(pool).await
}

pub async fn update_node_agent_config_policy(
    pool: &SqlitePool,
    node_id: &str,
    policy: AgentConfigPolicy,
) -> anyhow::Result<AgentConfigPolicyView> {
    save_agent_config_policy(pool, "node", node_id, policy).await?;
    node_agent_config_policy(pool, node_id).await
}

pub async fn effective_agent_config(
    pool: &SqlitePool,
    node_id: &str,
) -> anyhow::Result<AgentConfig> {
    let now = now_unix_secs();
    let global = read_policy(pool, "global", "").await?.unwrap_or_default();
    let node = read_policy(pool, "node", node_id)
        .await?
        .unwrap_or_default();
    let global_version = policy_version(pool, "global", "")
        .await?
        .unwrap_or(AGENT_CONFIG_VERSION);
    let node_version = policy_version(pool, "node", node_id).await?.unwrap_or(0);
    Ok(apply_policy(
        default_agent_config(now),
        global,
        node,
        global_version.max(node_version).max(AGENT_CONFIG_VERSION),
        now,
    ))
}

async fn save_agent_config_policy(
    pool: &SqlitePool,
    scope: &str,
    node_id: &str,
    policy: AgentConfigPolicy,
) -> anyhow::Result<()> {
    validate_policy(&policy)?;
    let now = now_unix_secs();
    let version = next_policy_version(pool).await?;
    sqlx::query(
        r#"
        INSERT INTO agent_config_policies (scope, node_id, policy_json, version, updated_at)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(scope, node_id) DO UPDATE SET
            policy_json = excluded.policy_json,
            version = excluded.version,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(scope)
    .bind(node_id)
    .bind(serde_json::to_string(&policy)?)
    .bind(version)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

async fn agent_config_policy_view(
    pool: &SqlitePool,
    scope: &str,
    node_id: Option<&str>,
) -> anyhow::Result<AgentConfigPolicyView> {
    let node_key = node_id.unwrap_or("");
    let policy = read_policy(pool, scope, node_key)
        .await?
        .unwrap_or_default();
    let version = policy_version(pool, scope, node_key)
        .await?
        .unwrap_or(AGENT_CONFIG_VERSION);
    let updated_at = policy_updated_at(pool, scope, node_key)
        .await?
        .unwrap_or_else(now_unix_secs);
    let effective_config = match node_id {
        Some(node_id) => effective_agent_config(pool, node_id).await?,
        None => apply_policy(
            default_agent_config(updated_at),
            policy.clone(),
            AgentConfigPolicy::default(),
            version,
            updated_at,
        ),
    };
    Ok(AgentConfigPolicyView {
        scope: scope.to_string(),
        node_id: node_id.map(str::to_string),
        version,
        updated_at,
        policy,
        effective_config,
        restart_required_fields: vec!["server_url", "node_name", "state_path", "listen_addr"],
        online_reload_fields: vec![
            "heartbeat_interval_secs",
            "metrics_sample_interval_secs",
            "command_timeout_secs",
            "environment_check_timeout_secs",
            "allowed_model_dirs",
            "nvidia_collector_enabled",
            "custom_collector_script",
            "collector_timeout_secs",
            "collector_max_output_bytes",
            "log_dir",
            "log_level",
            "log_max_file_bytes",
            "log_retention_files",
            "log_retention_days",
        ],
    })
}

async fn read_policy(
    pool: &SqlitePool,
    scope: &str,
    node_id: &str,
) -> anyhow::Result<Option<AgentConfigPolicy>> {
    let json: Option<String> = sqlx::query_scalar(
        "SELECT policy_json FROM agent_config_policies WHERE scope = ? AND node_id = ?",
    )
    .bind(scope)
    .bind(node_id)
    .fetch_optional(pool)
    .await?;
    json.map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(Into::into)
}

async fn policy_version(
    pool: &SqlitePool,
    scope: &str,
    node_id: &str,
) -> anyhow::Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT version FROM agent_config_policies WHERE scope = ? AND node_id = ?",
    )
    .bind(scope)
    .bind(node_id)
    .fetch_optional(pool)
    .await?)
}

async fn policy_updated_at(
    pool: &SqlitePool,
    scope: &str,
    node_id: &str,
) -> anyhow::Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT updated_at FROM agent_config_policies WHERE scope = ? AND node_id = ?",
    )
    .bind(scope)
    .bind(node_id)
    .fetch_optional(pool)
    .await?)
}

async fn next_policy_version(pool: &SqlitePool) -> anyhow::Result<i64> {
    let current: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM agent_config_policies")
        .fetch_one(pool)
        .await?;
    Ok(current.unwrap_or(AGENT_CONFIG_VERSION) + 1)
}

fn apply_policy(
    mut config: AgentConfig,
    global: AgentConfigPolicy,
    node: AgentConfigPolicy,
    version: i64,
    updated_at: i64,
) -> AgentConfig {
    apply_policy_layer(&mut config, global);
    apply_policy_layer(&mut config, node);
    config.config_version = version;
    config.last_config_updated_at = Some(updated_at);
    config
}

fn apply_policy_layer(config: &mut AgentConfig, policy: AgentConfigPolicy) {
    if let Some(value) = policy.heartbeat_interval_secs {
        config.heartbeat_interval_secs = value;
    }
    if let Some(value) = policy.metrics_sample_interval_secs {
        config.metrics_sample_interval_secs = value;
    }
    if let Some(value) = policy.command_timeout_secs {
        config.command_timeout_secs = value;
    }
    if let Some(value) = policy.environment_check_timeout_secs {
        config.environment_check_timeout_secs = value;
    }
    if let Some(value) = policy.allowed_model_dirs {
        config.allowed_model_dirs = value;
    }
    if let Some(value) = policy.nvidia_collector_enabled {
        config.nvidia_collector_enabled = value;
    }
    if let Some(value) = policy.custom_collector_script {
        config.custom_collector_script = value.filter(|item| !item.trim().is_empty());
    }
    if let Some(value) = policy.collector_timeout_secs {
        config.collector_timeout_secs = value;
    }
    if let Some(value) = policy.collector_max_output_bytes {
        config.collector_max_output_bytes = value;
    }
    if let Some(value) = policy.log_dir {
        config.log_policy.log_dir = value;
    }
    if let Some(value) = policy.log_level {
        config.log_policy.log_level = value;
    }
    if let Some(value) = policy.log_max_file_bytes {
        config.log_policy.log_max_file_bytes = value;
    }
    if let Some(value) = policy.log_retention_files {
        config.log_policy.log_retention_files = value;
    }
    if let Some(value) = policy.log_retention_days {
        config.log_policy.log_retention_days = value;
    }
}

fn validate_policy(policy: &AgentConfigPolicy) -> anyhow::Result<()> {
    for (name, value) in [
        ("heartbeat_interval_secs", policy.heartbeat_interval_secs),
        (
            "metrics_sample_interval_secs",
            policy.metrics_sample_interval_secs,
        ),
        ("command_timeout_secs", policy.command_timeout_secs),
        (
            "environment_check_timeout_secs",
            policy.environment_check_timeout_secs,
        ),
        ("collector_timeout_secs", policy.collector_timeout_secs),
    ] {
        if value.is_some_and(|value| value == 0 || value > 86_400) {
            anyhow::bail!("{name} must be between 1 and 86400");
        }
    }
    if policy
        .collector_max_output_bytes
        .is_some_and(|value| value == 0 || value > 16 * 1024 * 1024)
    {
        anyhow::bail!("collector_max_output_bytes must be between 1 and 16777216");
    }
    if let Some(dirs) = &policy.allowed_model_dirs {
        for dir in dirs {
            if dir.trim().is_empty() || dir.contains("..") {
                anyhow::bail!("allowed_model_dirs contains invalid path");
            }
        }
    }
    let mut log_policy = LogPolicy::default();
    if let Some(value) = &policy.log_dir {
        log_policy.log_dir = value.clone();
    }
    if let Some(value) = &policy.log_level {
        log_policy.log_level = value.clone();
    }
    if let Some(value) = policy.log_max_file_bytes {
        log_policy.log_max_file_bytes = value;
    }
    if let Some(value) = policy.log_retention_files {
        log_policy.log_retention_files = value;
    }
    if let Some(value) = policy.log_retention_days {
        log_policy.log_retention_days = value;
    }
    crate::platform_log::validate_policy(&log_policy)?;
    Ok(())
}

pub fn default_agent_config(updated_at: i64) -> AgentConfig {
    AgentConfig {
        config_version: AGENT_CONFIG_VERSION,
        heartbeat_interval_secs: HEARTBEAT_INTERVAL_SECS,
        metrics_sample_interval_secs: HEARTBEAT_INTERVAL_SECS,
        command_timeout_secs: 5,
        environment_check_timeout_secs: 5,
        last_config_updated_at: Some(updated_at),
        ..AgentConfig::default()
    }
}
