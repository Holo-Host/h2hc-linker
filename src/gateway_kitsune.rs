//! Kitsune2 proxy for browser extension agents
//!
//! This module allows the gateway to participate in kitsune2
//! on behalf of zero-arc browser agents whose private keys live
//! in the browser extension.
//!
//! # Architecture
//!
//! The gateway runs its own kitsune2 instance that:
//! 1. Joins spaces (DNAs) on behalf of browser agents
//! 2. Receives `RemoteSignalEvt` messages via `recv_notify`
//! 3. Forwards signals to browser via WebSocket
//!
//! # Signal Flow
//!
//! ```text
//! Conductor Agent A ──send_remote_signal──► kitsune2 network
//!                                                │
//!                                                ▼
//! Gateway ◄── recv_notify (RemoteSignalEvt) ◄────┘
//!    │
//!    └── decode WireMessage
//!    └── forward to AgentProxyManager
//!    └── WebSocket to browser
//! ```

use crate::agent_proxy::AgentProxyManager;
use crate::proxy_agent::ProxyAgent;
use crate::wire_preflight::WirePreflightMessage;
use bytes::Bytes;
use holochain_types::prelude::{AgentPubKey, DnaHash};
use kitsune2_api::{
    AgentId, BoxFut, DynKitsune, DynLocalAgent, DynSpace, DynSpaceHandler, K2Error, K2Result,
    KitsuneHandler, SpaceHandler, SpaceId, Url,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Top-level kitsune2 handler for the gateway.
///
/// This implements `KitsuneHandler` and creates `ProxySpaceHandler`
/// instances for each space (DNA) that has registered browser agents.
#[derive(Debug)]
pub struct KitsuneProxy {
    agent_proxy: AgentProxyManager,
}

impl KitsuneProxy {
    /// Create a new KitsuneProxy with the given agent proxy manager.
    pub fn new(agent_proxy: AgentProxyManager) -> Self {
        Self { agent_proxy }
    }
}

impl KitsuneHandler for KitsuneProxy {
    fn create_space(&self, space_id: SpaceId) -> BoxFut<'_, K2Result<DynSpaceHandler>> {
        let agent_proxy = self.agent_proxy.clone();
        Box::pin(async move {
            info!(?space_id, "Creating proxy space handler");
            let handler: DynSpaceHandler = Arc::new(ProxySpaceHandler {
                space_id,
                agent_proxy,
            });
            Ok(handler)
        })
    }

    fn new_listening_address(&self, this_url: Url) -> BoxFut<'static, ()> {
        info!(%this_url, "Gateway kitsune2 listening on new address");
        Box::pin(async move {})
    }

    fn peer_disconnect(&self, peer: Url, reason: Option<String>) {
        debug!(%peer, ?reason, "Peer disconnected from gateway");
    }

    fn preflight_gather_outgoing(&self, peer_url: Url) -> BoxFut<'_, K2Result<Bytes>> {
        Box::pin(async move {
            // Create preflight message with matching protocol version
            let preflight = WirePreflightMessage::new();

            info!(
                %peer_url,
                proto_ver = preflight.compat.proto_ver,
                "Sending preflight to peer"
            );

            preflight
                .encode()
                .map_err(|e| K2Error::other(format!("Failed to encode preflight: {e}")))
        })
    }

    fn preflight_validate_incoming(&self, peer_url: Url, data: Bytes) -> BoxFut<'_, K2Result<()>> {
        Box::pin(async move {
            // Decode and validate the incoming preflight
            let preflight = WirePreflightMessage::decode(&data)
                .map_err(|e| K2Error::other(format!("Invalid preflight from peer: {e}")))?;

            // Check protocol version compatibility
            if preflight.compat.proto_ver != 2 {
                return Err(K2Error::other(format!(
                    "Incompatible protocol version from {}: expected 2, got {}",
                    peer_url, preflight.compat.proto_ver
                )));
            }

            info!(
                %peer_url,
                proto_ver = preflight.compat.proto_ver,
                agent_count = preflight.agents.len(),
                "Validated incoming preflight"
            );

            Ok(())
        })
    }
}

