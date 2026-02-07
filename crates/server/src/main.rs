use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod api;
mod config;
mod graphql;
mod middleware;
mod ui;
mod websocket;

use config::ServerConfig;

#[derive(Parser, Debug)]
#[command(name = "shiioo")]
#[command(about = "Virtual Company OS — Run your virtual company", long_about = None)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = "shiioo.toml")]
    config: PathBuf,

    /// Data directory for storage
    #[arg(short, long, default_value = "./data")]
    data_dir: PathBuf,

    /// Port to listen on
    #[arg(short, long, default_value = "8080")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Output logs in JSON format (for production/log aggregation)
    #[arg(long, env = "SHIIOO_LOG_JSON")]
    log_json: bool,
}

fn init_logging(json_format: bool) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "shiioo=info,tower_http=debug".into());

    if json_format {
        // JSON format for production/log aggregation
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
        // Human-readable format for development
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
    let args = Args::parse();

    // Initialize structured logging
    init_logging(args.log_json);

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data_dir = %args.data_dir.display(),
        "Starting Shiioo Virtual Company OS"
    );

    // Load configuration
    let config = ServerConfig::load(&args.config, args.data_dir)?;

    // Start API server with graceful shutdown
    let addr = format!("{}:{}", args.host, args.port);
    tracing::info!(addr = %addr, "Starting API server");

    api::serve(&addr, config).await?;

    tracing::info!("Shiioo shutdown complete");
    Ok(())
}
