//! Service for running hc-membrane

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::config::Configuration;
use crate::error::HcMembraneResult;
use crate::router::create_router;
use crate::routes::kitsune::KitsuneState;

/// The main hc-membrane service
pub struct HcMembraneService {
    addr: SocketAddr,
    kitsune_state: Arc<KitsuneState>,
}

impl HcMembraneService {
    /// Create a new service with the given configuration
    pub async fn new(
        address: IpAddr,
        port: u16,
        config: Configuration,
    ) -> HcMembraneResult<Self> {
        let addr = SocketAddr::new(address, port);

        // Create Kitsune state (placeholder for now - DynKitsune will be wired up later)
        let kitsune_state = Arc::new(KitsuneState {
            enabled: config.kitsune_enabled(),
            bootstrap_url: config.bootstrap_url.clone(),
            signal_url: config.signal_url.clone(),
            kitsune: None, // TODO: Wire up to actual Kitsune2 instance
        });

        Ok(Self {
            addr,
            kitsune_state,
        })
    }

    /// Run the service
    pub async fn run(self) -> HcMembraneResult<()> {
        let router = create_router(self.kitsune_state);

        tracing::info!("Starting hc-membrane on {}", self.addr);

        let listener = TcpListener::bind(self.addr)
            .await
            .map_err(|e| crate::error::HcMembraneError::Network(e.to_string()))?;

        axum::serve(listener, router)
            .await
            .map_err(|e| crate::error::HcMembraneError::Network(e.to_string()))?;

        Ok(())
    }
}