/// Per-space handler that receives notifications from kitsune2.
///
/// When a `RemoteSignalEvt` is received, it decodes the wire message
/// and forwards the signal to the appropriate browser agent via WebSocket.
#[derive(Debug)]
struct ProxySpaceHandler {
    space_id: SpaceId,
    #[allow(dead_code)] // Will be used in M2b for signal forwarding
    agent_proxy: AgentProxyManager,
}

impl SpaceHandler for ProxySpaceHandler {
    fn recv_notify(&self, from_peer: Url, space_id: SpaceId, data: Bytes) -> K2Result<()> {
        debug!(
            %from_peer,
            ?space_id,
            data_len = data.len(),
            "Received notification in proxy space (signal forwarding not yet implemented)"
        );

        // Signal forwarding will be implemented in M2b
        // For now, just log and acknowledge

        Ok(())
    }
}

/// Builder for creating a gateway kitsune2 instance.
///
/// # Example
///
/// ```ignore
/// let proxy = KitsuneProxy::new(agent_proxy_manager);
/// let kitsune = KitsuneProxyBuilder::new(proxy)
///     .with_bootstrap_url("https://bootstrap.example.com")
///     .with_signal_url("wss://signal.example.com")
///     .build()
///     .await?;
/// ```
pub struct KitsuneProxyBuilder {
    handler: Arc<KitsuneProxy>,
    bootstrap_url: Option<String>,
    signal_url: Option<String>,
}

impl KitsuneProxyBuilder {
    /// Create a new builder with the given handler.
    pub fn new(handler: KitsuneProxy) -> Self {
        Self {
            handler: Arc::new(handler),
            bootstrap_url: None,
            signal_url: None,
        }
    }

    /// Set the bootstrap server URL.
    pub fn with_bootstrap_url(mut self, url: impl Into<String>) -> Self {
        self.bootstrap_url = Some(url.into());
        self
    }

    /// Set the signal server URL (for tx5 transport).
    pub fn with_signal_url(mut self, url: impl Into<String>) -> Self {
        self.signal_url = Some(url.into());
        self
    }

