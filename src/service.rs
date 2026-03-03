//! Service for running h2hc-linker

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::agent_proxy::AgentProxyManager;
use crate::auth::AuthStore;
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
    /// DHT query handler for direct kitsune2 queries
    pub dht_query: Option<DhtQuery>,
    /// Auth store (if auth enabled via H2HC_LINKER_ADMIN_SECRET)
    pub auth_store: Option<AuthStore>,
}

/// The main h2hc-linker service
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
        let pending_dht_responses = PendingDhtResponses::new();

        // Create Kitsune2 instance (always enabled - bootstrap_url is required)
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

            let mut builder =
                KitsuneProxyBuilder::new(kitsune_proxy).with_op_store(op_store_factory.into_dyn());

            builder = builder.with_bootstrap_url(&config.bootstrap_url);
            if let Some(ref relay_url) = config.relay_url {
                builder = builder.with_relay_url(relay_url);
            }

            // Configure reporting if enabled
            if let crate::config::ReportConfig::JsonLines {
                days_retained,
                fetched_op_interval_s,
            } = &config.report
            {
                tracing::info!(
                    days_retained,
                    fetched_op_interval_s,
                    path = %config.report_path.display(),
                    "Enabling kitsune2 reporting (JsonLines)"
                );
                let report_factory = crate::linker_report::LinkerReportFactory::create();
                let report_config = crate::linker_report::HcReportConfig {
                    days_retained: *days_retained,
                    path: config.report_path.clone(),
                    fetched_op_interval_s: *fetched_op_interval_s,
                };
                builder = builder.with_report(report_factory, report_config);
            }

            match builder.build().await {
                Ok(k) => {
                    tracing::info!("Kitsune2 instance created successfully");
                    let gw_kitsune = GatewayKitsune::new(k.clone(), agent_proxy.clone());
                    // Create DhtQuery with the same pending responses
                    let dht_query =
                        DhtQuery::new(gw_kitsune.clone(), pending_dht_responses.clone());
                    (
                        Some(k),
                        Some(gw_kitsune),
                        Some(op_store_handle),
                        Some(dht_query),
                    )
                }
                Err(e) => {
                    tracing::error!("Failed to create Kitsune2 instance: {}", e);
                    return Err(HcMembraneError::Internal(format!(
                        "Failed to create Kitsune2 instance: {e}"
                    )));
                }
            }
        } else {
            unreachable!("bootstrap_url is required; from_env() enforces this")
        };

        let kitsune_state = Arc::new(KitsuneState {
            enabled: true,
            bootstrap_url: Some(config.bootstrap_url.clone()),
            relay_url: config.relay_url.clone(),
            kitsune,
        });

        // Create conductor connection if configured
        let app_conn = if let Some(admin_addr) = config.admin_socket_addr {
            tracing::info!("Initializing conductor connection to {}", admin_addr);
            let admin_conn = AdminConn::new(admin_addr);
            Some(AppConn::new(
                admin_conn,
                admin_addr,
                config.zome_call_timeout,
            ))
        } else {
            tracing::info!("Conductor not configured (no admin WebSocket URL)");
            None
        };

        // Create auth store if auth is enabled
        let auth_store = if config.auth_enabled() {
            tracing::info!("Authentication enabled (H2HC_LINKER_ADMIN_SECRET set)");
            let store = AuthStore::new(config.session_ttl);
            store.start_cleanup_task();
            Some(store)
        } else {
            tracing::info!("Authentication disabled (no H2HC_LINKER_ADMIN_SECRET)");
            None
        };

        let app_state = AppState {
            configuration: config,
            agent_proxy,
            gateway_kitsune,
            kitsune_state,
            app_conn,
            temp_op_store,
            dht_query,
            auth_store,
        };

        Ok(Self { addr, app_state })
    }

    /// Run the service
    pub async fn run(self) -> HcMembraneResult<()> {
        let router = create_router(self.app_state);

        tracing::info!("Starting h2hc-linker on {}", self.addr);

        let listener = TcpListener::bind(self.addr)
            .await
            .map_err(|e| crate::error::HcMembraneError::Network(e.to_string()))?;

        axum::serve(listener, router)
            .await
            .map_err(|e| crate::error::HcMembraneError::Network(e.to_string()))?;

        Ok(())
    }
}
