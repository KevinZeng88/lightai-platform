use std::net::SocketAddr;
use std::sync::Arc;

use lightai_agent::{
    collector, config::Config, heartbeat, managed_process, platform_log, routes, tasks,
};
use tokio::sync::RwLock;

const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const AGENT_HELP: &str = r#"lightai-agent — LightAI GPU 模型管理平台 Agent

USAGE:
    lightai-agent [OPTIONS]
    lightai-agent config <SUBCOMMAND>
    lightai-agent collector <SUBCOMMAND>

OPTIONS:
    --config <PATH>  Path to agent config TOML file
    --help           Show this help message
    --version        Show version information

CONFIGURATION (priority order):
    1. --config <PATH>
    2. LIGHTAI_AGENT_CONFIG environment variable
    3. <executable_dir>/agent.toml
    4. <executable_dir>/lightai-agent.toml
    5. Built-in defaults

    Config file supports [agent] and optional [gpu_collectors] sections.
    When [gpu_collectors].root is set, collectors must be registered in the
    Server registry before they execute (fail-closed).
    If [gpu_collectors] is omitted, the legacy built-in NVIDIA/custom path is used.

CONFIG SUBCOMMANDS:
    lightai-agent config init <PATH> [--force]
        Generate a configuration template file at <PATH>.
        Does NOT overwrite existing files unless --force is given.
        This command does not require a Server connection.

COLLECTOR SUBCOMMANDS:
    lightai-agent collector inspect <DIR>
        Inspect a collector directory and output registry-ready JSON.
        The output can be pasted into the Web '采集器登记' page.
        This command does NOT register, approve, or execute any script.

DESCRIPTION:
    LightAI Agent 运行在 GPU 节点上，负责：
    - 主动向 Server 注册并上报心跳
    - 执行 GPU 设备发现和指标采集
    - 受控执行模型验证、实例启停、文件清理等任务
"#;

const CONFIG_HELP: &str = r#"lightai-agent config — Configuration file management

USAGE:
    lightai-agent config init <PATH> [--force]

SUBCOMMANDS:
    init    Generate a configuration template at <PATH>.

EXAMPLES:
    lightai-agent config init ./agent.toml
    lightai-agent config init /etc/lightai/agent.toml --force

    The generated template includes:
    - [agent] section with defaults
    - [gpu_collectors] section (commented out by default)
    - Inline comments explaining each field
"#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── CLI argument handling ──
    let args: Vec<String> = std::env::args().collect();

    if args.len() >= 2 {
        match args[1].as_str() {
            "--help" | "-h" => {
                println!("{AGENT_HELP}");
                return Ok(());
            }
            "--version" | "-V" => {
                println!("lightai-agent {AGENT_VERSION}");
                return Ok(());
            }
            "config" => return handle_config_cmd(&args),
            "collector" => return handle_collector_cmd(&args),
            "--config" => {
                // --config <PATH> — continue to normal startup below.
            }
            other => {
                if !other.starts_with('-') {
                    eprintln!("unknown command: {other}");
                } else {
                    eprintln!("unknown option: {other}");
                }
                eprintln!("try: lightai-agent --help");
                std::process::exit(1);
            }
        }
    }

    // Extract --config <PATH> from args.
    let cli_config_path = extract_config_path(&args);

    // ── Normal Agent startup ──

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let (config, source) = Config::load_with_priority(cli_config_path.as_deref())?;
    let default_log_policy = platform_log::LogPolicy::default();

    platform_log::append(
        &default_log_policy,
        "agent.log",
        "info",
        &format!("Agent 启动，配置来源: {}", Config::source_label(source)),
    )
    .await?;

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
        config_source = %Config::source_label(source),
        collector_root = ?config.collector_root,
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

/// Extract `--config <PATH>` from args.  Returns None if not present.
fn extract_config_path(args: &[String]) -> Option<String> {
    for i in 1..args.len() {
        if args[i] == "--config" {
            return args.get(i + 1).cloned();
        }
    }
    None
}

// ── config subcommand ──

fn handle_config_cmd(args: &[String]) -> anyhow::Result<()> {
    if args.len() < 3 {
        match args.get(2).map(String::as_str) {
            Some("--help" | "-h") | None => {
                println!("{CONFIG_HELP}");
                return Ok(());
            }
            _ => {}
        }
        eprintln!("missing config subcommand");
        eprintln!("try: lightai-agent config --help");
        std::process::exit(1);
    }

    match args[2].as_str() {
        "--help" | "-h" => {
            println!("{CONFIG_HELP}");
            Ok(())
        }
        "init" => {
            if args.len() >= 4 && (args[3] == "--help" || args[3] == "-h") {
                println!(
                    "USAGE: lightai-agent config init <PATH> [--force]\n\n\
                     Generate a configuration template at <PATH>.\n\
                     Does NOT overwrite existing files unless --force is given.\n\
                     This command does not require a Server connection."
                );
                return Ok(());
            }

            let path = args.get(3).map(String::as_str).unwrap_or("agent.toml");
            let force = args.get(4).map(String::as_str) == Some("--force");

            let dest = std::path::Path::new(path);
            if dest.exists() && !force {
                anyhow::bail!(
                    "'{}' already exists. Use --force to overwrite.",
                    dest.display()
                );
            }

            std::fs::write(dest, lightai_agent::config::CONFIG_TEMPLATE)?;
            println!("Config template written to '{}'", dest.display());
            Ok(())
        }
        other => {
            eprintln!("unknown config subcommand: {other}");
            eprintln!("try: lightai-agent config --help");
            std::process::exit(1);
        }
    }
}

// ── collector subcommand ──

fn handle_collector_cmd(args: &[String]) -> anyhow::Result<()> {
    if args.len() < 3 {
        eprintln!("missing collector subcommand");
        eprintln!("try: lightai-agent collector --help");
        std::process::exit(1);
    }

    match args[2].as_str() {
        "--help" | "-h" => {
            println!(
                r#"lightai-agent collector — GPU collector management

USAGE:
    lightai-agent collector inspect <DIR>

SUBCOMMANDS:
    inspect  Inspect a collector directory and output registry JSON.

EXAMPLES:
    lightai-agent collector inspect /opt/lightai/collectors/gpu/nvidia

    The inspect command:
    - Reads collector.toml from the directory
    - Computes SHA-256 of discover.sh and metrics.sh (in-process, no sha256sum)
    - Checks file permissions (no symlinks, no world-writable)
    - Outputs JSON to stdout for pasting into Web '采集器登记' page
    - Does NOT register, approve, or execute any script
"#
            );
            Ok(())
        }
        "inspect" => {
            if args.len() >= 4 && (args[3] == "--help" || args[3] == "-h") {
                println!(
                    "USAGE: lightai-agent collector inspect <DIR>\n\n\
                     Inspect a collector directory and output registry JSON.\n\
                     The output can be pasted into the Web '采集器登记' page.\n\
                     This command does NOT register, approve, or execute scripts."
                );
                return Ok(());
            }
            let dir = args.get(3).map(String::as_str).unwrap_or(".");
            let path = std::path::Path::new(dir);
            let output = collector::inspect::inspect(path)?;
            let json = serde_json::to_string_pretty(&output)?;
            println!("{json}");
            Ok(())
        }
        other => {
            eprintln!("unknown collector subcommand: {other}");
            eprintln!("try: lightai-agent collector --help");
            std::process::exit(1);
        }
    }
}
