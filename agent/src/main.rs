use std::net::SocketAddr;
use std::sync::Arc;

use lightai_agent::{config::Config, heartbeat, routes, tasks};
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::load()?;
    let listen_addr: SocketAddr = config.listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    let heartbeat_config = config.clone();
    let task_config = config.clone();
    let runtime_config = Arc::new(RwLock::new(heartbeat::RuntimeConfig::from_config(&config)));

    tracing::info!(
        service = "agent",
        listen_addr = %listen_addr,
        server_url = %config.server_url,
        "starting lightai agent"
    );

    let server = axum::serve(listener, routes::app());
    tokio::select! {
        result = server => result?,
        _ = heartbeat::run(heartbeat_config, runtime_config.clone()) => {}
        _ = tasks::run(task_config, runtime_config.clone()) => {}
    }

    Ok(())
}
