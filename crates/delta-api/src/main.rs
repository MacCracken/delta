use clap::Parser;
use delta_api::{routes, state::AppState};
use delta_core::{DeltaConfig, db};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "delta",
    about = "Delta — code hosting, CI/CD, and artifact registry"
)]
struct Cli {
    /// Config file path
    #[arg(short, long, default_value = "/etc/delta/config.toml")]
    config: String,

    /// Override listen port
    #[arg(short, long)]
    port: Option<u16>,

    /// Emit structured JSON logs (AGNOS journald compatible)
    #[arg(long, default_value_t = false)]
    json_log: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let env_filter =
        EnvFilter::from_default_env().add_directive("delta=info".parse()?);
    if cli.json_log {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .init();
    }

    let mut config = if std::path::Path::new(&cli.config).exists() {
        let contents = std::fs::read_to_string(&cli.config)?;
        toml::from_str(&contents)?
    } else {
        tracing::warn!("config file not found, using defaults");
        DeltaConfig::default()
    };

    if let Some(port) = cli.port {
        config.server.port = port;
    }

    if config.auth.secrets_key == "delta-change-me-in-production"
        || config.auth.secrets_key == "change-me-to-a-strong-random-passphrase"
    {
        tracing::warn!(
            "secrets_key is set to a default value — pipeline secrets are NOT secure. \
             Set auth.secrets_key in your config file."
        );
    }

    let pool = db::init_pool(&config.storage.db_url).await?;

    // Clone pool for SSH before moving into AppState
    let ssh_pool = pool.clone();

    let state = AppState::new(config.clone(), pool);
    let app = routes::router(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("delta HTTP listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // Start SSH server if enabled
    if config.ssh.enabled {
        let ssh_config = config.ssh.clone();
        let ssh_repos_dir = config.storage.repos_dir.clone();
        let ssh_host = config.server.host.clone();

        tokio::spawn(async move {
            if let Err(e) =
                delta_api::ssh::start_ssh_server(&ssh_config, ssh_pool, ssh_repos_dir, &ssh_host)
                    .await
            {
                tracing::error!("SSH server error: {}", e);
            }
        });
    }

    // Register with AGNOS Daimon agent runtime if enabled
    if config.agnos.enabled {
        let agnos_config = config.agnos.clone();
        tokio::spawn(async move {
            match delta_core::agnos::register_with_daimon(&agnos_config, env!("CARGO_PKG_VERSION")).await {
                Ok(()) => tracing::info!("registered with daimon agent runtime"),
                Err(e) => tracing::warn!("daimon registration failed (non-fatal): {}", e),
            }
        });
    }

    axum::serve(listener, app).await?;

    Ok(())
}
