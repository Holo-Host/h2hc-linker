//! Service for running hc-membrane

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::agent_proxy::AgentProxyManager;
use crate::conductor::{AdminConn, AppConn};
use crate::config::Configuration;
use crate::error::{HcMembraneError, HcMembraneResult};
use crate::gateway_kitsune::{GatewayKitsune, KitsuneProxy, KitsuneProxyBuilder};
use crate::router::create_router;
use crate::routes::kitsune::KitsuneState;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    /// Configuration
    pub configuration: Configuration,
    /// Agent proxy manager for WebSocket connections
    pub agent_proxy: AgentProxyManager,
    /// Gateway kitsune manager (if kitsune2 enabled)
    pub gateway_kitsune: Option<GatewayKitsune>,
    /// Kitsune state for liveness endpoints
    pub kitsune_state: Arc<KitsuneState>,
    /// App connection for conductor zome calls (if conductor enabled)
    pub app_conn: Option<AppConn>,
}

/// The main hc-membrane service
pub struct HcMembraneService {
    addr: SocketAddr,
    app_state: AppState,
}

impl HcMembraneService {
    /// Create a new service with the given configuration
    pub async fn new(address: IpAddr, port: u16, config: Configuration) -> HcMembraneResult<Self> {
        let addr = SocketAddr::new(address, port);

        // Create agent proxy manager
        let agent_proxy = AgentProxyManager::new();

        // Create Kitsune2 instance if configured
        let (kitsune, gateway_kitsune) = if config.kitsune_enabled() {
            tracing::info!("Initializing Kitsune2 instance with agent registration support");

            // Create KitsuneProxy handler (supports agent registration)
            let kitsune_proxy = KitsuneProxy::new(agent_proxy.clone());

            let mut builder = KitsuneProxyBuilder::new(kitsune_proxy);

            if let Some(ref bootstrap_url) = config.bootstrap_url {
                builder = builder.with_bootstrap_url(bootstrap_url);
            }
            if let Some(ref signal_url) = config.signal_url {
                builder = builder.with_signal_url(signal_url);
            }

            match builder.build().await {
                Ok(k) => {
                    tracing::info!("Kitsune2 instance created successfully");
                    let gw_kitsune = GatewayKitsune::new(k.clone(), agent_proxy.clone());
                    (Some(k), Some(gw_kitsune))
                }
                Err(e) => {
                    tracing::error!("Failed to create Kitsune2 instance: {}", e);
                    return Err(HcMembraneError::Internal(format!(
                        "Failed to create Kitsune2 instance: {e}"
                    )));
                }
            }
        } else {
            tracing::info!("Kitsune2 not configured (no bootstrap/signal URLs)");
            (None, None)
        };

        let kitsune_state = Arc::new(KitsuneState {
            enabled: config.kitsune_enabled(),
            bootstrap_url: config.bootstrap_url.clone(),
            signal_url: config.signal_url.clone(),
            kitsune,
        });

        // Create conductor connection if configured
        let app_conn = if let Some(admin_addr) = config.admin_socket_addr {
            tracing::info!("Initializing conductor connection to {}", admin_addr);
            let admin_conn = AdminConn::new(admin_addr);
            Some(AppConn::new(admin_conn, admin_addr, config.zome_call_timeout))
        } else {
            tracing::info!("Conductor not configured (no admin WebSocket URL)");
            None
        };

        let app_state = AppState {
            configuration: config,
            agent_proxy,
            gateway_kitsune,
            kitsune_state,
            app_conn,
        };

        Ok(Self { addr, app_state })
    }

    /// Run the service
    pub async fn run(self) -> HcMembraneResult<()> {
        let router = create_router(self.app_state);

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
