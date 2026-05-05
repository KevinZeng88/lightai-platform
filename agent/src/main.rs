use std::net::SocketAddr;
use std::sync::Arc;

use lightai_agent::{config::Config, heartbeat, managed_process, platform_log, routes, tasks};
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::load()?;
    let default_log_policy = platform_log::LogPolicy::default();
    platform_log::append(&default_log_policy, "agent.log", "info", "Agent 启动").await?;
    let listen_addr: SocketAddr = config.listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    let heartbeat_config = config.clone();
    let task_config = config.clone();
    let state_path = config.state_path.clone();
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

    let managed_store_path = managed_process::store_path_from_state_path(&state_path);
    let record_count = managed_process::load(&managed_store_path)
        .await
        .map(|records| records.len())
        .unwrap_or(0);
    let _ = lightai_agent::platform_log::append(
        &lightai_agent::platform_log::LogPolicy::default(),
        "agent.log",
        "info",
        &format!(
            "Agent 正在退出，不会终止受管实例。managed store 保留 {record_count} 条受管进程记录。",
        ),
    )
    .await;

    Ok(())
}
