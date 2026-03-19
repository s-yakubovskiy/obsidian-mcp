use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use obsidian_mcp::config::Config;
use obsidian_mcp::tools::ObsidianMcp;
use obsidian_mcp::vault::Vault;

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
