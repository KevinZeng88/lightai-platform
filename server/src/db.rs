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
    execute_sql_file(pool, include_str!("../../migrations/0001_init.sql")).await?;
    execute_sql_file(pool, include_str!("../../migrations/0002_stage2_nodes.sql")).await?;
    // Agent identity rules: name globally unique, hostname globally unique.
    pool.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name)")
        .await?;
    pool.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_hostname ON nodes(hostname)")
        .await?;
    execute_sql_file(
        pool,
        include_str!("../../migrations/0003_stage3a_models.sql"),
    )
    .await?;
    execute_sql_file(pool, include_str!("../../migrations/0005_platform.sql")).await?;
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

async fn execute_sql_file(pool: &SqlitePool, sql: &str) -> anyhow::Result<()> {
    for statement in sql.split(';') {
        let statement = statement.trim();
        if !statement.is_empty() && !statement.starts_with("--") {
            pool.execute(statement).await?;
        }
    }
    Ok(())
}
