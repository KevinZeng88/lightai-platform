use std::collections::HashSet;

use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::auth;
use crate::models::{
    AgentConfig, AgentConfigPoliciesResponse, AgentConfigPolicy, AgentConfigPolicyView,
    AuditEventView, AuditListResponse, AuditQuery, AuthUser, CollectorRegistryEntry,
    GpuMetricSample, GpuMetricSamplesResponse, GpuMetrics, GpuView, HeartbeatRequest,
    NodeListResponse, NodeMetricSample, NodeMetricSamplesResponse, NodeMetrics, NodeView,
    RegisterCollectorRequest, RegisterRequest, RegisterResponse, UserCreateRequest,
    UserGroupCreateRequest, UserGroupMembersRequest, UserGroupUpdateRequest, UserGroupView,
    UserUpdateRequest,
};
use crate::platform_log;
use crate::platform_log::LogPolicy;

const HEARTBEAT_INTERVAL_SECS: u64 = 15;
pub const ONLINE_THRESHOLD_SECS: i64 = 60;
const AGENT_CONFIG_VERSION: i64 = 1;
const SESSION_TTL_SECS: i64 = 12 * 60 * 60;

#[derive(Debug, Clone)]
pub struct PasswordPolicy {
    pub min_length: usize,
    pub complexity_required: bool,
    pub expires_days: Option<i64>,
    pub force_change_after_reset: bool,
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            min_length: 12,
            complexity_required: false,
            expires_days: None,
            force_change_after_reset: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionPolicy {
    pub ttl_secs: i64,
    pub idle_timeout_secs: Option<i64>,
    pub secure_cookie: bool,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            ttl_secs: SESSION_TTL_SECS,
            idle_timeout_secs: Some(2 * 60 * 60),
            secure_cookie: false,
        }
    }
}

pub async fn user_count(pool: &SqlitePool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?)
}

