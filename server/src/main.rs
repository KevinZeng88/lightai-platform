use std::net::SocketAddr;

use lightai_server::{config::Config, routes};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::default();
    let listen_addr: SocketAddr = config.listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;

    tracing::info!(
        service = "server",
        listen_addr = %listen_addr,
        database_url = %config.database_url,
        "starting lightai server"
    );

    axum::serve(listener, routes::app()).await?;
    Ok(())
}
