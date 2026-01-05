use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod api;
mod config;
mod middleware;
mod ui;
mod websocket;

use config::ServerConfig;

#[derive(Parser, Debug)]
#[command(name = "shiioo")]
#[command(about = "Virtual Company OS - Agentic Enterprise Orchestrator", long_about = None)]
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
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "shiioo=info,tower_http=debug".into()),
        )
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    let args = Args::parse();

    tracing::info!("Starting Shiioo Virtual Company OS");
    tracing::info!("Data directory: {}", args.data_dir.display());

    // Load configuration
    let config = ServerConfig::load(&args.config, args.data_dir)?;

    // Start API server
    let addr = format!("{}:{}", args.host, args.port);
    tracing::info!("Starting API server on {}", addr);

    api::serve(&addr, config).await?;

    Ok(())
}
