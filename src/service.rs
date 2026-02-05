//! Service for running hc-membrane

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::agent_proxy::AgentProxyManager;
use crate::conductor::{AdminConn, AppConn};
use crate::config::Configuration;
use crate::dht_query::{DhtQuery, PendingDhtResponses};
use crate::error::{HcMembraneError, HcMembraneResult};
use crate::gateway_kitsune::{GatewayKitsune, KitsuneProxy, KitsuneProxyBuilder};
use crate::router::create_router;
use crate::routes::kitsune::KitsuneState;
use crate::temp_op_store::{TempOpStoreFactory, TempOpStoreHandle};

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
    /// Temp op store for publishing (if kitsune2 enabled)
    pub temp_op_store: Option<TempOpStoreHandle>,
    /// DHT query handler for direct kitsune2 queries (if kitsune2 enabled)
    #[cfg(not(feature = "conductor-dht"))]
    pub dht_query: Option<DhtQuery>,
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

        // Create shared pending DHT responses for DhtQuery <-> ProxySpaceHandler communication
        #[cfg(not(feature = "conductor-dht"))]
        let pending_dht_responses = PendingDhtResponses::new();

        // Create Kitsune2 instance if configured
        #[cfg(not(feature = "conductor-dht"))]
        let (kitsune, gateway_kitsune, temp_op_store, dht_query) = if config.kitsune_enabled() {
            tracing::info!("Initializing Kitsune2 instance with agent registration support and direct DHT queries");

            // Create TempOpStore for publishing
            let (op_store_factory, op_store_handle) = TempOpStoreFactory::create();
            op_store_factory.start_cleanup_task();

            // Create KitsuneProxy handler with shared pending responses
            let kitsune_proxy = KitsuneProxy::with_pending_responses(
                agent_proxy.clone(),
                pending_dht_responses.clone(),
            );

            let mut builder = KitsuneProxyBuilder::new(kitsune_proxy)
                .with_op_store(op_store_factory.into_dyn());

            if let Some(ref bootstrap_url) = config.bootstrap_url {
                builder = builder.with_bootstrap_url(bootstrap_url);
            }
            if let Some(ref relay_url) = config.relay_url {
                builder = builder.with_relay_url(relay_url);
            }

            match builder.build().await {
                Ok(k) => {
                    tracing::info!("Kitsune2 instance created successfully");
                    let gw_kitsune = GatewayKitsune::new(k.clone(), agent_proxy.clone());
                    // Create DhtQuery with the same pending responses
                    let dht_query = DhtQuery::new(gw_kitsune.clone(), pending_dht_responses.clone());
                    (Some(k), Some(gw_kitsune), Some(op_store_handle), Some(dht_query))
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
            (None, None, None, None)
        };

        // Create Kitsune2 instance if configured (conductor-dht feature: use conductor for DHT queries)
        #[cfg(feature = "conductor-dht")]
        let (kitsune, gateway_kitsune, temp_op_store) = if config.kitsune_enabled() {
            tracing::info!("Initializing Kitsune2 instance with agent registration support (conductor-dht mode)");

            // Create TempOpStore for publishing
            let (op_store_factory, op_store_handle) = TempOpStoreFactory::create();
            op_store_factory.start_cleanup_task();

            // Create KitsuneProxy handler (no shared pending responses needed in conductor-dht mode)
            let kitsune_proxy = KitsuneProxy::new(agent_proxy.clone());

            let mut builder = KitsuneProxyBuilder::new(kitsune_proxy)
                .with_op_store(op_store_factory.into_dyn());

            if let Some(ref bootstrap_url) = config.bootstrap_url {
                builder = builder.with_bootstrap_url(bootstrap_url);
            }
            if let Some(ref relay_url) = config.relay_url {
                builder = builder.with_relay_url(relay_url);
            }

            match builder.build().await {
                Ok(k) => {
                    tracing::info!("Kitsune2 instance created successfully");
                    let gw_kitsune = GatewayKitsune::new(k.clone(), agent_proxy.clone());
                    (Some(k), Some(gw_kitsune), Some(op_store_handle))
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
            (None, None, None)
        };

        let kitsune_state = Arc::new(KitsuneState {
            enabled: config.kitsune_enabled(),
            bootstrap_url: config.bootstrap_url.clone(),
            relay_url: config.relay_url.clone(),
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
            temp_op_store,
            #[cfg(not(feature = "conductor-dht"))]
            dht_query,
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
