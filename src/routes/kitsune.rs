//! Kitsune direct API endpoints for network status and liveness
//!
//! These endpoints provide introspection into the Kitsune2 network layer,
//! needed for the Fishy extension's liveness UI (Step 14).
//!
//! Reuses types from kitsune2_api where available:
//! - `AgentInfo` / `AgentInfoSigned` for peer information
//! - `TransportStats` / `TransportConnectionStats` for transport metrics
//! - `DhtArc` for storage arc representation

use axum::{
    extract::{Path, State},
    response::Json,
    routing::get,
    Router,
};
use kitsune2_api::{AgentId, AgentInfo, DhtArc, SpaceId, TransportStats};
use serde::Serialize;
use std::sync::Arc;

use crate::error::HcMembraneResult;

/// State for Kitsune routes - will hold actual Kitsune2 instance
#[derive(Clone)]
pub struct KitsuneState {
    /// Whether Kitsune2 is enabled and connected
    pub enabled: bool,
    /// Bootstrap server URL if configured
    pub bootstrap_url: Option<String>,
    /// Signal server URL if configured
    pub signal_url: Option<String>,
    // TODO: Add DynKitsune when wiring up to actual instance
}

impl Default for KitsuneState {
    fn default() -> Self {
        Self {
            enabled: false,
            bootstrap_url: None,
            signal_url: None,
        }
    }
}

/// Network status response
#[derive(Serialize)]
pub struct NetworkStatus {
    /// Whether Kitsune2 is enabled and connected
    pub connected: bool,
    /// Bootstrap server URL if configured
    pub bootstrap_url: Option<String>,
    /// Signal server URL if configured
    pub signal_url: Option<String>,
    /// Number of known peers across all spaces
    pub total_peers: usize,
    /// Number of active spaces (DNAs)
    pub active_spaces: usize,
}

/// Peer info for API responses - serializable wrapper around AgentInfo
#[derive(Serialize)]
pub struct PeerInfoResponse {
    /// Agent ID (serializes to base64)
    pub agent_id: AgentId,
    /// Space ID (serializes to base64)
    pub space_id: SpaceId,
    /// Created at timestamp (micros)
    pub created_at: i64,
    /// Expires at timestamp (micros)
    pub expires_at: i64,
    /// Whether this is a tombstone (agent went offline)
    pub is_tombstone: bool,
    /// URL to reach this agent
    pub url: Option<String>,
    /// Storage arc representation
    pub storage_arc: StorageArcResponse,
}

/// Storage arc for API responses
#[derive(Serialize)]
pub struct StorageArcResponse {
    /// Arc type: "empty", "full", or "arc"
    pub arc_type: String,
    /// Start location (for "arc" type)
    pub start: Option<u32>,
    /// Length (for "arc" type)
    pub length: Option<u32>,
}

impl From<DhtArc> for StorageArcResponse {
    fn from(arc: DhtArc) -> Self {
        match arc {
            DhtArc::Empty => StorageArcResponse {
                arc_type: "empty".to_string(),
                start: None,
                length: None,
            },
            DhtArc::Arc(start, end) => {
                // Check if it's a full arc (DhtArc::FULL = Arc(0, u32::MAX))
                if start == 0 && end == u32::MAX {
                    StorageArcResponse {
                        arc_type: "full".to_string(),
                        start: None,
                        length: None,
                    }
                } else {
                    StorageArcResponse {
                        arc_type: "arc".to_string(),
                        start: Some(start),
                        length: Some(end),
                    }
                }
            }
        }
    }
}

impl From<&AgentInfo> for PeerInfoResponse {
    fn from(info: &AgentInfo) -> Self {
        PeerInfoResponse {
            agent_id: info.agent.clone(),
            space_id: info.space.clone(),
            created_at: info.created_at.as_micros(),
            expires_at: info.expires_at.as_micros(),
            is_tombstone: info.is_tombstone,
            url: info.url.as_ref().map(|u| u.to_string()),
            storage_arc: info.storage_arc.into(),
        }
    }
}

