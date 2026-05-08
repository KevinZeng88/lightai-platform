use std::net::SocketAddr;

use lightai_server::{config::Config, db, platform_log, repository, routes};

const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

const SERVER_HELP: &str = r#"lightai-server — LightAI GPU 模型管理平台 Server

USAGE:
    lightai-server [OPTIONS]

OPTIONS:
    --help       Show this help message
    --version    Show version information
    --config <PATH>  Path to server config TOML file (env: LIGHTAI_SERVER_CONFIG)
    --reset-password <USERNAME> <PASSWORD>
                Reset a local user's password from the server host

DESCRIPTION:
    LightAI Server 是平台的中央控制面，负责：
    - Agent 注册、心跳鉴权、配置下发
    - 节点/GPU 状态管理和历史指标存储
    - Runtime / Model / Model File / Instance / Trash 管理
    - 平台日志和审计

CONFIGURATION:
    Environment variable: LIGHTAI_SERVER_CONFIG=<path>
    Empty databases enter Web setup mode for first-admin creation
    Default: embedded defaults (listen 127.0.0.1:8080, SQLite data/lightai.db)
"#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── CLI argument handling ──
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 {
        match args[1].as_str() {
            "--help" | "-h" => {
                println!("{SERVER_HELP}");
                return Ok(());
            }
            "--version" | "-V" => {
                println!("lightai-server {SERVER_VERSION}");
                return Ok(());
            }
            "--reset-password" => {
                if args.len() != 4 {
                    eprintln!("usage: lightai-server --reset-password <USERNAME> <PASSWORD>");
                    std::process::exit(1);
                }
                let config = Config::load()?;
                config.validate_auth()?;
                let pool = db::connect(&config.database_url).await?;
                repository::reset_user_password(
                    &pool,
                    &args[2],
                    &args[3],
                    config.password_policy.clone(),
                )
                .await?;
                println!("password reset completed for user {}", args[2]);
                return Ok(());
            }
            other => {
                eprintln!("unknown option: {other}");
                eprintln!("try: lightai-server --help");
                std::process::exit(1);
            }
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::load()?;
    config.validate_auth()?;
    platform_log::set_global(config.log_policy.clone());
    platform_log::append(&config.log_policy, "server.log", "info", "Server 启动").await?;
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

    axum::serve(
        listener,
        routes::app_with_auth_policies(
            pool,
            config.emergency_control_token,
            config.password_policy,
            config.session_policy,
        ),
    )
    .await?;
    Ok(())
}