    /// Build the kitsune2 instance.
    pub async fn build(self) -> Result<DynKitsune, Box<dyn std::error::Error + Send + Sync>> {
        use kitsune2::default_builder;
        use kitsune2_core::factories::config::{CoreBootstrapConfig, CoreBootstrapModConfig};
        use kitsune2_transport_tx5::config::{Tx5TransportConfig, Tx5TransportModConfig};

        let builder = default_builder().with_default_config()?;

        // Configure bootstrap server
        if let Some(bootstrap_url) = self.bootstrap_url {
            builder.config.set_module_config(&CoreBootstrapModConfig {
                core_bootstrap: CoreBootstrapConfig {
                    server_url: bootstrap_url,
                    ..Default::default()
                },
            })?;
        }

        // Configure signal server with STUN servers for WebRTC
        if let Some(signal_url) = self.signal_url {
            use kitsune2_transport_tx5::{IceServers, WebRtcConfig};

            builder.config.set_module_config(&Tx5TransportModConfig {
                tx5_transport: Tx5TransportConfig {
                    server_url: signal_url,
                    signal_allow_plain_text: true, // TODO: configure for production
                    webrtc_config: WebRtcConfig {
                        ice_servers: vec![IceServers {
                            urls: vec![
                                "stun:stun.l.google.com:19302".to_string(),
                                "stun:stun1.l.google.com:19302".to_string(),
                            ],
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            })?;
        }

        // Build and register handler
        let kitsune = builder.build().await?;
        kitsune.register_handler(self.handler).await?;

        info!("Gateway kitsune2 instance created");
        Ok(kitsune)
    }
}

/// Gateway kitsune2 manager that handles space and agent lifecycle.
///
/// This wraps a `DynKitsune` instance and provides methods to:
/// - Join browser agents to spaces when they register
/// - Leave agents from spaces when they disconnect
/// - Track active spaces and agents
///
/// # Example
///
/// ```ignore
/// let gateway_kitsune = GatewayKitsune::new(kitsune, agent_proxy);
///
/// // When browser agent registers via WebSocket
/// gateway_kitsune.agent_join(&dna_hash, &agent_pubkey).await?;
///
/// // When browser agent disconnects
/// gateway_kitsune.agent_leave(&dna_hash, &agent_pubkey).await;
/// ```
#[derive(Clone)]
pub struct GatewayKitsune {
    kitsune: DynKitsune,
    /// Agent proxy manager for remote signing.
    agent_proxy: AgentProxyManager,
    /// Active spaces by DNA hash.
    spaces: Arc<RwLock<HashMap<DnaHash, DynSpace>>>,
    /// Registered agents by (DnaHash, AgentPubKey).
    /// Value is the ProxyAgent for potential future use.
    agents: Arc<RwLock<HashMap<(DnaHash, AgentPubKey), Arc<ProxyAgent>>>>,
}

impl std::fmt::Debug for GatewayKitsune {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayKitsune").finish_non_exhaustive()
    }
}

impl GatewayKitsune {
    /// Create a new gateway kitsune manager.
    ///
    /// The `agent_proxy` is used for remote signing when kitsune2 needs
    /// to sign agent info on behalf of browser agents.
    pub fn new(kitsune: DynKitsune, agent_proxy: AgentProxyManager) -> Self {
        Self {
            kitsune,
            agent_proxy,
            spaces: Arc::new(RwLock::new(HashMap::new())),
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create a space for a DNA.
    async fn get_or_create_space(&self, dna_hash: &DnaHash) -> Result<DynSpace, String> {
        // Check if space exists
        {
            let spaces = self.spaces.read().await;
            if let Some(space) = spaces.get(dna_hash) {
                return Ok(space.clone());
            }
        }

        // Create new space using built-in conversion
        let space_id = dna_hash.to_k2_space();

        let space = self
            .kitsune
            .space(space_id)
            .await
            .map_err(|e| format!("Failed to create space: {e}"))?;

        // Store space
        {
            let mut spaces = self.spaces.write().await;
            spaces.insert(dna_hash.clone(), space.clone());
        }

        info!(dna = %dna_hash, "Created kitsune2 space for DNA");
        Ok(space)
    }

    /// Join a browser agent to a DNA's kitsune2 space.
    ///
    /// This creates a ProxyAgent and registers it with the space,
    /// enabling the gateway to receive signals on behalf of this agent.
    ///
    /// # Arguments
    ///
    /// * `dna_hash` - The DNA hash (proper Holochain type)
    /// * `agent_pubkey` - The agent public key (proper Holochain type)
    pub async fn agent_join(
        &self,
        dna_hash: &DnaHash,
        agent_pubkey: &AgentPubKey,
    ) -> Result<(), String> {
        let key = (dna_hash.clone(), agent_pubkey.clone());

        // Check if already registered
        {
            let agents = self.agents.read().await;
            if agents.contains_key(&key) {
                debug!(
                    dna = %dna_hash,
                    agent = %agent_pubkey,
                    "Agent already joined to space"
                );
                return Ok(());
            }
        }

        // Get or create space
        let space = self.get_or_create_space(dna_hash).await?;

        // Create proxy agent with access to agent_proxy for remote signing
        let proxy_agent = Arc::new(ProxyAgent::new(
            agent_pubkey.clone(),
            self.agent_proxy.clone(),
        ));

        // Join space
        space
            .local_agent_join(proxy_agent.clone() as DynLocalAgent)
            .await
            .map_err(|e| format!("Failed to join agent to space: {e}"))?;

        // Store agent
        {
            let mut agents = self.agents.write().await;
            agents.insert(key, proxy_agent);
        }

        info!(
            dna = %dna_hash,
            agent = %agent_pubkey,
            "Browser agent joined kitsune2 space"
        );
        Ok(())
    }

    /// Remove a browser agent from a DNA's kitsune2 space.
    ///
    /// This publishes a tombstone agent info to the network and removes
    /// the agent from the local tracking.
    ///
    /// # Arguments
    ///
    /// * `dna_hash` - The DNA hash (proper Holochain type)
    /// * `agent_pubkey` - The agent public key (proper Holochain type)
    pub async fn agent_leave(&self, dna_hash: &DnaHash, agent_pubkey: &AgentPubKey) {
        let key = (dna_hash.clone(), agent_pubkey.clone());

        // Remove from tracking
        let proxy_agent = {
            let mut agents = self.agents.write().await;
            agents.remove(&key)
        };

        if proxy_agent.is_none() {
            debug!(
                dna = %dna_hash,
                agent = %agent_pubkey,
                "Agent not found in space (already left?)"
            );
            return;
        }

        // Get space (if it exists)
        let space = {
            let spaces = self.spaces.read().await;
            spaces.get(dna_hash).cloned()
        };

        if let Some(space) = space {
            // Create AgentId from 32-byte key
            let agent_id = AgentId::from(Bytes::copy_from_slice(agent_pubkey.get_raw_32()));

            // Leave space (publishes tombstone)
            space.local_agent_leave(agent_id).await;

            info!(
                dna = %dna_hash,
                agent = %agent_pubkey,
                "Browser agent left kitsune2 space"
            );
        }

        // Check if space has no more agents, and clean up if so
        self.maybe_cleanup_space(dna_hash).await;
    }

    /// Remove a space if it has no more registered agents.
    async fn maybe_cleanup_space(&self, dna_hash: &DnaHash) {
        let has_agents = {
            let agents = self.agents.read().await;
            agents.keys().any(|(dna, _)| dna == dna_hash)
        };

        if !has_agents {
            let mut spaces = self.spaces.write().await;
            if spaces.remove(dna_hash).is_some() {
                info!(dna = %dna_hash, "Removed empty kitsune2 space");
            }
        }
    }

    /// Leave all agents from all spaces.
    ///
    /// Called during gateway shutdown to publish tombstones for all agents.
    pub async fn shutdown(&self) {
        let agents: Vec<_> = {
            let agents = self.agents.read().await;
            agents.keys().cloned().collect()
        };

        for (dna_hash, agent_pubkey) in agents {
            self.agent_leave(&dna_hash, &agent_pubkey).await;
        }

        info!("Gateway kitsune2 shutdown complete");
    }

    /// Get the number of registered agents.
    pub async fn agent_count(&self) -> usize {
        self.agents.read().await.len()
    }

    /// Get the number of active spaces.
    pub async fn space_count(&self) -> usize {
        self.spaces.read().await.len()
    }

    /// Check if an agent is registered in a space.
    ///
    /// # Arguments
    ///
    /// * `dna_hash` - The DNA hash (proper Holochain type)
    /// * `agent_pubkey` - The agent public key (proper Holochain type)
    pub async fn is_agent_joined(&self, dna_hash: &DnaHash, agent_pubkey: &AgentPubKey) -> bool {
        let key = (dna_hash.clone(), agent_pubkey.clone());
        self.agents.read().await.contains_key(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a test DNA hash (uses from_raw_32 for valid checksums)
    fn test_dna(id: u8) -> DnaHash {
        DnaHash::from_raw_32(vec![id; 32])
    }

    // Helper to create a test agent (uses from_raw_32 for valid checksums)
    fn test_agent(id: u8) -> AgentPubKey {
        AgentPubKey::from_raw_32(vec![id; 32])
    }

    #[test]
    fn test_kitsune_proxy_creation() {
        let agent_proxy = AgentProxyManager::new();
        let proxy = KitsuneProxy::new(agent_proxy);
        assert!(format!("{:?}", proxy).contains("KitsuneProxy"));
    }

    #[test]
    fn test_space_handler_creation() {
        let agent_proxy = AgentProxyManager::new();
        let dna = test_dna(1);
        let handler = ProxySpaceHandler {
            space_id: dna.to_k2_space(),
            agent_proxy,
        };
        assert!(format!("{:?}", handler).contains("ProxySpaceHandler"));
    }
}
