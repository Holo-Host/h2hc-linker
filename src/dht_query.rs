//! Direct DHT queries via kitsune2 wire protocol.
//!
//! This module provides direct DHT queries (get, get_links) by sending
//! wire protocol messages to DHT authorities, bypassing the conductor.
//!
//! # Architecture
//!
//! ```text
//! DhtQuery
//!    │
//!    ├── Finds peers near hash location via peer_store
//!    ├── Sends WireMessage::GetReq/GetLinksReq via space.send_notify()
//!    │
//!    ▼
//! Remote Authority
//!    │
//!    ├── Receives request, queries local DHT
//!    ├── Sends WireMessage::GetRes/GetLinksRes back
//!    │
//!    ▼
//! ProxySpaceHandler.recv_notify()
//!    │
//!    └── Routes response via shared PendingDhtResponses
//! ```

use holo_hash::AnyDhtHash;
use holochain_p2p::WireMessage;
use holochain_types::dht_op::WireOps;
use holochain_types::link::{WireLinkKey, WireLinkOps};
use holochain_types::prelude::{AgentPubKey, DnaHash};
use kitsune2_api::{DynSpace, Url};
use kitsune2_core::get_responsive_remote_agents_near_location;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, RwLock};
use tracing::{debug, info, warn};

use crate::error::{HcMembraneError, HcMembraneResult};
use crate::gateway_kitsune::GatewayKitsune;

/// Number of peers to query in parallel (same as holochain_p2p)
const PARALLEL_GET_AGENTS_COUNT: usize = 3;

/// Default timeout for DHT queries
const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Pending request tracker
type PendingResponder = oneshot::Sender<WireMessage>;

/// Shared storage for pending DHT query responses.
///
/// This is shared between DhtQuery (which registers pending requests)
/// and ProxySpaceHandler (which routes incoming responses).
#[derive(Clone, Default)]
pub struct PendingDhtResponses {
    inner: Arc<RwLock<HashMap<u64, PendingResponder>>>,
}

impl std::fmt::Debug for PendingDhtResponses {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingDhtResponses").finish()
    }
}

impl PendingDhtResponses {
    /// Create a new empty pending responses tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a pending request.
    pub async fn register(&self, msg_id: u64, responder: PendingResponder) {
        let mut guard = self.inner.write().await;
        debug!(msg_id, pending_count = guard.len(), "Registering pending DHT request");
        guard.insert(msg_id, responder);
    }

    /// Remove a pending request (for cleanup on timeout).
    pub async fn remove(&self, msg_id: u64) {
        let mut guard = self.inner.write().await;
        let existed = guard.remove(&msg_id).is_some();
        debug!(msg_id, existed, pending_count = guard.len(), "Removed pending DHT request (timeout cleanup)");
    }

    /// Route a response to its pending request.
    ///
    /// Returns true if the response was routed, false if no pending request was found.
    pub async fn route_response(&self, msg: WireMessage) -> bool {
        let msg_id = match msg.get_msg_id() {
            Some(id) => id,
            None => {
                warn!("Cannot route response without msg_id");
                return false;
            }
        };

        let (responder, pending_count) = {
            let mut guard = self.inner.write().await;
            let r = guard.remove(&msg_id);
            (r, guard.len())
        };

        match responder {
            Some(tx) => {
                debug!(msg_id, pending_count, "Routing DHT response to pending request");
                if tx.send(msg).is_err() {
                    debug!(msg_id, "Pending request receiver dropped (caller cancelled)");
                }
                true
            }
            None => {
                debug!(msg_id, pending_count, "No pending request for msg_id (possibly timed out or wrong instance)");
                false
            }
        }
    }
}

/// DHT query handler for direct wire protocol queries.
///
/// This allows the gateway to query the DHT directly via kitsune2,
/// without going through the conductor.
#[derive(Clone)]
pub struct DhtQuery {
    /// Gateway kitsune manager for space access
    gateway_kitsune: GatewayKitsune,
    /// Shared pending requests (also used by ProxySpaceHandler for response routing)
    pending: PendingDhtResponses,
    /// Query timeout
    timeout: Duration,
}

impl std::fmt::Debug for DhtQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DhtQuery")
            .field("pending", &self.pending)
            .finish()
    }
}

impl DhtQuery {
    /// Create a new DhtQuery handler with shared pending responses.
    ///
    /// The `pending` should be the same instance passed to KitsuneProxy
    /// so that response routing works correctly.
    pub fn new(gateway_kitsune: GatewayKitsune, pending: PendingDhtResponses) -> Self {
        Self {
            gateway_kitsune,
            pending,
            timeout: DEFAULT_QUERY_TIMEOUT,
        }
    }

    /// Create a new DhtQuery handler with custom timeout.
    pub fn with_timeout(
        gateway_kitsune: GatewayKitsune,
        pending: PendingDhtResponses,
        timeout: Duration,
    ) -> Self {
        Self {
            gateway_kitsune,
            pending,
            timeout,
        }
    }

