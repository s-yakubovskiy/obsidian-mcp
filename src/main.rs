#[cfg(any(unix, test))]
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use obsidian_mcp::client::semantic_daemon::{DaemonConnectPolicy, SemanticDaemonClient};
use obsidian_mcp::config::{
    Config, SemanticMode, SemanticRuntimeConfig, Transport, parse_cli_args,
};
use obsidian_mcp::daemon::bootstrap::{BootstrapConfig, ensure_daemon};
use obsidian_mcp::daemon::server::IpcEndpoint;
use obsidian_mcp::error::VaultError;
use obsidian_mcp::tools::{ObsidianMcp, SemanticRuntime};
use obsidian_mcp::vault::Vault;

const DAEMON_DISABLED_BY_WATCH_REASON: &str =
    "semantic daemon disabled because OBSIDIAN_WATCH is false";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(code) = handle_cli_flags() {
        std::process::exit(code);
    }

    let cli = parse_cli_args();
    let config = Config::load(&cli)?;
    let semantic_runtime_config = SemanticRuntimeConfig::load_from_env();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(&config.log_level))
        .with_writer(std::io::stderr)
        .init();

    tracing::info!(
        vault = %config.vault_path.display(),
        transport = ?config.transport,
        "starting obsidian-mcp"
    );

    let semantic_runtime = init_semantic_runtime(&config, &semantic_runtime_config).await;
    tracing::info!(
        semantic_mode = semantic_runtime.mode.as_str(),
        daemon_ready = semantic_runtime.daemon_client.is_some(),
        "semantic runtime configured"
    );

    let vault = Vault::open(&config).await?;

    match config.transport {
        Transport::Stdio => {
            let server = ObsidianMcp::new(vault, config.hybrid_alpha, semantic_runtime)
                .serve(rmcp::transport::io::stdio())
                .await?;
            server.waiting().await?;
        }
        Transport::Http => {
            serve_http(vault, config.hybrid_alpha, semantic_runtime, &config).await?;
        }
    }

    Ok(())
}

async fn serve_http(
    vault: Vault,
    hybrid_alpha: f32,
    semantic_runtime: SemanticRuntime,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    use axum::{Router, routing::get};
    use rmcp::transport::StreamableHttpServerConfig;
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, tower::StreamableHttpService,
    };

    let mut mcp_config = StreamableHttpServerConfig::default();
    mcp_config.stateful_mode = true;
    mcp_config.json_response = true;

    let mcp_service: StreamableHttpService<ObsidianMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(ObsidianMcp::new(
                    vault.clone(),
                    hybrid_alpha,
                    semantic_runtime.clone(),
                ))
            },
            Arc::new(LocalSessionManager::default()),
            mcp_config,
        );

    let app = Router::new()
        .route("/health", get(health_handler))
        .nest_service("/mcp", mcp_service);

    let addr = std::net::SocketAddr::new(config.http_host, config.http_port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "HTTP MCP server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "server": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.ok() };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .ok()?
            .recv()
            .await
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<Option<()>>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received Ctrl+C, shutting down"),
        _ = terminate => tracing::info!("received SIGTERM, shutting down"),
    }
}

struct InitializedDaemonClient {
    client: SemanticDaemonClient,
    #[cfg(feature = "embeddings")]
    semantic_home: Option<PathBuf>,
}

async fn init_semantic_runtime(
    config: &Config,
    runtime_cfg: &SemanticRuntimeConfig,
) -> SemanticRuntime {
    let mut runtime = SemanticRuntime {
        mode: runtime_cfg.mode,
        daemon_client: None,
        daemon_unavailable_reason: None,
        prefetch_count: runtime_cfg.prefetch_count,
        vault_ensured: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    if runtime_cfg.mode == SemanticMode::Local {
        return runtime;
    }
    if !config.watch {
        runtime.daemon_unavailable_reason = Some(DAEMON_DISABLED_BY_WATCH_REASON.to_string());
        tracing::info!("semantic daemon disabled because OBSIDIAN_WATCH=false");
        return runtime;
    }

    match initialize_daemon_client(runtime_cfg).await {
        Ok(initialized) => {
            #[cfg(feature = "embeddings")]
            if let Some(semantic_home) = initialized.semantic_home.as_deref() {
                match obsidian_mcp::vault::embeddings::migrate_legacy_cache_to_daemon_store(
                    &config.vault_path,
                    semantic_home,
                ) {
                    Ok(obsidian_mcp::vault::embeddings::LegacyCacheMigration::Migrated(path)) => {
                        tracing::info!(
                            path = %path.display(),
                            "migrated legacy local embedding cache into daemon namespace store"
                        );
                    }
                    Ok(obsidian_mcp::vault::embeddings::LegacyCacheMigration::AlreadyPresent(
                        path,
                    )) => {
                        tracing::debug!(
                            path = %path.display(),
                            "daemon embedding cache already present; skipping legacy cache migration"
                        );
                    }
                    Ok(obsidian_mcp::vault::embeddings::LegacyCacheMigration::NotFound) => {}
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "failed to migrate legacy embedding cache to daemon namespace"
                        );
                    }
                }
            }

            runtime.daemon_client = Some(initialized.client);
        }
        Err(err) => {
            let reason = err.to_string();
            runtime.daemon_unavailable_reason = Some(reason.clone());
            match runtime_cfg.mode {
                SemanticMode::Daemon => {
                    tracing::error!(
                        error = %reason,
                        "semantic daemon mode requested but daemon is unavailable"
                    );
                }
                SemanticMode::Auto => {
                    tracing::warn!(
                        error = %reason,
                        "semantic daemon unavailable; auto mode may fall back to local backend"
                    );
                }
                SemanticMode::Local => {}
            }
        }
    }

    runtime
}

