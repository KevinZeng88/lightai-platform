use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Executor, SqlitePool};
use std::str::FromStr;

pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    ensure_sqlite_parent_dir(database_url).await?;

    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
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
    add_node_status_column_if_missing(pool, "agent_config_version", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "heartbeat_interval_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "metrics_sample_interval_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "task_poll_interval_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "config_refresh_interval_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "command_timeout_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "environment_check_timeout_secs", "INTEGER").await?;
    add_node_status_column_if_missing(pool, "last_config_updated_at", "INTEGER").await?;

    let columns = table_columns(pool, "model_instances").await?;
    let needs_rebuild = columns
        .iter()
        .any(|column| column.name == "runtime_environment_id" && column.not_null)
        || !columns.iter().any(|column| column.name == "base_url")
        || !columns.iter().any(|column| column.name == "model_name")
        || !columns.iter().any(|column| column.name == "description");

    if needs_rebuild {
        rebuild_model_instances_table(pool).await?;
    }

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
            model_id TEXT NOT NULL REFERENCES models(id),
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