    /// Get a record from the DHT by hash.
    ///
    /// Queries multiple peers near the hash location in parallel and returns
    /// the first non-empty response.
    pub async fn get(
        &self,
        dna_hash: &DnaHash,
        hash: AnyDhtHash,
    ) -> HcMembraneResult<Option<WireOps>> {
        let space = self.gateway_kitsune.get_or_create_space(dna_hash).await
            .map_err(|e| HcMembraneError::Internal(e))?;

        let loc = hash.get_loc();
        let agents = self.get_peers_for_location(&space, loc).await?;

        if agents.is_empty() {
            info!(dna = %dna_hash, loc, "No peers found for DHT location");
            return Ok(None);
        }

        debug!(
            dna = %dna_hash,
            hash = %hash,
            peer_count = agents.len(),
            "Querying peers for get"
        );

        // Query peers in parallel, return first non-empty response
        let mut handles = Vec::new();
        for (agent, url) in agents.into_iter().take(PARALLEL_GET_AGENTS_COUNT) {
            let space = space.clone();
            let hash = hash.clone();
            let pending = self.pending.clone();
            let timeout = self.timeout;

            handles.push(tokio::spawn(async move {
                Self::send_get_request(space, agent, url, hash, pending, timeout).await
            }));
        }

        // Await all and return first non-empty success
        let mut last_result = None;
        for handle in handles {
            match handle.await {
                Ok(Ok(Some(wire_ops))) if !is_wire_ops_empty(&wire_ops) => {
                    return Ok(Some(wire_ops));
                }
                Ok(Ok(result)) => {
                    last_result = Some(Ok(result));
                }
                Ok(Err(e)) => {
                    debug!(?e, "Peer query failed");
                    if last_result.is_none() {
                        last_result = Some(Err(e));
                    }
                }
                Err(e) => {
                    debug!(?e, "Task join error");
                }
            }
        }

        last_result.unwrap_or(Ok(None))
    }

    /// Get links from the DHT by base hash.
    ///
    /// Queries multiple peers near the base hash location in parallel and returns
    /// the first non-empty response.
    pub async fn get_links(
        &self,
        dna_hash: &DnaHash,
        link_key: WireLinkKey,
    ) -> HcMembraneResult<Option<WireLinkOps>> {
        let space = self.gateway_kitsune.get_or_create_space(dna_hash).await
            .map_err(|e| HcMembraneError::Internal(e))?;

        let loc = link_key.base.get_loc();
        let agents = self.get_peers_for_location(&space, loc).await?;

        if agents.is_empty() {
            info!(dna = %dna_hash, loc, "No peers found for DHT location");
            return Ok(None);
        }

        debug!(
            dna = %dna_hash,
            base = %link_key.base,
            peer_count = agents.len(),
            "Querying peers for get_links"
        );

        // Query peers in parallel, return first non-empty response
        let mut handles = Vec::new();
        for (agent, url) in agents.into_iter().take(PARALLEL_GET_AGENTS_COUNT) {
            let space = space.clone();
            let link_key = link_key.clone();
            let pending = self.pending.clone();
            let timeout = self.timeout;

            handles.push(tokio::spawn(async move {
                Self::send_get_links_request(space, agent, url, link_key, pending, timeout).await
            }));
        }

        // Await all and return first non-empty success
        let mut last_result = None;
        for handle in handles {
            match handle.await {
                Ok(Ok(Some(wire_link_ops))) if !wire_link_ops.creates.is_empty() => {
                    return Ok(Some(wire_link_ops));
                }
                Ok(Ok(result)) => {
                    last_result = Some(Ok(result));
                }
                Ok(Err(e)) => {
                    debug!(?e, "Peer query failed");
                    if last_result.is_none() {
                        last_result = Some(Err(e));
                    }
                }
                Err(e) => {
                    debug!(?e, "Task join error");
                }
            }
        }

        last_result.unwrap_or(Ok(None))
    }

    /// Find peers near a given DHT location.
    async fn get_peers_for_location(
        &self,
        space: &DynSpace,
        loc: u32,
    ) -> HcMembraneResult<Vec<(AgentPubKey, Url)>> {
        let agents = get_responsive_remote_agents_near_location(
            space.peer_store().clone(),
            space.local_agent_store().clone(),
            space.peer_meta_store().clone(),
            loc,
            1024,
        )
        .await
        .map_err(|e| HcMembraneError::Internal(format!("Failed to get peers: {e}")))?;

        Ok(agents
            .into_iter()
            .filter_map(|a| {
                if a.url.is_none() || a.is_tombstone {
                    return None;
                }
                if !a.storage_arc.contains(loc) {
                    return None;
                }
                Some((
                    AgentPubKey::from_k2_agent(&a.agent),
                    a.url.as_ref().unwrap().clone(),
                ))
            })
            .collect())
    }