async fn initialize_daemon_client(
    runtime_cfg: &SemanticRuntimeConfig,
) -> Result<InitializedDaemonClient, VaultError> {
    let policy = DaemonConnectPolicy::from_runtime_config(runtime_cfg);
    let initialized = if let Some(raw_endpoint) = runtime_cfg.daemon_endpoint_override.as_deref() {
        InitializedDaemonClient {
            client: SemanticDaemonClient::new(endpoint_from_override(raw_endpoint), policy),
            #[cfg(feature = "embeddings")]
            semantic_home: None,
        }
    } else {
        let bootstrap_result = ensure_daemon(&BootstrapConfig {
            semantic_home_override: runtime_cfg.semantic_home_override.clone(),
            daemon_path_override: runtime_cfg.daemon_path_override.clone(),
            model_name: runtime_cfg.model_name.clone(),
            download_url_override: runtime_cfg.daemon_download_url.clone(),
            bootstrap_client_name: "obsidian-mcp".to_string(),
            bootstrap_client_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .await?;
        InitializedDaemonClient {
            client: SemanticDaemonClient::new(bootstrap_result.endpoint, policy),
            #[cfg(feature = "embeddings")]
            semantic_home: Some(bootstrap_result.semantic_home),
        }
    };

    let health = initialized
        .client
        .health("obsidian-mcp", env!("CARGO_PKG_VERSION"))
        .await?;
    tracing::info!(
        daemon_version = %health.daemon_version,
        daemon_api_version = health.daemon_api_version,
        daemon_status = %health.status,
        daemon_semantic_home = %health.semantic_home,
        "semantic daemon connection established"
    );
    Ok(initialized)
}

fn handle_cli_flags() -> Option<i32> {
    let arg = std::env::args().nth(1)?;
    match arg.as_str() {
        "--version" | "-v" => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            Some(0)
        }
        "--help" | "-h" | "help" => {
            print_help();
            Some(0)
        }
        "serve" => {
            if std::env::args().any(|a| a == "--help" || a == "-h") {
                print_help();
                return Some(0);
            }
            match daemonize() {
                Ok(()) => Some(0),
                Err(e) => {
                    eprintln!("error: {e}");
                    Some(1)
                }
            }
        }
        _ => None,
    }
}

fn print_help() {
    println!(
        "{name} {version} — {description}\n\
         \n\
         USAGE:\n    \
             {name} [OPTIONS] [VAULT_PATH]          Run with stdio transport (default)\n    \
             {name} --http [OPTIONS] [VAULT_PATH]   Run with Streamable HTTP transport\n    \
             {name} serve [OPTIONS] [VAULT_PATH]    Start HTTP server in background\n\
         \n\
         The 'serve' command daemonizes and logs to a platform-specific file:\n    \
             macOS:   ~/Library/Logs/obsidian-mcp.log\n    \
             Linux:   $XDG_STATE_HOME/obsidian-mcp/obsidian-mcp.log\n    \
             Windows: %LOCALAPPDATA%/obsidian-mcp/obsidian-mcp.log\n\
         \n\
         ARGUMENTS:\n    \
             VAULT_PATH    Path to Obsidian vault (or set OBSIDIAN_VAULT_PATH)\n\
         \n\
         OPTIONS:\n    \
             -h, --help           Print this help message\n    \
             -v, --version        Print version\n    \
             --http               Use Streamable HTTP transport instead of stdio\n    \
             --port <PORT>        HTTP listen port                  [default: 37842]\n    \
             --host <ADDR>        HTTP bind address                 [default: 127.0.0.1]\n\
         \n\
         ENVIRONMENT VARIABLES:\n    \
             OBSIDIAN_VAULT_PATH     Vault root (required if not passed as argument)\n    \
             OBSIDIAN_TRANSPORT      Transport: stdio | http        [default: stdio]\n    \
             OBSIDIAN_HTTP_PORT      HTTP listen port               [default: 37842]\n    \
             OBSIDIAN_HTTP_HOST      HTTP bind address              [default: 127.0.0.1]\n    \
             OBSIDIAN_WATCH          Enable filesystem watcher      [default: true]\n    \
             OBSIDIAN_LOG_LEVEL      Tracing log level              [default: info]\n    \
             OBSIDIAN_TANTIVY        Enable BM25 full-text index    [default: true]\n    \
             OBSIDIAN_EMBEDDINGS     Enable semantic embeddings     [default: false]\n    \
             OBSIDIAN_HYBRID_ALPHA   BM25/semantic blend weight     [default: 0.25]",
        name = env!("CARGO_PKG_NAME"),
        version = env!("CARGO_PKG_VERSION"),
        description = env!("CARGO_PKG_DESCRIPTION"),
    );
}

/// Spawn a detached child running `--http` and exit the parent.
fn daemonize() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;

    let mut child_args = vec!["--http".to_string()];
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "serve" {
            continue;
        }
        let takes_value = arg == "--port" || arg == "--host";
        child_args.push(arg);
        if takes_value && let Some(val) = args.next() {
            child_args.push(val);
        }
    }

    let log_file = daemon_log_path()?;
    if let Some(parent) = log_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stderr_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;

    let mut cmd = std::process::Command::new(&exe);
    cmd.args(&child_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd.spawn()?;

    std::thread::sleep(std::time::Duration::from_millis(150));

    match child.try_wait()? {
        Some(status) if !status.success() => Err(format!(
            "server exited immediately (exit code: {})\ncheck logs: {}",
            status,
            log_file.display()
        )
        .into()),
        Some(_) => Err(format!(
            "server exited immediately\ncheck logs: {}",
            log_file.display()
        )
        .into()),
        None => {
            eprintln!(
                "{name} HTTP server started (PID {pid})\nlogs: {log}",
                name = env!("CARGO_PKG_NAME"),
                pid = child.id(),
                log = log_file.display(),
            );
            Ok(())
        }
    }
}