/// Space status response
#[derive(Serialize)]
pub struct SpaceStatusResponse {
    /// Space ID (DNA hash as base64)
    pub space_id: String,
    /// Number of local agents in this space
    pub local_agents: usize,
    /// Number of known peers in this space
    pub peer_count: usize,
}

/// Create Kitsune routes
pub fn kitsune_routes() -> Router<Arc<KitsuneState>> {
    Router::new()
        .route("/status", get(get_network_status))
        .route("/peers", get(get_all_peers))
        .route("/space/{space_id}/status", get(get_space_status))
        .route("/space/{space_id}/peers", get(get_space_peers))
        .route("/space/{space_id}/local-agents", get(get_local_agents))
        .route("/transport/stats", get(get_transport_stats))
}

/// GET /k2/status - Get overall network status
async fn get_network_status(
    State(state): State<Arc<KitsuneState>>,
) -> HcMembraneResult<Json<NetworkStatus>> {
    // TODO: Get actual status from Kitsune2 instance
    Ok(Json(NetworkStatus {
        connected: state.enabled,
        bootstrap_url: state.bootstrap_url.clone(),
        signal_url: state.signal_url.clone(),
        total_peers: 0,
        active_spaces: 0,
    }))
}

/// GET /k2/peers - Get all known peers across all spaces
async fn get_all_peers(
    State(_state): State<Arc<KitsuneState>>,
) -> HcMembraneResult<Json<Vec<PeerInfoResponse>>> {
    // TODO: Aggregate peers from all spaces using:
    // kitsune.space(space_id).await?.peer_store().get_all().await?
    Ok(Json(vec![]))
}

/// GET /k2/space/{space_id}/status - Get status for a specific space (DNA)
async fn get_space_status(
    State(_state): State<Arc<KitsuneState>>,
    Path(space_id): Path<String>,
) -> HcMembraneResult<Json<SpaceStatusResponse>> {
    // TODO: Get actual space status using:
    // let space = kitsune.space(space_id.parse()?).await?;
    // let peers = space.peer_store().get_all().await?;
    // let local = space.local_agent_store().get_all().await?;
    Ok(Json(SpaceStatusResponse {
        space_id,
        local_agents: 0,
        peer_count: 0,
    }))
}

/// GET /k2/space/{space_id}/peers - Get peers for a specific space
async fn get_space_peers(
    State(_state): State<Arc<KitsuneState>>,
    Path(_space_id): Path<String>,
) -> HcMembraneResult<Json<Vec<PeerInfoResponse>>> {
    // TODO: Get peers from space's peer store using:
    // let space = kitsune.space(space_id.parse()?).await?;
    // let peers = space.peer_store().get_all().await?;
    // peers.iter().map(|p| PeerInfoResponse::from(p.as_ref())).collect()
    Ok(Json(vec![]))
}

/// GET /k2/space/{space_id}/local-agents - Get local agents for a space
async fn get_local_agents(
    State(_state): State<Arc<KitsuneState>>,
    Path(_space_id): Path<String>,
) -> HcMembraneResult<Json<Vec<String>>> {
    // TODO: Get local agents from space's local agent store using:
    // let space = kitsune.space(space_id.parse()?).await?;
    // let agents = space.local_agent_store().get_all().await?;
    // agents.iter().map(|a| a.agent().to_string()).collect()
    Ok(Json(vec![]))
}

/// GET /k2/transport/stats - Get transport statistics
/// Returns the kitsune2_api::TransportStats directly
async fn get_transport_stats(
    State(_state): State<Arc<KitsuneState>>,
) -> HcMembraneResult<Json<TransportStats>> {
    // TODO: Get actual transport stats using:
    // kitsune.transport().dump_network_stats().await?
    // TransportStats is already Serialize from kitsune2_api
    Ok(Json(TransportStats {
        backend: "tx5".to_string(),
        peer_urls: vec![],
        connections: vec![],
    }))
}
