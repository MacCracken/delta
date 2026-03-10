use clap::Parser;
use delta_api::{routes, state::AppState};
use delta_core::DeltaConfig;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "delta", about = "Delta — code hosting, CI/CD, and artifact registry")]
struct Cli {
    /// Config file path
    #[arg(short, long, default_value = "/etc/delta/config.toml")]
    config: String,

    /// Override listen port
    #[arg(short, long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("delta=info".parse()?))
        .init();

    let cli = Cli::parse();

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

    let state = AppState::new(config.clone());
    let app = routes::router(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("delta listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
