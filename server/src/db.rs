use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Executor, SqlitePool};
use std::str::FromStr;

pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    ensure_sqlite_parent_dir(database_url).await?;

    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    execute_migration(pool, include_str!("../../migrations/0001_init.sql")).await?;
    execute_migration(pool, include_str!("../../migrations/0002_stage2_nodes.sql")).await?;
    // Agent 身份规则：name 全局唯一、hostname 全局唯一。
    // 应用层 register_node 在事务中先检查冲突再写入，此处唯一索引作为并发兜底。
    // 若已有库存在违反约束的重复记录，需先手动合并后再升级。
    pool.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name)")
        .await?;
    pool.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_hostname ON nodes(hostname)")
        .await?;
    execute_migration(
        pool,
        include_str!("../../migrations/0003_stage3a_models.sql"),
    )
    .await?;
    migrate_stage3a_corrections(pool).await?;
    Ok(())
}

async fn ensure_sqlite_parent_dir(database_url: &str) -> anyhow::Result<()> {
    let path = database_url
        .strip_prefix("sqlite://")
        .unwrap_or(database_url);
    if path == ":memory:" || path.starts_with("file:") {
        return Ok(());
    }

    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    Ok(())
}

async fn execute_migration(pool: &SqlitePool, sql: &str) -> anyhow::Result<()> {
    for statement in sql.split(';') {
        let statement = statement.trim();
        if !statement.is_empty() && !statement.starts_with("--") {
            pool.execute(statement).await?;
        }
    }
    Ok(())
}

async fn migrate_stage3a_corrections(pool: &SqlitePool) -> anyhow::Result<()> {
    // This project does not yet have a migration ledger, so schema corrections that
    // rebuild SQLite tables must stay idempotent here. Plain SQL files cannot safely
    // express "rebuild this table only if this column is still NOT NULL" without a
    // larger migration framework, which is intentionally out of scope for Stage 3A.
    add_node_status_column_if_missing(pool, "agent_config_version", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "heartbeat_interval_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "metrics_sample_interval_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "task_poll_interval_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "config_refresh_interval_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "command_timeout_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "environment_check_timeout_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "allowed_model_dirs_json", "TEXT").await?;
    add_node_status_column_if_missing(pool, "collector_timeout_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "collector_max_output_bytes", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "last_config_updated_at", "INTEGER").await?;
    add_gpu_status_column_if_missing(pool, "status", "TEXT").await?;
    add_gpu_status_column_if_missing(pool, "last_error", "TEXT").await?;

    let columns = table_columns(pool, "model_instances").await?;
    let needs_rebuild = columns
        .iter()
        .any(|column| column.name == "model_id" && column.not_null)
        || columns
            .iter()
            .any(|column| column.name == "runtime_environment_id" && column.not_null)
        || !columns.iter().any(|column| column.name == "base_url")
        || !columns.iter().any(|column| column.name == "model_name")
        || !columns.iter().any(|column| column.name == "description");

    if needs_rebuild {
        rebuild_model_instances_table(pool).await?;
    }
    ensure_stage3b_tables(pool).await?;
    ensure_agent_config_tables(pool).await?;
    ensure_audit_tables(pool).await?;
    ensure_user_tables(pool).await?;
    ensure_platform_settings(pool).await?;
    ensure_collector_registry(pool).await?;

    Ok(())
}

async fn ensure_user_tables(pool: &SqlitePool) -> anyhow::Result<()> {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            password_changed_at INTEGER NOT NULL DEFAULT 0,
            must_change_password INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    )
    .await?;
    add_user_column_if_missing(pool, "password_changed_at", "INTEGER NOT NULL DEFAULT 0").await?;
    add_user_column_if_missing(pool, "must_change_password", "INTEGER NOT NULL DEFAULT 0").await?;
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS user_sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            token_hash TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            last_seen_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            revoked_at INTEGER
        )
        "#,
    )
    .await?;
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS user_groups (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            role TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    )
    .await?;
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS user_group_members (
            group_id TEXT NOT NULL REFERENCES user_groups(id) ON DELETE CASCADE,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (group_id, user_id)
        )
        "#,
    )
    .await?;
    pool.execute(
        "CREATE INDEX IF NOT EXISTS idx_user_group_members_user ON user_group_members(user_id)",
    )
    .await?;
    pool.execute("CREATE INDEX IF NOT EXISTS idx_user_sessions_token ON user_sessions(token_hash)")
        .await?;
    pool.execute(
        "CREATE INDEX IF NOT EXISTS idx_user_sessions_user_expires ON user_sessions(user_id, expires_at)",
    )
    .await?;
    Ok(())
}

async fn add_user_column_if_missing(
    pool: &SqlitePool,
    column: &str,
    column_type: &str,
) -> anyhow::Result<()> {
    let columns = table_columns(pool, "users").await?;
    if !columns.iter().any(|existing| existing.name == column) {
        pool.execute(format!("ALTER TABLE users ADD COLUMN {column} {column_type}").as_str())
            .await?;
    }
    Ok(())
}

