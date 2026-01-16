//! hc-membrane binary entry point

use clap::Parser;
use hc_membrane::{Configuration, HcMembraneService};
use std::net::IpAddr;
use tracing_subscriber::{
    fmt::{self, format::FmtSpan, time::UtcTime},
    layer::SubscriberExt,
    EnvFilter, Registry,
};

const DEFAULT_LOG_LEVEL: &str = "info";

/// Command line arguments for hc-membrane
#[derive(clap::Parser, Debug)]
#[command(name = "hc-membrane")]
#[command(about = "Holochain Membrane - Network edge gateway for lightweight clients")]
pub struct Args {
    /// The address to bind to
    #[arg(short, long, env = "HC_MEMBRANE_ADDRESS", default_value = "127.0.0.1")]
    pub address: IpAddr,

    /// The port to bind to
    #[arg(short, long, env = "HC_MEMBRANE_PORT", default_value = "8090")]
    pub port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Install the default rustls crypto provider
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    initialize_tracing()?;

    let args = Args::parse();
    let config = Configuration::from_env()?;

    tracing::info!(
        address = %args.address,
        port = %args.port,
        kitsune_enabled = config.kitsune_enabled(),
        "Starting hc-membrane"
    );

    let service = HcMembraneService::new(args.address, args.port, config).await?;
    service.run().await?;

    Ok(())
}

fn initialize_tracing() -> Result<(), tracing::subscriber::SetGlobalDefaultError> {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_LEVEL));

    let formatting_layer = fmt::layer()
        .with_timer(UtcTime::rfc_3339())
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_file(true)
        .with_line_number(true);

    let subscriber = Registry::default().with(env_filter).with(formatting_layer);

    tracing::subscriber::set_global_default(subscriber)
}