pub async fn ensure_initial_admin(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> anyhow::Result<Option<AuthUser>> {
    if user_count(pool).await? > 0 {
        return Ok(None);
    }
    create_user(
        pool,
        UserCreateRequest {
            username: username.to_string(),
            password: password.to_string(),
            role: "admin".to_string(),
        },
    )
    .await
    .map(Some)
}

pub async fn setup_initial_admin(
    pool: &SqlitePool,
    username: &str,
    password: &str,
    policy: &PasswordPolicy,
) -> anyhow::Result<AuthUser> {
    validate_username(username)?;
    validate_password(password, policy)?;
    let mut tx = pool.begin().await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if count > 0 {
        anyhow::bail!("setup already completed");
    }
    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    let password_hash = auth::hash_password(password)?;
    sqlx::query(
        r#"
        INSERT INTO users (
            id, username, password_hash, role, enabled, password_changed_at,
            must_change_password, created_at, updated_at
        )
        VALUES (?, ?, ?, 'admin', 1, ?, 0, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(username.trim())
    .bind(password_hash)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    user_by_id(pool, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("created admin not found"))
}

pub async fn create_user(
    pool: &SqlitePool,
    request: UserCreateRequest,
) -> anyhow::Result<AuthUser> {
    create_user_with_policy(pool, request, &PasswordPolicy::default()).await
}

pub async fn create_user_with_policy(
    pool: &SqlitePool,
    request: UserCreateRequest,
    policy: &PasswordPolicy,
) -> anyhow::Result<AuthUser> {
    validate_username(&request.username)?;
    validate_password(&request.password, policy)?;
    validate_role(&request.role)?;
    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    let password_hash = auth::hash_password(&request.password)?;
    sqlx::query(
        r#"
        INSERT INTO users (
            id, username, password_hash, role, enabled, password_changed_at,
            must_change_password, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, 1, ?, 0, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(request.username.trim())
    .bind(password_hash)
    .bind(request.role.as_str())
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    user_by_id(pool, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("created user not found"))
}

pub async fn list_users(pool: &SqlitePool) -> anyhow::Result<Vec<AuthUser>> {
    let rows = sqlx::query(
        r#"
        SELECT id, username, role, enabled, must_change_password
        FROM users
        ORDER BY username ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let mut users = Vec::with_capacity(rows.len());
    for row in rows {
        users.push(enrich_effective_role(pool, user_from_row(row)).await?);
    }
    Ok(users)
}

pub async fn update_user(
    pool: &SqlitePool,
    id: &str,
    request: UserUpdateRequest,
) -> anyhow::Result<AuthUser> {
    update_user_with_policy(pool, id, request, &PasswordPolicy::default()).await
}

pub async fn update_user_with_policy(
    pool: &SqlitePool,
    id: &str,
    request: UserUpdateRequest,
    policy: &PasswordPolicy,
) -> anyhow::Result<AuthUser> {
    if let Some(role) = request.role.as_deref() {
        validate_role(role)?;
    }
    if let Some(password) = request.password.as_deref() {
        validate_password(password, policy)?;
    }
    let Some(existing) = user_by_id(pool, id).await? else {
        anyhow::bail!("user not found");
    };
    let role = request.role.as_deref().unwrap_or(&existing.role);
    let enabled = request.enabled.unwrap_or(existing.enabled);
    if existing.role == "admin" && (!enabled || role != "admin") {
        let other_admins: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE id != ? AND role = 'admin' AND enabled = 1",
        )
        .bind(id)
        .fetch_one(pool)
        .await?;
        if other_admins == 0 {
            anyhow::bail!("cannot disable or demote the last enabled admin");
        }
    }
    let now = now_unix_secs();
    if let Some(password) = request.password.as_deref() {
        let password_hash = auth::hash_password(password)?;
        sqlx::query(
            r#"
            UPDATE users
            SET password_hash = ?, role = ?, enabled = ?, password_changed_at = ?, must_change_password = 1, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(password_hash)
        .bind(role)
        .bind(enabled)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            r#"
            UPDATE users
            SET role = ?, enabled = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(role)
        .bind(enabled)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    }
    if !enabled {
        sqlx::query(
            "UPDATE user_sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    }
    user_by_id(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("updated user not found"))
}

pub async fn login_user(
    pool: &SqlitePool,
    username: &str,
    password: &str,
    session_policy: &SessionPolicy,
    password_policy: &PasswordPolicy,
) -> anyhow::Result<Option<(AuthUser, String)>> {
    let Some(row) = sqlx::query(
        r#"
        SELECT id, username, password_hash, role, enabled, password_changed_at, must_change_password
        FROM users
        WHERE username = ?
        "#,
    )
    .bind(username.trim())
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let password_hash: String = row.get("password_hash");
    let enabled = row.get::<i64, _>("enabled") != 0;
    if !enabled || !auth::verify_password(password, &password_hash) {
        return Ok(None);
    }
    if let Some(days) = password_policy.expires_days {
        let changed_at: i64 = row.get("password_changed_at");
        if changed_at > 0 && now_unix_secs() - changed_at > days * 86_400 {
            return Ok(None);
        }
    }

    let user = enrich_effective_role(
        pool,
        AuthUser {
            id: row.get("id"),
            username: row.get("username"),
            role: row.get("role"),
            effective_role: "viewer".to_string(),
            enabled,
            must_change_password: row.get::<i64, _>("must_change_password") != 0,
        },
    )
    .await?;
    let session_token = auth::generate_agent_token();
    let session_hash = auth::hash_token(&session_token);
    let now = now_unix_secs();
    sqlx::query(
        r#"
        INSERT INTO user_sessions (id, user_id, token_hash, created_at, last_seen_at, expires_at, revoked_at)
        VALUES (?, ?, ?, ?, ?, ?, NULL)
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&user.id)
    .bind(session_hash)
    .bind(now)
    .bind(now)
    .bind(now + session_policy.ttl_secs)
    .execute(pool)
    .await?;
    Ok(Some((user, session_token)))
}

pub async fn authenticate_session(
    pool: &SqlitePool,
    session_token: &str,
    session_policy: &SessionPolicy,
) -> anyhow::Result<Option<AuthUser>> {
    let session_hash = auth::hash_token(session_token);
    let now = now_unix_secs();
    let row = sqlx::query(
        r#"
        SELECT users.id, users.username, users.role, users.enabled, users.password_changed_at, users.must_change_password,
               user_sessions.last_seen_at
        FROM user_sessions
        JOIN users ON users.id = user_sessions.user_id
        WHERE user_sessions.token_hash = ?
          AND user_sessions.revoked_at IS NULL
          AND user_sessions.expires_at > ?
          AND users.enabled = 1
        "#,
    )
    .bind(&session_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = &row {
        if let Some(idle) = session_policy.idle_timeout_secs {
            let last_seen_at: i64 = row.get("last_seen_at");
            if now - last_seen_at > idle {
                sqlx::query("UPDATE user_sessions SET revoked_at = ? WHERE token_hash = ?")
                    .bind(now)
                    .bind(&session_hash)
                    .execute(pool)
                    .await?;
                return Ok(None);
            }
        }
        sqlx::query("UPDATE user_sessions SET last_seen_at = ? WHERE token_hash = ?")
            .bind(now)
            .bind(session_hash)
            .execute(pool)
            .await?;
    }
    match row {
        Some(row) => Ok(Some(enrich_effective_role(pool, user_from_row(row)).await?)),
        None => Ok(None),
    }
}

pub async fn reset_user_password(
    pool: &SqlitePool,
    username: &str,
    password: &str,
    policy: PasswordPolicy,
) -> anyhow::Result<AuthUser> {
    validate_password(password, &policy)?;
    let Some(user) = sqlx::query("SELECT id FROM users WHERE username = ?")
        .bind(username.trim())
        .fetch_optional(pool)
        .await?
    else {
        anyhow::bail!("user not found");
    };
    let user_id: String = user.get("id");
    let now = now_unix_secs();
    sqlx::query(
        r#"
        UPDATE users
        SET password_hash = ?, password_changed_at = ?, must_change_password = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(auth::hash_password(password)?)
    .bind(now)
    .bind(policy.force_change_after_reset)
    .bind(now)
    .bind(&user_id)
    .execute(pool)
    .await?;
    sqlx::query("UPDATE user_sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
        .bind(now)
        .bind(&user_id)
        .execute(pool)
        .await?;
    user_by_id(pool, &user_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("updated user not found"))
}

pub async fn change_user_password(
    pool: &SqlitePool,
    user_id: &str,
    current_password: &str,
    new_password: &str,
    policy: &PasswordPolicy,
) -> anyhow::Result<()> {
    validate_password(new_password, policy)?;
    let Some(row) = sqlx::query("SELECT password_hash FROM users WHERE id = ? AND enabled = 1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?
    else {
        anyhow::bail!("user not found");
    };
    let password_hash: String = row.get("password_hash");
    if !auth::verify_password(current_password, &password_hash) {
        anyhow::bail!("current password is incorrect");
    }
    let now = now_unix_secs();
    sqlx::query(
        r#"
        UPDATE users
        SET password_hash = ?, password_changed_at = ?, must_change_password = 0, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(auth::hash_password(new_password)?)
    .bind(now)
    .bind(now)
    .bind(user_id)
    .execute(pool)
    .await?;
    sqlx::query("UPDATE user_sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
        .bind(now)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn revoke_session(pool: &SqlitePool, session_token: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE user_sessions SET revoked_at = ? WHERE token_hash = ?")
        .bind(now_unix_secs())
        .bind(auth::hash_token(session_token))
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_user_groups(pool: &SqlitePool) -> anyhow::Result<Vec<UserGroupView>> {
    let rows = sqlx::query(
        r#"
        SELECT id, name, role, enabled
        FROM user_groups
        ORDER BY name ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let mut groups = Vec::with_capacity(rows.len());
    for row in rows {
        groups.push(group_view_from_row(pool, row).await?);
    }
    Ok(groups)
}

pub async fn create_user_group(
    pool: &SqlitePool,
    request: UserGroupCreateRequest,
) -> anyhow::Result<UserGroupView> {
    validate_group_name(&request.name)?;
    validate_role(&request.role)?;
    let now = now_unix_secs();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO user_groups (id, name, role, enabled, created_at, updated_at)
        VALUES (?, ?, ?, 1, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(request.name.trim())
    .bind(request.role.as_str())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    user_group_by_id(pool, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("created group not found"))
}

pub async fn update_user_group(
    pool: &SqlitePool,
    id: &str,
    request: UserGroupUpdateRequest,
) -> anyhow::Result<UserGroupView> {
    let Some(existing) = user_group_by_id(pool, id).await? else {
        anyhow::bail!("group not found");
    };
    let name = request.name.as_deref().unwrap_or(&existing.name);
    validate_group_name(name)?;
    let role = request.role.as_deref().unwrap_or(&existing.role);
    validate_role(role)?;
    let enabled = request.enabled.unwrap_or(existing.enabled);
    sqlx::query(
        r#"
        UPDATE user_groups
        SET name = ?, role = ?, enabled = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(name.trim())
    .bind(role)
    .bind(enabled)
    .bind(now_unix_secs())
    .bind(id)
    .execute(pool)
    .await?;
    user_group_by_id(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("updated group not found"))
}

pub async fn replace_user_group_members(
    pool: &SqlitePool,
    group_id: &str,
    request: UserGroupMembersRequest,
) -> anyhow::Result<UserGroupView> {
    if user_group_by_id(pool, group_id).await?.is_none() {
        anyhow::bail!("group not found");
    }
    let mut seen = HashSet::new();
    for user_id in &request.user_ids {
        if !seen.insert(user_id.clone()) {
            continue;
        }
        if user_by_id(pool, user_id).await?.is_none() {
            anyhow::bail!("user not found");
        }
    }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_group_members WHERE group_id = ?")
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
    let now = now_unix_secs();
    for user_id in seen {
        sqlx::query(
            "INSERT INTO user_group_members (group_id, user_id, created_at) VALUES (?, ?, ?)",
        )
        .bind(group_id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    user_group_by_id(pool, group_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("updated group not found"))
}

pub async fn delete_user_group(pool: &SqlitePool, group_id: &str) -> anyhow::Result<()> {
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM user_group_members WHERE group_id = ?")
            .bind(group_id)
            .fetch_one(pool)
            .await?;
    if member_count > 0 {
        anyhow::bail!("group has members; remove members before deleting");
    }
    let result = sqlx::query("DELETE FROM user_groups WHERE id = ?")
        .bind(group_id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        anyhow::bail!("group not found");
    }
    Ok(())
}

async fn user_group_by_id(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<UserGroupView>> {
    let Some(row) = sqlx::query("SELECT id, name, role, enabled FROM user_groups WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(group_view_from_row(pool, row).await?))
}

async fn group_view_from_row(
    pool: &SqlitePool,
    row: sqlx::sqlite::SqliteRow,
) -> anyhow::Result<UserGroupView> {
    let id: String = row.get("id");
    let members = group_members(pool, &id).await?;
    Ok(UserGroupView {
        id,
        name: row.get("name"),
        role: row.get("role"),
        enabled: row.get::<i64, _>("enabled") != 0,
        member_count: members.len() as i64,
        members,
    })
}

async fn group_members(pool: &SqlitePool, group_id: &str) -> anyhow::Result<Vec<AuthUser>> {
    let rows = sqlx::query(
        r#"
        SELECT users.id, users.username, users.role, users.enabled, users.must_change_password
        FROM user_group_members
        JOIN users ON users.id = user_group_members.user_id
        WHERE user_group_members.group_id = ?
        ORDER BY users.username ASC
        "#,
    )
    .bind(group_id)
    .fetch_all(pool)
    .await?;
    let mut users = Vec::with_capacity(rows.len());
    for row in rows {
        users.push(enrich_effective_role(pool, user_from_row(row)).await?);
    }
    Ok(users)
}

async fn user_by_id(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<AuthUser>> {
    let Some(row) = sqlx::query(
        "SELECT id, username, role, enabled, must_change_password FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(enrich_effective_role(pool, user_from_row(row)).await?))
}

fn user_from_row(row: sqlx::sqlite::SqliteRow) -> AuthUser {
    let role: String = row.get("role");
    AuthUser {
        id: row.get("id"),
        username: row.get("username"),
        effective_role: role.clone(),
        role,
        enabled: row.get::<i64, _>("enabled") != 0,
        must_change_password: row.get::<i64, _>("must_change_password") != 0,
    }
}

async fn enrich_effective_role(pool: &SqlitePool, mut user: AuthUser) -> anyhow::Result<AuthUser> {
    let group_roles: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT user_groups.role
        FROM user_group_members
        JOIN user_groups ON user_groups.id = user_group_members.group_id
        WHERE user_group_members.user_id = ?
          AND user_groups.enabled = 1
        "#,
    )
    .bind(&user.id)
    .fetch_all(pool)
    .await?;
    let mut effective = user.role.as_str();
    for role in &group_roles {
        if role_rank(role) > role_rank(effective) {
            effective = role;
        }
    }
    user.effective_role = effective.to_string();
    Ok(user)
}

pub fn role_rank(role: &str) -> i32 {
    match role {
        "admin" => 3,
        "operator" => 2,
        "viewer" => 1,
        _ => 0,
    }
}

fn validate_username(username: &str) -> anyhow::Result<()> {
    let username = username.trim();
    if username.len() < 3 || username.len() > 64 {
        anyhow::bail!("username must be 3-64 characters");
    }
    if !username
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!("username may only contain letters, numbers, '.', '-' and '_'");
    }
    Ok(())
}

fn validate_group_name(name: &str) -> anyhow::Result<()> {
    let name = name.trim();
    if name.len() < 2 || name.len() > 80 {
        anyhow::bail!("group name must be 2-80 characters");
    }
    if name.chars().any(char::is_control) {
        anyhow::bail!("group name must not contain control characters");
    }
    Ok(())
}

fn validate_password(password: &str, policy: &PasswordPolicy) -> anyhow::Result<()> {
    if password.len() < policy.min_length {
        anyhow::bail!("password does not meet the configured password policy");
    }
    if policy.complexity_required {
        let has_lower = password.chars().any(|ch| ch.is_ascii_lowercase());
        let has_upper = password.chars().any(|ch| ch.is_ascii_uppercase());
        let has_digit = password.chars().any(|ch| ch.is_ascii_digit());
        if !(has_lower && has_upper && has_digit) {
            anyhow::bail!("password does not meet the configured password policy");
        }
    }
    Ok(())
}

fn validate_role(role: &str) -> anyhow::Result<()> {
    if !matches!(role, "admin" | "operator" | "viewer") {
        anyhow::bail!("role must be admin, operator or viewer");
    }
    Ok(())
}

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
    pub actor_id: Option<&'a str>,
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
        VALUES (?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(now)
    .bind(actor_type)
    .bind(record.actor_id.unwrap_or("local"))
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
    user: Option<&AuthUser>,
) -> anyhow::Result<()> {
    let detail = serde_json::json!({
        "message": crate::platform_log::sanitize(message),
        "stack": stack.map(|s| s.chars().take(1024).collect::<String>()),
        "url": url.map(|u| u.chars().take(512).collect::<String>()),
        "client_ts": occurred_at,
        "user": user.map(|u| u.username.clone()),
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
            actor_type: Some(user.map(|_| "user").unwrap_or("frontend")),
            actor_id: user.map(|user| user.id.as_str()),
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
    // 正常情况下不会到达此处：loop 内所有分支均 return。
    // 保留为 Err 而非 unreachable!() 以避免生产环境 panic。
    anyhow::bail!("register_node: unexpected retry exhaustion")
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
            environment_check_timeout_secs, allowed_model_dirs_json, collector_timeout_secs,
            collector_max_output_bytes, last_config_updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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

    // ── Device disappearance detection ──
    // Only run when collectors succeeded (no errors). If any collector failed,
    // we conservatively skip disappearance marking — a failed collector means
    // we can't distinguish "device gone" from "collector broken".
    if request.collector_errors.is_empty() {
        mark_missing_gpus(&mut tx, &request.node_id, request.sampled_at).await?;
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

        if status == "failed" {
            let _ = platform_log::append(
                &platform_log::global(),
                "server.log",
                "warn",
                &format!(
                    "heartbeat reconcile: instance={instance_id} node={node_id} running→failed: {reason}",
                    instance_id = report.instance_id,
                    node_id = node_id,
                    reason = report.message,
                ),
            )
            .await;
        }
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
        .bind(&id)
        .execute(&mut **tx)
        .await?;

        let _ = platform_log::append(
            &platform_log::global(),
            "server.log",
            "warn",
            &format!(
                "heartbeat reconcile: instance={id} node={node_id} running→failed: Agent 未上报该实例受管进程状态",
            ),
        )
        .await;
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
            status = NULL,
            last_error = NULL,
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

/// Mark GPUs for a node that were not present in the current heartbeat as missing.
///
/// Only call when `collector_errors` is empty — a failed collector should not
/// cause its devices to be marked disappeared.
async fn mark_missing_gpus(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    node_id: &str,
    sampled_at: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE gpu_status
        SET status = 'missing',
            last_error = 'device not found in latest discovery',
            updated_at = ?
        WHERE node_id = ? AND updated_at < ?
        "#,
    )
    .bind(sampled_at)
    .bind(node_id)
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
               temperature_celsius, power_watts, collector, status, last_error, updated_at
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
            status: row.get("status"),
            last_error: row.get("last_error"),
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

// ── Collector registry ──

pub async fn list_collector_registry(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<CollectorRegistryEntry>> {
    let rows = sqlx::query(
        "SELECT id, vendor, name, version, description, discover_sha256, metrics_sha256, enabled, created_at, updated_at FROM collector_registry ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| CollectorRegistryEntry {
            id: row.get("id"),
            vendor: row.get("vendor"),
            name: row.get("name"),
            version: row.get("version"),
            description: row.get("description"),
            discover_sha256: row.get("discover_sha256"),
            metrics_sha256: row.get("metrics_sha256"),
            enabled: row.get::<i64, _>("enabled") != 0,
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

pub async fn register_collector(
    pool: &SqlitePool,
    req: &RegisterCollectorRequest,
) -> anyhow::Result<CollectorRegistryEntry> {
    let now = now_unix_secs();
    sqlx::query(
        r#"
        INSERT INTO collector_registry (id, version, vendor, name, description, discover_sha256, metrics_sha256, enabled, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id, version) DO UPDATE SET
            vendor = excluded.vendor,
            name = excluded.name,
            description = excluded.description,
            discover_sha256 = excluded.discover_sha256,
            metrics_sha256 = excluded.metrics_sha256,
            enabled = excluded.enabled,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&req.id)
    .bind(&req.version)
    .bind(&req.vendor)
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.discover_sha256)
    .bind(&req.metrics_sha256)
    .bind(req.enabled as i64)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(CollectorRegistryEntry {
        id: req.id.clone(),
        vendor: req.vendor.clone(),
        name: req.name.clone(),
        version: req.version.clone(),
        description: req.description.clone(),
        discover_sha256: req.discover_sha256.clone(),
        metrics_sha256: req.metrics_sha256.clone(),
        enabled: req.enabled,
        created_at: now,
        updated_at: now,
    })
}

pub async fn update_collector_enabled(
    pool: &SqlitePool,
    id: &str,
    version: &str,
    enabled: bool,
) -> anyhow::Result<Option<CollectorRegistryEntry>> {
    let now = now_unix_secs();
    let result = sqlx::query(
        "UPDATE collector_registry SET enabled = ?, updated_at = ? WHERE id = ? AND version = ?",
    )
    .bind(enabled as i64)
    .bind(now)
    .bind(id)
    .bind(version)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Ok(None);
    }
    let row = sqlx::query("SELECT * FROM collector_registry WHERE id = ? AND version = ?")
        .bind(id)
        .bind(version)
        .fetch_one(pool)
        .await?;
    Ok(Some(CollectorRegistryEntry {
        id: row.get("id"),
        vendor: row.get("vendor"),
        name: row.get("name"),
        version: row.get("version"),
        description: row.get("description"),
        discover_sha256: row.get("discover_sha256"),
        metrics_sha256: row.get("metrics_sha256"),
        enabled: row.get::<i64, _>("enabled") != 0,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}
