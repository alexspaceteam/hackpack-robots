use clap::Parser;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

mod slip;
mod connection;
mod manifest;
mod protocol;
mod server;

use connection::ConnectionManager;
use manifest::ManifestManager;
use server::McpServer;

#[derive(Parser)]
#[command(name = "arduino-mcp-adapter")]
#[command(about = "MCP adapter for serial Arduino devices")]
struct Cli {
    /// Serial line (e.g. /dev/ttyUSB0)
    #[arg(short, long)]
    line: String,
    
    /// JSON manifest directory
    #[arg(short, long)]
    manifest_dir: PathBuf,
    
    /// HTTP port for MCP server
    #[arg(short, long, default_value = "8080")]
    port: u16,
    
    /// Baud rate
    #[arg(short, long, default_value = "115200")]
    baud: u32,
}


#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    let cli = Cli::parse();
    
    info!("Starting Arduino MCP Adapter");
    info!("Serial line: {}", cli.line);
    info!("Manifest directory: {}", cli.manifest_dir.display());
    info!("HTTP port: {}", cli.port);
    
    // Create managers
    let connection_manager = Arc::new(ConnectionManager::new(cli.line, cli.baud));
    let manifest_manager = Arc::new(ManifestManager::new(cli.manifest_dir));
    
    // List available manifests
    match manifest_manager.list_available_manifests() {
        Ok(manifests) => {
            if manifests.is_empty() {
                info!("No manifest files found in manifest directory");
            } else {
                info!("Available device manifests: {:?}", manifests);
            }
        }
        Err(e) => {
            info!("Warning: Could not list manifests: {}", e);
        }
    }
    
    // Create and start MCP server
    let server = McpServer::new(connection_manager, manifest_manager);
    server.start(cli.port).await?;
    
    Ok(())
}

