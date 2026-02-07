use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod api;
mod cli;
mod config;
mod graphql;
mod middleware;
mod ui;
mod websocket;

use config::ServerConfig;

#[derive(Parser, Debug)]
#[command(name = "shiioo")]
#[command(about = "Virtual Company OS — Talk to your Chief of Staff")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "shiioo.toml", global = true)]
    config: PathBuf,

    /// Data directory for storage
    #[arg(short, long, default_value = "./data", global = true)]
    data_dir: PathBuf,

    /// Output logs in JSON format (for production/log aggregation)
    #[arg(long, env = "SHIIOO_LOG_JSON", global = true)]
    log_json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the HTTP API server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "8080")]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
}

fn init_logging(json_format: bool) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "shiioo=info,tower_http=debug".into());

    if json_format {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_span_list(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_thread_ids(true),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt::layer()
                    .with_target(false)
                    .with_thread_ids(true)
                    .with_file(true)
                    .with_line_number(true),
            )
            .init();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Serve { port, host }) => {
            // Server mode: start the HTTP API
            init_logging(cli.log_json);

            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                data_dir = %cli.data_dir.display(),
                "Starting Shiioo Virtual Company OS"
            );

            let config = ServerConfig::load(&cli.config, cli.data_dir)?;

            let addr = format!("{}:{}", host, port);
            tracing::info!(addr = %addr, "Starting API server");

            api::serve(&addr, config).await?;

            tracing::info!("Shiioo shutdown complete");
        }
        None => {
            // REPL mode: conversational Chief of Staff
            let config = ServerConfig::load(&cli.config, cli.data_dir)?;
            cli::repl::run(config).await?;
        }
    }

    Ok(())
}