    /// Send a get request to a specific peer.
    async fn send_get_request(
        space: DynSpace,
        to_agent: AgentPubKey,
        to_url: Url,
        hash: AnyDhtHash,
        pending: PendingDhtResponses,
        timeout: Duration,
    ) -> HcMembraneResult<Option<WireOps>> {
        let (msg_id, req) = WireMessage::get_req(to_agent.clone(), hash);

        let encoded = WireMessage::encode_batch(&[&req])
            .map_err(|e| HcMembraneError::Internal(format!("Failed to encode request: {e}")))?;

        // Register pending request
        let (tx, rx) = oneshot::channel();
        pending.register(msg_id, tx).await;

        // Set up timeout to clean up pending request
        let pending_cleanup = pending.clone();
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            pending_cleanup.remove(msg_id).await;
        });

        // Send request
        space
            .send_notify(to_url.clone(), encoded)
            .await
            .map_err(|e| HcMembraneError::Internal(format!("Failed to send request: {e}")))?;

        debug!(msg_id, %to_url, %to_agent, "Sent get request");

        // Wait for response
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(WireMessage::GetRes { response, .. })) => {
                debug!(msg_id, "Got response");
                Ok(Some(response))
            }
            Ok(Ok(WireMessage::ErrorRes { error, .. })) => {
                Err(HcMembraneError::Internal(format!("Remote error: {error}")))
            }
            Ok(Ok(other)) => Err(HcMembraneError::Internal(format!(
                "Unexpected response: {other:?}"
            ))),
            Ok(Err(_)) => Err(HcMembraneError::Internal(
                "Response channel closed".to_string(),
            )),
            Err(_) => Err(HcMembraneError::Internal("Request timed out".to_string())),
        }
    }

    /// Send a get_links request to a specific peer.
    async fn send_get_links_request(
        space: DynSpace,
        to_agent: AgentPubKey,
        to_url: Url,
        link_key: WireLinkKey,
        pending: PendingDhtResponses,
        timeout: Duration,
    ) -> HcMembraneResult<Option<WireLinkOps>> {
        use holochain_p2p::event::GetLinksOptions;

        let (msg_id, req) =
            WireMessage::get_links_req(to_agent.clone(), link_key, GetLinksOptions {});

        let encoded = WireMessage::encode_batch(&[&req])
            .map_err(|e| HcMembraneError::Internal(format!("Failed to encode request: {e}")))?;

        // Register pending request
        let (tx, rx) = oneshot::channel();
        pending.register(msg_id, tx).await;

        // Set up timeout to clean up pending request
        let pending_cleanup = pending.clone();
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            pending_cleanup.remove(msg_id).await;
        });

        // Send request
        space.send_notify(to_url.clone(), encoded).await
            .map_err(|e| HcMembraneError::Internal(format!("Failed to send request: {e}")))?;

        debug!(msg_id, %to_url, %to_agent, "Sent get_links request");

        // Wait for response
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(WireMessage::GetLinksRes { response, .. })) => {
                debug!(
                    msg_id,
                    creates_count = response.creates.len(),
                    deletes_count = response.deletes.len(),
                    "Got get_links response"
                );
                Ok(Some(response))
            }
            Ok(Ok(WireMessage::ErrorRes { error, .. })) => {
                Err(HcMembraneError::Internal(format!("Remote error: {error}")))
            }
            Ok(Ok(other)) => {
                Err(HcMembraneError::Internal(format!("Unexpected response: {other:?}")))
            }
            Ok(Err(_)) => {
                Err(HcMembraneError::Internal("Response channel closed".to_string()))
            }
            Err(_) => {
                Err(HcMembraneError::Internal("Request timed out".to_string()))
            }
        }
    }
}

/// Check if WireOps is empty (no data found)
fn is_wire_ops_empty(ops: &WireOps) -> bool {
    match ops {
        WireOps::Entry(entry_ops) => {
            entry_ops.creates.is_empty()
                && entry_ops.deletes.is_empty()
                && entry_ops.updates.is_empty()
                && entry_ops.entry.is_none()
        }
        WireOps::Record(record_ops) => {
            record_ops.action.is_none()
                && record_ops.deletes.is_empty()
                && record_ops.updates.is_empty()
                && record_ops.entry.is_none()
        }
        WireOps::Warrant(_) => false, // Warrants are never "empty"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use holochain_types::entry::WireEntryOps;
    use holochain_types::record::WireRecordOps;

    #[test]
    fn test_pending_dht_responses() {
        // Test that PendingDhtResponses can be created
        let pending = PendingDhtResponses::new();
        assert!(format!("{:?}", pending).contains("PendingDhtResponses"));
    }

    #[test]
    fn test_is_wire_ops_empty() {
        // Empty entry ops
        let empty_entry = WireOps::Entry(WireEntryOps {
            creates: vec![],
            deletes: vec![],
            updates: vec![],
            entry: None,
        });
        assert!(is_wire_ops_empty(&empty_entry));

        // Empty record ops
        let empty_record = WireOps::Record(WireRecordOps {
            action: None,
            deletes: vec![],
            updates: vec![],
            entry: None,
        });
        assert!(is_wire_ops_empty(&empty_record));
    }
}
