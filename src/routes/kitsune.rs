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
use base64::prelude::*;
use kitsune2_api::{AgentId, AgentInfo, ApiTransportStats, DhtArc, DynKitsune, SpaceId};
use serde::Serialize;
use std::sync::Arc;

use crate::error::{HcMembraneError, HcMembraneResult};

/// Parse a base64-encoded space ID from a URL path parameter
fn parse_space_id(space_id_str: &str) -> HcMembraneResult<SpaceId> {
    let bytes = BASE64_URL_SAFE_NO_PAD
        .decode(space_id_str)
        .map_err(|e| HcMembraneError::InvalidRequest(format!("Invalid space ID: {}", e)))?;
    Ok(SpaceId::from(bytes::Bytes::from(bytes)))
}

/// State for Kitsune routes - holds the Kitsune2 instance
#[derive(Clone)]
pub struct KitsuneState {
    /// Whether Kitsune2 is enabled and connected
    pub enabled: bool,
    /// Bootstrap server URL if configured
    pub bootstrap_url: Option<String>,
    /// Signal server URL if configured
    pub signal_url: Option<String>,
    /// The Kitsune2 instance (None if not yet connected)
    pub kitsune: Option<DynKitsune>,
}

impl Default for KitsuneState {
    fn default() -> Self {
        Self {
            enabled: false,
            bootstrap_url: None,
            signal_url: None,
            kitsune: None,
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
    let (total_peers, active_spaces) = if let Some(kitsune) = &state.kitsune {
        let spaces = kitsune.list_spaces();
        let active_spaces = spaces.len();

        // Aggregate peer count across all spaces
        let mut total_peers = 0;
        for space_id in spaces {
            if let Some(space) = kitsune.space_if_exists(space_id).await {
                if let Ok(peers) = space.peer_store().get_all().await {
                    total_peers += peers.len();
                }
            }
        }
        (total_peers, active_spaces)
    } else {
        (0, 0)
    };

    Ok(Json(NetworkStatus {
        connected: state.enabled && state.kitsune.is_some(),
        bootstrap_url: state.bootstrap_url.clone(),
        signal_url: state.signal_url.clone(),
        total_peers,
        active_spaces,
    }))
}

/// GET /k2/peers - Get all known peers across all spaces
async fn get_all_peers(
    State(state): State<Arc<KitsuneState>>,
) -> HcMembraneResult<Json<Vec<PeerInfoResponse>>> {
    let Some(kitsune) = &state.kitsune else {
        return Ok(Json(vec![]));
    };

    let mut all_peers = Vec::new();
    for space_id in kitsune.list_spaces() {
        if let Some(space) = kitsune.space_if_exists(space_id).await {
            if let Ok(peers) = space.peer_store().get_all().await {
                for peer in peers {
                    // peer is Arc<AgentInfoSigned>, get_agent_info() returns &AgentInfo
                    all_peers.push(PeerInfoResponse::from(peer.get_agent_info()));
                }
            }
        }
    }

    Ok(Json(all_peers))
}

/// GET /k2/space/{space_id}/status - Get status for a specific space (DNA)
async fn get_space_status(
    State(state): State<Arc<KitsuneState>>,
    Path(space_id_str): Path<String>,
) -> HcMembraneResult<Json<SpaceStatusResponse>> {
    let space_id = parse_space_id(&space_id_str)?;

    let Some(kitsune) = &state.kitsune else {
        return Ok(Json(SpaceStatusResponse {
            space_id: space_id_str,
            local_agents: 0,
            peer_count: 0,
        }));
    };

    let Some(space) = kitsune.space_if_exists(space_id).await else {
        return Err(HcMembraneError::NotFound(format!(
            "Space not found: {}",
            space_id_str
        )));
    };

    let peer_count = space
        .peer_store()
        .get_all()
        .await
        .map(|p| p.len())
        .unwrap_or(0);

    let local_agents = space
        .local_agent_store()
        .get_all()
        .await
        .map(|a| a.len())
        .unwrap_or(0);

    Ok(Json(SpaceStatusResponse {
        space_id: space_id_str,
        local_agents,
        peer_count,
    }))
}

/// GET /k2/space/{space_id}/peers - Get peers for a specific space
async fn get_space_peers(
    State(state): State<Arc<KitsuneState>>,
    Path(space_id_str): Path<String>,
) -> HcMembraneResult<Json<Vec<PeerInfoResponse>>> {
    let space_id = parse_space_id(&space_id_str)?;

    let Some(kitsune) = &state.kitsune else {
        return Ok(Json(vec![]));
    };

    let Some(space) = kitsune.space_if_exists(space_id).await else {
        return Err(HcMembraneError::NotFound(format!(
            "Space not found: {}",
            space_id_str
        )));
    };

    let peers = space
        .peer_store()
        .get_all()
        .await
        .map_err(|e| HcMembraneError::Internal(e.to_string()))?;

    let responses: Vec<PeerInfoResponse> = peers
        .iter()
        .map(|p| PeerInfoResponse::from(p.get_agent_info()))
        .collect();

    Ok(Json(responses))
}

/// GET /k2/space/{space_id}/local-agents - Get local agents for a space
async fn get_local_agents(
    State(state): State<Arc<KitsuneState>>,
    Path(space_id_str): Path<String>,
) -> HcMembraneResult<Json<Vec<AgentId>>> {
    let space_id = parse_space_id(&space_id_str)?;

    let Some(kitsune) = &state.kitsune else {
        return Ok(Json(vec![]));
    };

    let Some(space) = kitsune.space_if_exists(space_id).await else {
        return Err(HcMembraneError::NotFound(format!(
            "Space not found: {}",
            space_id_str
        )));
    };

    let agents = space
        .local_agent_store()
        .get_all()
        .await
        .map_err(|e| HcMembraneError::Internal(e.to_string()))?;

    let agent_ids: Vec<AgentId> = agents.iter().map(|a| a.agent().clone()).collect();

    Ok(Json(agent_ids))
}

/// GET /k2/transport/stats - Get transport statistics
/// Returns the kitsune2_api::ApiTransportStats directly
async fn get_transport_stats(
    State(state): State<Arc<KitsuneState>>,
) -> HcMembraneResult<Json<ApiTransportStats>> {
    let Some(kitsune) = &state.kitsune else {
        // Return empty stats if Kitsune not connected
        return Ok(Json(ApiTransportStats {
            transport_stats: kitsune2_api::TransportStats {
                backend: "tx5".to_string(),
                peer_urls: vec![],
                connections: vec![],
            },
            blocked_message_counts: std::collections::HashMap::new(),
        }));
    };

    let transport = kitsune
        .transport()
        .await
        .map_err(|e| HcMembraneError::Internal(e.to_string()))?;

    let stats = transport
        .dump_network_stats()
        .await
        .map_err(|e| HcMembraneError::Internal(e.to_string()))?;

    Ok(Json(stats))
}