async fn ensure_platform_settings(pool: &SqlitePool) -> anyhow::Result<()> {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS platform_settings (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    )
    .await?;
    Ok(())
}

async fn ensure_audit_tables(pool: &SqlitePool) -> anyhow::Result<()> {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS audit_events (
            id TEXT PRIMARY KEY,
            occurred_at INTEGER NOT NULL,
            actor_type TEXT NOT NULL,
            actor_id TEXT,
            actor_group_id TEXT,
            operation_type TEXT NOT NULL,
            target_type TEXT NOT NULL,
            target_id TEXT,
            node_id TEXT,
            instance_id TEXT,
            result TEXT NOT NULL,
            error_message TEXT,
            source TEXT NOT NULL,
            detail_json TEXT
        )
        "#,
    )
    .await?;
    pool.execute(
        "CREATE INDEX IF NOT EXISTS idx_audit_events_filters ON audit_events(occurred_at, operation_type, target_type, result)",
    )
    .await?;
    Ok(())
}

async fn ensure_agent_config_tables(pool: &SqlitePool) -> anyhow::Result<()> {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agent_config_policies (
            scope TEXT NOT NULL,
            node_id TEXT NOT NULL,
            policy_json TEXT NOT NULL,
            version INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (scope, node_id)
        )
        "#,
    )
    .await?;
    Ok(())
}

async fn ensure_stage3b_tables(pool: &SqlitePool) -> anyhow::Result<()> {
    execute_migration(
        pool,
        include_str!("../../migrations/0003_stage3a_models.sql"),
    )
    .await?;
    add_model_file_trash_column_if_missing(pool, "model_file_id", "TEXT").await?;
    add_model_file_trash_column_if_missing(pool, "file_deleted_at", "INTEGER").await?;
    add_model_file_trash_column_if_missing(pool, "cleanup_task_id", "TEXT").await?;
    add_model_file_trash_column_if_missing(pool, "last_error", "TEXT").await?;
    add_model_file_column_if_missing(pool, "deleted_at", "INTEGER").await?;
    add_model_file_column_if_missing(pool, "path_type", "TEXT").await?;
    add_runtime_environment_column_if_missing(pool, "endpoint_url", "TEXT").await?;
    add_model_instance_column_if_missing(pool, "model_file_id", "TEXT").await?;
    add_model_instance_column_if_missing(pool, "process_id", "INTEGER").await?;
    add_model_instance_column_if_missing(pool, "process_ref", "TEXT").await?;
    add_model_instance_column_if_missing(pool, "log_tail", "TEXT").await?;
    add_model_instance_column_if_missing(pool, "command", "TEXT").await?;
    Ok(())
}

async fn add_node_status_column_if_missing(
    pool: &SqlitePool,
    column: &str,
    column_type: &str,
) -> anyhow::Result<()> {
    let columns = table_columns(pool, "node_status").await?;
    if !columns.iter().any(|existing| existing.name == column) {
        pool.execute(format!("ALTER TABLE node_status ADD COLUMN {column} {column_type}").as_str())
            .await?;
    }
    Ok(())
}

async fn add_gpu_status_column_if_missing(
    pool: &SqlitePool,
    column: &str,
    column_type: &str,
) -> anyhow::Result<()> {
    let columns = table_columns(pool, "gpu_status").await?;
    if !columns.iter().any(|existing| existing.name == column) {
        pool.execute(format!("ALTER TABLE gpu_status ADD COLUMN {column} {column_type}").as_str())
            .await?;
    }
    Ok(())
}

async fn add_model_file_trash_column_if_missing(
    pool: &SqlitePool,
    column: &str,
    column_type: &str,
) -> anyhow::Result<()> {
    let columns = table_columns(pool, "model_file_trash").await?;
    if !columns.iter().any(|existing| existing.name == column) {
        pool.execute(
            format!("ALTER TABLE model_file_trash ADD COLUMN {column} {column_type}").as_str(),
        )
        .await?;
    }
    Ok(())
}

async fn add_model_file_column_if_missing(
    pool: &SqlitePool,
    column: &str,
    column_type: &str,
) -> anyhow::Result<()> {
    let columns = table_columns(pool, "model_files").await?;
    if !columns.iter().any(|existing| existing.name == column) {
        pool.execute(format!("ALTER TABLE model_files ADD COLUMN {column} {column_type}").as_str())
            .await?;
    }
    Ok(())
}

async fn add_runtime_environment_column_if_missing(
    pool: &SqlitePool,
    column: &str,
    column_type: &str,
) -> anyhow::Result<()> {
    let columns = table_columns(pool, "runtime_environments").await?;
    if !columns.iter().any(|existing| existing.name == column) {
        pool.execute(
            format!("ALTER TABLE runtime_environments ADD COLUMN {column} {column_type}").as_str(),
        )
        .await?;
    }
    Ok(())
}