fn daemon_log_path() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME")?;
        Ok(std::path::PathBuf::from(home)
            .join("Library/Logs")
            .join("obsidian-mcp.log"))
    }
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var("LOCALAPPDATA")?;
        Ok(std::path::PathBuf::from(local)
            .join("obsidian-mcp")
            .join("obsidian-mcp.log"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let state = std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            format!("{home}/.local/state")
        });
        Ok(std::path::PathBuf::from(state)
            .join("obsidian-mcp")
            .join("obsidian-mcp.log"))
    }
}

fn endpoint_from_override(raw: &str) -> IpcEndpoint {
    #[cfg(unix)]
    {
        IpcEndpoint::UnixSocket(PathBuf::from(raw))
    }
    #[cfg(windows)]
    {
        IpcEndpoint::NamedPipe(raw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_config(mode: SemanticMode) -> SemanticRuntimeConfig {
        SemanticRuntimeConfig {
            mode,
            semantic_home_override: None,
            daemon_path_override: None,
            daemon_endpoint_override: Some("/tmp/semanticd.sock".to_string()),
            daemon_download_url: None,
            model_name: "BAAI/bge-small-en-v1.5".to_string(),
            connect_timeout_ms: 2_000,
            connect_retries: 2,
            retry_backoff_ms: 250,
            prefetch_count: 50,
        }
    }

    #[tokio::test]
    async fn watch_disabled_skips_daemon_initialization() {
        let config = Config {
            vault_path: PathBuf::from("/tmp/test-vault"),
            watch: false,
            log_level: "error".to_string(),
            transport: Transport::Stdio,
            http_host: obsidian_mcp::config::DEFAULT_HTTP_HOST,
            http_port: obsidian_mcp::config::DEFAULT_HTTP_PORT,
            tantivy: true,
            embeddings: false,
            embeddings_model: "BAAI/bge-small-en-v1.5".to_string(),
            hybrid_alpha: 0.25,
        };
        let runtime = init_semantic_runtime(&config, &runtime_config(SemanticMode::Daemon)).await;
        assert!(runtime.daemon_client.is_none());
        assert_eq!(
            runtime.daemon_unavailable_reason.as_deref(),
            Some(DAEMON_DISABLED_BY_WATCH_REASON)
        );
    }
}
