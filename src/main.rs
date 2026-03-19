mod config;
mod error;
mod models;
mod tools;
mod vault;

use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::tools::ObsidianMcp;
use crate::vault::Vault;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(&config.log_level))
        .with_writer(std::io::stderr)
        .init();

    tracing::info!(vault = %config.vault_path.display(), "starting obsidian-mcp");

    let vault = Vault::open(&config).await?;
    let server = ObsidianMcp::new(vault)
        .serve(rmcp::transport::io::stdio())
        .await?;

    server.waiting().await?;
    Ok(())
}
