use std::net::SocketAddr;
use std::path::PathBuf;

use lightai_gateway::config::Config;

const GATEWAY_VERSION: &str = env!("CARGO_PKG_VERSION");

const GATEWAY_HELP: &str = r#"lightai-gateway — LightAI independent model traffic data plane

USAGE:
    lightai-gateway [OPTIONS]

OPTIONS:
    --config <PATH>  Path to LightAI Gateway config TOML file
    --help           Show this help message
    --version        Show version information

DESCRIPTION:
    This first-round Gateway skeleton only exposes its own health endpoint.
    It does not implement model forwarding, OpenAI-compatible APIs, API Key,
    Usage, Quota, or Cost features.
"#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let config = match parse_cli_args(&args)? {
        CliAction::Help => {
            println!("{GATEWAY_HELP}");
            return Ok(());
        }
        CliAction::Version => {
            println!("lightai-gateway {GATEWAY_VERSION}");
            return Ok(());
        }
        CliAction::Run(config) => config,
    };

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(config.log_level.clone()));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_ansi(false)
        .init();

    let listen_addr: SocketAddr = config.listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    tracing::info!(service = "gateway", listen_addr = %listen_addr, "starting lightai gateway");

    axum::serve(listener, lightai_gateway::routes::app()).await?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    Help,
    Version,
    Run(Config),
}

fn parse_cli_args(args: &[String]) -> anyhow::Result<CliAction> {
    let mut config_path: Option<PathBuf> = None;
    let mut index = 1;

    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => return Ok(CliAction::Help),
            "--version" | "-V" => return Ok(CliAction::Version),
            "--config" => {
                let Some(path) = args.get(index + 1) else {
                    anyhow::bail!("--config requires a PATH");
                };
                if path.starts_with('-') {
                    anyhow::bail!("--config requires a PATH, got option '{path}'");
                }
                config_path = Some(PathBuf::from(path));
                index += 2;
            }
            other => {
                anyhow::bail!("unknown argument: {other}");
            }
        }
    }

    let config = match config_path {
        Some(path) => Config::from_file(path)?,
        None => Config::default(),
    };
    Ok(CliAction::Run(config))
}

#[cfg(test)]
mod tests {
    use super::{parse_cli_args, CliAction};
    use lightai_gateway::config::Config;

    #[test]
    fn parses_help() {
        let args = args(&["lightai-gateway", "--help"]);

        assert_eq!(parse_cli_args(&args).unwrap(), CliAction::Help);
    }

    #[test]
    fn parses_version() {
        let args = args(&["lightai-gateway", "--version"]);

        assert_eq!(parse_cli_args(&args).unwrap(), CliAction::Version);
    }

    #[test]
    fn loads_config_path() {
        let path = unique_temp_path("gateway-cli.toml");
        std::fs::write(
            &path,
            r#"
[gateway]
listen_addr = "127.0.0.1:19091"
log_level = "debug"
"#,
        )
        .unwrap();
        let args = args(&["lightai-gateway", "--config", path.to_str().unwrap()]);

        let action = parse_cli_args(&args).unwrap();

        assert_eq!(
            action,
            CliAction::Run(Config {
                listen_addr: "127.0.0.1:19091".to_string(),
                log_level: "debug".to_string(),
            })
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn config_without_path_returns_clear_error() {
        let args = args(&["lightai-gateway", "--config"]);

        let error = parse_cli_args(&args).unwrap_err().to_string();

        assert!(error.contains("--config requires a PATH"), "{error}");
    }

    #[test]
    fn config_followed_by_option_returns_clear_error() {
        let args = args(&["lightai-gateway", "--config", "--unknown"]);

        let error = parse_cli_args(&args).unwrap_err().to_string();

        assert!(
            error.contains("--config requires a PATH, got option '--unknown'"),
            "{error}"
        );
    }

    #[test]
    fn unknown_argument_returns_clear_error() {
        let args = args(&["lightai-gateway", "--unknown"]);

        let error = parse_cli_args(&args).unwrap_err().to_string();

        assert!(error.contains("unknown argument: --unknown"), "{error}");
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn unique_temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "lightai-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
