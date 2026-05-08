use std::net::SocketAddr;
use std::sync::Arc;

use lightai_agent::{
    collector, config::Config, heartbeat, managed_process, platform_log, routes, tasks,
};
use tokio::sync::RwLock;

const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const AGENT_HELP: &str = r#"lightai-agent — LightAI GPU Model Management Platform Agent

USAGE:
    lightai-agent [OPTIONS]
    lightai-agent config <SUBCOMMAND>
    lightai-agent collector <SUBCOMMAND>

OPTIONS:
    --config <PATH>
                Path to LightAI Agent config TOML file.
                Env: LIGHTAI_AGENT_CONFIG.
                Default: /etc/lightai/lightai-agent.toml.
                Falls back to built-in defaults if the file does not exist.
    --help           Show this help message
    --version        Show version information

CONFIGURATION (priority order):
    1. --config <PATH>
    2. LIGHTAI_AGENT_CONFIG environment variable
    3. /etc/lightai/lightai-agent.toml
    4. Built-in defaults

    Config file supports [agent] and optional [gpu_collectors] sections.
    When [gpu_collectors].root is set, collectors must be registered in the
    Server registry before they execute (fail-closed).
    If [gpu_collectors] is omitted, no GPU collector scripts execute.

CONFIG SUBCOMMANDS:
    lightai-agent config init <PATH> [--force]
        Generate a configuration template file at <PATH>.
        Does NOT overwrite existing files unless --force is given.
        This command does not require a Server connection.

COLLECTOR SUBCOMMANDS:
    lightai-agent collector inspect <DIR>
        Inspect a collector directory and output registry-ready JSON.
        The output can be pasted into the Web collector registry page.
        This command does NOT register, approve, or execute any script.

DESCRIPTION:
    LightAI Agent runs on GPU nodes. Responsibilities:
    - Register with Server and report heartbeat
    - Execute GPU device discovery and metrics collection
    - Execute controlled tasks: model verification, instance start/stop, file cleanup
"#;

const CONFIG_HELP: &str = r#"lightai-agent config — Configuration file management

USAGE:
    lightai-agent config init [PATH] [--force]

SUBCOMMANDS:
    init    Generate a LightAI Agent config template.
            If PATH is omitted, writes ./lightai-agent.toml
            in the current working directory.
            Use --force to overwrite an existing file.

EXAMPLES:
    lightai-agent config init
    lightai-agent config init ./my-agent.toml
    lightai-agent config init /etc/lightai/lightai-agent.toml
    lightai-agent config init --force
    lightai-agent config init /etc/lightai/lightai-agent.toml --force

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
        &format!(
            "Agent starting, config source: {}",
            Config::source_label(source)
        ),
    )
    .await?;

    // ── GPU collector config diagnostics ──
    log_gpu_collector_diagnostics(&config, &default_log_policy).await?;

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
            "Agent exiting without terminating managed instances. managed store retained {record_count} record(s).",
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
            if args.get(3).map(String::as_str) == Some("--help")
                || args.get(3).map(String::as_str) == Some("-h")
            {
                println!("{CONFIG_HELP}");
                return Ok(());
            }

            let mut path: Option<&str> = None;
            let mut force = false;
            for arg in args.iter().skip(3) {
                match arg.as_str() {
                    "--force" => force = true,
                    "--help" | "-h" => {
                        println!("{CONFIG_HELP}");
                        return Ok(());
                    }
                    other if !other.starts_with('-') => path = Some(other),
                    _ => {}
                }
            }

            let dest_path = path.unwrap_or("lightai-agent.toml");
            let dest = std::path::Path::new(dest_path);
            if let Some(parent) = dest.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
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
    lightai-agent collector inspect /opt/lightai/collectors/gpu/nvidia-wsl

    The inspect command:
    - Reads collector.toml from the directory
    - Computes SHA-256 of discover.sh and metrics.sh (in-process, no sha256sum)
    - Checks file permissions (no symlinks, no world-writable)
    - Outputs JSON to stdout for pasting into Web 'collector registry' page
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
                     The output can be pasted into the Web collector registry page.\n\
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

/// Emit structured GPU collector diagnostics on Agent startup.
async fn log_gpu_collector_diagnostics(
    config: &lightai_agent::config::Config,
    log_policy: &lightai_agent::platform_log::LogPolicy,
) -> anyhow::Result<()> {
    if config.collector_root.is_none() {
        lightai_agent::platform_log::append(
            log_policy,
            "agent.log",
            "warn",
            "GPU collector: [gpu_collectors].root is not configured; no GPU scripts will execute. \
             Configure collector_root and register via the Web collector registry page.",
        )
        .await?;
        return Ok(());
    }

    let root = config.collector_root.as_deref().unwrap_or("");
    lightai_agent::platform_log::append(
        log_policy,
        "agent.log",
        "info",
        &format!(
            "GPU collector: root={root}, mode={}, enabled={:?}, disabled={:?}",
            config.collector_mode, config.collector_enabled, config.collector_disabled,
        ),
    )
    .await?;

    // Scan for local collector directories and report find/not-found.
    let dirs = lightai_agent::collector::scan_collector_dirs(&std::path::PathBuf::from(root));
    if dirs.is_empty() {
        lightai_agent::platform_log::append(
            log_policy,
            "agent.log",
            "warn",
            &format!(
                "GPU collector: no valid collector directories found under root={root}.\
                 Each collector directory requires collector.toml + discover.sh + metrics.sh."
            ),
        )
        .await?;
        return Ok(());
    }

    for dir in &dirs {
        lightai_agent::platform_log::append(
            log_policy,
            "agent.log",
            "info",
            &format!(
                "GPU collector found: id={}, vendor={}, name={}, version={}, \
                 discover_sha256={}, metrics_sha256={}",
                dir.manifest.id,
                dir.manifest.vendor,
                dir.manifest.name,
                dir.manifest.version,
                &dir.discover_sha256[..dir.discover_sha256.len().min(16)],
                &dir.metrics_sha256[..dir.metrics_sha256.len().min(16)],
            ),
        )
        .await?;
    }

    lightai_agent::platform_log::append(
        log_policy,
        "agent.log",
        "info",
        &format!(
            "GPU collector: found {} collector dir(s) (mode={}）。\
             Scripts require Server registry hash verification before execution.",
            dirs.len(),
            config.collector_mode,
        ),
    )
    .await?;
    Ok(())
}
