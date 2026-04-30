use std::net::SocketAddr;

use lightai_server::{config::Config, db, routes};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::load()?;
    let listen_addr: SocketAddr = config.listen_addr.parse()?;
    let pool = db::connect(&config.database_url).await?;
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;

    tracing::info!(
        service = "server",
        listen_addr = %listen_addr,
        database_url = %config.database_url,
        metrics_retention_days = config.metrics_retention_days,
        "starting lightai server"
    );

    axum::serve(listener, routes::app(pool)).await?;
    Ok(())
}