async fn add_model_instance_column_if_missing(
    pool: &SqlitePool,
    column: &str,
    column_type: &str,
) -> anyhow::Result<()> {
    let columns = table_columns(pool, "model_instances").await?;
    if !columns.iter().any(|existing| existing.name == column) {
        pool.execute(
            format!("ALTER TABLE model_instances ADD COLUMN {column} {column_type}").as_str(),
        )
        .await?;
    }
    Ok(())
}

struct TableColumn {
    name: String,
    not_null: bool,
}

async fn table_columns(pool: &SqlitePool, table: &str) -> anyhow::Result<Vec<TableColumn>> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| TableColumn {
            name: sqlx::Row::get(&row, "name"),
            not_null: sqlx::Row::get::<i64, _>(&row, "notnull") != 0,
        })
        .collect())
}

async fn rebuild_model_instances_table(pool: &SqlitePool) -> anyhow::Result<()> {
    pool.execute("ALTER TABLE model_instances RENAME TO model_instances_old")
        .await?;
    pool.execute(
        r#"
        CREATE TABLE model_instances (
            id TEXT PRIMARY KEY,
            model_id TEXT REFERENCES models(id),
            model_file_id TEXT REFERENCES model_files(id),
            node_id TEXT REFERENCES nodes(id),
            runtime_environment_id TEXT REFERENCES runtime_environments(id),
            name TEXT NOT NULL,
            backend TEXT NOT NULL,
            deploy_type TEXT NOT NULL,
            status TEXT NOT NULL,
            base_url TEXT,
            endpoint_url TEXT,
            health_url TEXT,
            runtime_version TEXT,
            model_name TEXT,
            description TEXT,
            params_json TEXT,
            process_id INTEGER,
            process_ref TEXT,
            log_tail TEXT,
            command TEXT,
            last_checked_at INTEGER,
            last_error TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    )
    .await?;
    pool.execute(
        r#"
        INSERT INTO model_instances (
            id, model_id, node_id, runtime_environment_id, name, backend,
            deploy_type, status, endpoint_url, health_url, runtime_version,
            params_json, last_checked_at, last_error, created_at, updated_at
        )
        SELECT
            id, model_id, node_id, runtime_environment_id, name, backend,
            deploy_type, status, endpoint_url, health_url, runtime_version,
            params_json, last_checked_at, last_error, created_at, updated_at
        FROM model_instances_old
        "#,
    )
    .await?;
    pool.execute("DROP TABLE model_instances_old").await?;
    pool.execute(
        "CREATE INDEX IF NOT EXISTS idx_model_instances_model_status ON model_instances(model_id, status)",
    )
    .await?;
    pool.execute(
        "CREATE INDEX IF NOT EXISTS idx_model_instances_node_environment ON model_instances(node_id, runtime_environment_id)",
    )
    .await?;

    Ok(())
}

async fn ensure_collector_registry(pool: &SqlitePool) -> anyhow::Result<()> {
    let columns = table_columns(pool, "collector_registry")
        .await
        .unwrap_or_default();

    // Fresh DB: create table with correct schema.
    if columns.is_empty() {
        pool.execute(
            r#"
            CREATE TABLE IF NOT EXISTS collector_registry (
                id TEXT NOT NULL,
                version TEXT NOT NULL,
                vendor TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                discover_sha256 TEXT NOT NULL,
                metrics_sha256 TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (id, version)
            )
            "#,
        )
        .await?;
        return Ok(());
    }

    // Existing DB: check if the PK is the old id-only schema.
    let pk_info: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_table_info('collector_registry') WHERE pk > 0 ORDER BY pk",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    if pk_info == vec!["id"] {
        // Old schema (id-only PK): migrate to id+version PK safely.
        // 1. Create new table.
        // 2. Copy data (keep latest per id+version, grouping by id+version since
        //    the old schema couldn't have duplicate versions anyway).
        // 3. Drop old table.
        // 4. Rename new table.
        pool.execute(
            r#"
            CREATE TABLE collector_registry_new (
                id TEXT NOT NULL,
                version TEXT NOT NULL,
                vendor TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                discover_sha256 TEXT NOT NULL,
                metrics_sha256 TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (id, version)
            )
            "#,
        )
        .await?;
        pool.execute(
            r#"INSERT INTO collector_registry_new
               SELECT id, version, vendor, name, description,
                      discover_sha256, metrics_sha256, enabled,
                      created_at, updated_at
               FROM collector_registry"#,
        )
        .await?;
        pool.execute("DROP TABLE collector_registry").await?;
        pool.execute("ALTER TABLE collector_registry_new RENAME TO collector_registry")
            .await?;
    }

    // Ensure `version` column exists (for DBs that have the correct PK but
    // were created before the column was added).
    if !columns.iter().any(|c| c.name == "version") {
        pool.execute(
            "ALTER TABLE collector_registry ADD COLUMN version TEXT NOT NULL DEFAULT '0.0.0'",
        )
        .await?;
    }

    Ok(())
}
