use std::net::SocketAddr;

use lightai_agent::{config::Config, routes};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::default();
    let listen_addr: SocketAddr = config.listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;

    tracing::info!(
        service = "agent",
        listen_addr = %listen_addr,
        server_url = %config.server_url,
        "starting lightai agent"
    );

    axum::serve(listener, routes::app()).await?;
    Ok(())
}
