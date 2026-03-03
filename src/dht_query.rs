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
use holochain_p2p::event::GetActivityOptions;
use holochain_p2p::WireMessage;
use holochain_types::activity::AgentActivityResponse;
use holochain_types::chain::MustGetAgentActivityResponse;
use holochain_types::dht_op::WireOps;
use holochain_types::link::{CountLinksResponse, WireLinkKey, WireLinkOps, WireLinkQuery};
use holochain_types::prelude::{AgentPubKey, DnaHash};
use holochain_zome_types::chain::ChainFilter;
use holochain_zome_types::query::{ChainQueryFilter, ChainStatus};
use kitsune2_api::{DynSpace, Url};
use kitsune2_core::get_responsive_remote_agents_near_location;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, RwLock};
use tracing::{debug, info, warn};

use crate::error::{LinkerError, LinkerResult};
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
        debug!(
            msg_id,
            pending_count = guard.len(),
            "Registering pending DHT request"
        );
        guard.insert(msg_id, responder);
    }

    /// Remove a pending request (for cleanup on timeout).
    pub async fn remove(&self, msg_id: u64) {
        let mut guard = self.inner.write().await;
        let existed = guard.remove(&msg_id).is_some();
        debug!(
            msg_id,
            existed,
            pending_count = guard.len(),
            "Removed pending DHT request (timeout cleanup)"
        );
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

        info!(msg_id, "Attempting to route response to pending request");

        let (responder, pending_count, pending_ids) = {
            let mut guard = self.inner.write().await;
            let pending_ids: Vec<u64> = guard.keys().copied().collect();
            let r = guard.remove(&msg_id);
            (r, guard.len(), pending_ids)
        };

        info!(
            msg_id,
            pending_count,
            ?pending_ids,
            "Pending requests state before routing"
        );

        match responder {
            Some(tx) => {
                info!(
                    msg_id,
                    "Found matching pending request, sending response..."
                );
                if tx.send(msg).is_err() {
                    warn!(
                        msg_id,
                        "Pending request receiver dropped (caller cancelled)"
                    );
                }
                true
            }
            None => {
                warn!(
                    msg_id,
                    pending_count,
                    ?pending_ids,
                    "No pending request for msg_id - response dropped (possibly timed out or wrong instance)"
                );
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
    pub async fn get(&self, dna_hash: &DnaHash, hash: AnyDhtHash) -> LinkerResult<Option<WireOps>> {
        let space = self
            .gateway_kitsune
            .get_or_create_space(dna_hash)
            .await
            .map_err(LinkerError::Internal)?;

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
    ) -> LinkerResult<Option<WireLinkOps>> {
        let space = self
            .gateway_kitsune
            .get_or_create_space(dna_hash)
            .await
            .map_err(LinkerError::Internal)?;

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

    /// Count links from the DHT by base hash.
    ///
    /// Sends CountLinksReq to peers near the base hash location and returns
    /// the first response.
    pub async fn count_links(
        &self,
        dna_hash: &DnaHash,
        query: WireLinkQuery,
    ) -> LinkerResult<Option<CountLinksResponse>> {
        let space = self
            .gateway_kitsune
            .get_or_create_space(dna_hash)
            .await
            .map_err(LinkerError::Internal)?;

        let loc = query.base.get_loc();
        let agents = self.get_peers_for_location(&space, loc).await?;

        if agents.is_empty() {
            info!(dna = %dna_hash, loc, "No peers found for DHT location");
            return Ok(None);
        }

        debug!(
            dna = %dna_hash,
            base = %query.base,
            peer_count = agents.len(),
            "Querying peers for count_links"
        );

        // Query peers in parallel, return first response
        let mut handles = Vec::new();
        for (agent, url) in agents.into_iter().take(PARALLEL_GET_AGENTS_COUNT) {
            let space = space.clone();
            let query = query.clone();
            let pending = self.pending.clone();
            let timeout = self.timeout;

            handles.push(tokio::spawn(async move {
                Self::send_count_links_request(space, agent, url, query, pending, timeout).await
            }));
        }

        // Await all and return first success
        let mut last_result = None;
        for handle in handles {
            match handle.await {
                Ok(Ok(Some(response))) => {
                    return Ok(Some(response));
                }
                Ok(Ok(result)) => {
                    last_result = Some(Ok(result));
                }
                Ok(Err(e)) => {
                    debug!(?e, "Peer count_links query failed");
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
    ) -> LinkerResult<Vec<(AgentPubKey, Url)>> {
        // First log what we have in peer store
        match space.peer_store().get_all().await {
            Ok(all_peers) => {
                info!(
                    peer_count = all_peers.len(),
                    loc, "Peer store contents for DHT query"
                );
                for peer in &all_peers {
                    debug!(
                        agent = ?peer.agent,
                        url = ?peer.url,
                        arc = ?peer.storage_arc,
                        is_tombstone = peer.is_tombstone,
                        contains_loc = peer.storage_arc.contains(loc),
                        "Peer in store"
                    );
                }
            }
            Err(e) => {
                warn!(?e, "Failed to get all peers for debug");
            }
        }

        // Also log local agents
        match space.local_agent_store().get_all().await {
            Ok(local_agents) => {
                info!(
                    local_agent_count = local_agents.len(),
                    "Local agents in space"
                );
            }
            Err(e) => {
                warn!(?e, "Failed to get local agents for debug");
            }
        }

        let agents = get_responsive_remote_agents_near_location(
            space.peer_store().clone(),
            space.local_agent_store().clone(),
            space.peer_meta_store().clone(),
            loc,
            1024,
        )
        .await
        .map_err(|e| LinkerError::Internal(format!("Failed to get peers: {e}")))?;

        info!(
            responsive_count = agents.len(),
            loc, "Found responsive remote agents"
        );

        Ok(agents
            .into_iter()
            .filter_map(|a| {
                if a.url.is_none() || a.is_tombstone {
                    debug!(agent = ?a.agent, "Skipping agent (no url or tombstone)");
                    return None;
                }
                if !a.storage_arc.contains(loc) {
                    debug!(agent = ?a.agent, arc = ?a.storage_arc, loc, "Skipping agent (arc doesn't contain loc)");
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
    ) -> LinkerResult<Option<WireOps>> {
        let (msg_id, req) = WireMessage::get_req(to_agent.clone(), hash.clone());

        info!(
            msg_id,
            %to_url,
            %to_agent,
            %hash,
            "Preparing GetReq message"
        );

        let encoded = WireMessage::encode_batch(&[&req])
            .map_err(|e| LinkerError::Internal(format!("Failed to encode request: {e}")))?;

        info!(
            msg_id,
            encoded_len = encoded.len(),
            "Encoded GetReq message"
        );

        // Register pending request
        let (tx, rx) = oneshot::channel();
        pending.register(msg_id, tx).await;

        info!(
            msg_id,
            "Registered pending request, sending via send_notify..."
        );

        // Set up timeout to clean up pending request
        let pending_cleanup = pending.clone();
        let timeout_msg_id = msg_id;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            warn!(
                msg_id = timeout_msg_id,
                "Timeout cleanup triggered for pending request"
            );
            pending_cleanup.remove(timeout_msg_id).await;
        });

        // Send request - this is the key step
        let send_start = std::time::Instant::now();
        match space.send_notify(to_url.clone(), encoded).await {
            Ok(()) => {
                info!(
                    msg_id,
                    %to_url,
                    %to_agent,
                    send_elapsed_ms = send_start.elapsed().as_millis(),
                    "send_notify completed successfully, waiting for response..."
                );
            }
            Err(e) => {
                warn!(
                    msg_id,
                    %to_url,
                    %to_agent,
                    error = %e,
                    "send_notify FAILED"
                );
                return Err(LinkerError::Internal(format!(
                    "Failed to send request: {e}"
                )));
            }
        }

        // Wait for response
        info!(
            msg_id,
            timeout_secs = timeout.as_secs(),
            "Waiting for response..."
        );
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(WireMessage::GetRes { response, .. })) => {
                info!(msg_id, "Got GetRes response");
                Ok(Some(response))
            }
            Ok(Ok(WireMessage::ErrorRes { error, .. })) => {
                warn!(msg_id, %error, "Got ErrorRes response");
                Err(LinkerError::Internal(format!("Remote error: {error}")))
            }
            Ok(Ok(other)) => {
                warn!(msg_id, ?other, "Got unexpected response type");
                Err(LinkerError::Internal(format!(
                    "Unexpected response: {other:?}"
                )))
            }
            Ok(Err(_)) => {
                warn!(msg_id, "Response channel closed (receiver dropped)");
                Err(LinkerError::Internal("Response channel closed".to_string()))
            }
            Err(_) => {
                warn!(
                    msg_id,
                    timeout_secs = timeout.as_secs(),
                    "Request TIMED OUT - no response received"
                );
                Err(LinkerError::Internal("Request timed out".to_string()))
            }
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
    ) -> LinkerResult<Option<WireLinkOps>> {
        use holochain_p2p::event::GetLinksOptions;

        let (msg_id, req) =
            WireMessage::get_links_req(to_agent.clone(), link_key.clone(), GetLinksOptions {});

        info!(
            msg_id,
            %to_url,
            %to_agent,
            base = %link_key.base,
            "Preparing GetLinksReq message"
        );

        let encoded = WireMessage::encode_batch(&[&req])
            .map_err(|e| LinkerError::Internal(format!("Failed to encode request: {e}")))?;

        info!(
            msg_id,
            encoded_len = encoded.len(),
            "Encoded GetLinksReq message"
        );

        // Register pending request
        let (tx, rx) = oneshot::channel();
        pending.register(msg_id, tx).await;

        info!(
            msg_id,
            "Registered pending request, sending via send_notify..."
        );

        // Set up timeout to clean up pending request
        let pending_cleanup = pending.clone();
        let timeout_msg_id = msg_id;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            warn!(
                msg_id = timeout_msg_id,
                "Timeout cleanup triggered for pending get_links request"
            );
            pending_cleanup.remove(timeout_msg_id).await;
        });

        // Send request - this is the key step
        let send_start = std::time::Instant::now();
        match space.send_notify(to_url.clone(), encoded).await {
            Ok(()) => {
                info!(
                    msg_id,
                    %to_url,
                    %to_agent,
                    send_elapsed_ms = send_start.elapsed().as_millis(),
                    "send_notify completed successfully, waiting for response..."
                );
            }
            Err(e) => {
                warn!(
                    msg_id,
                    %to_url,
                    %to_agent,
                    error = %e,
                    "send_notify FAILED"
                );
                return Err(LinkerError::Internal(format!(
                    "Failed to send request: {e}"
                )));
            }
        }

        // Wait for response
        info!(
            msg_id,
            timeout_secs = timeout.as_secs(),
            "Waiting for get_links response..."
        );
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(WireMessage::GetLinksRes { response, .. })) => {
                info!(
                    msg_id,
                    creates_count = response.creates.len(),
                    deletes_count = response.deletes.len(),
                    "Got GetLinksRes response"
                );
                Ok(Some(response))
            }
            Ok(Ok(WireMessage::ErrorRes { error, .. })) => {
                warn!(msg_id, %error, "Got ErrorRes response");
                Err(LinkerError::Internal(format!("Remote error: {error}")))
            }
            Ok(Ok(other)) => {
                warn!(msg_id, ?other, "Got unexpected response type");
                Err(LinkerError::Internal(format!(
                    "Unexpected response: {other:?}"
                )))
            }
            Ok(Err(_)) => {
                warn!(msg_id, "Response channel closed (receiver dropped)");
                Err(LinkerError::Internal("Response channel closed".to_string()))
            }
            Err(_) => {
                warn!(
                    msg_id,
                    timeout_secs = timeout.as_secs(),
                    "GetLinks request TIMED OUT - no response received"
                );
                Err(LinkerError::Internal("Request timed out".to_string()))
            }
        }
    }

    /// Get agent activity from the DHT.
    ///
    /// Queries multiple peers near the agent's DHT location in parallel and returns
    /// the first non-empty response.
    pub async fn get_agent_activity(
        &self,
        dna_hash: &DnaHash,
        agent: AgentPubKey,
        query: ChainQueryFilter,
        options: GetActivityOptions,
    ) -> LinkerResult<Option<AgentActivityResponse>> {
        let space = self
            .gateway_kitsune
            .get_or_create_space(dna_hash)
            .await
            .map_err(LinkerError::Internal)?;

        let loc = agent.get_loc();
        let agents = self.get_peers_for_location(&space, loc).await?;

        if agents.is_empty() {
            info!(dna = %dna_hash, loc, "No peers found for agent activity query");
            return Ok(None);
        }

        debug!(
            dna = %dna_hash,
            %agent,
            peer_count = agents.len(),
            "Querying peers for get_agent_activity"
        );

        let mut handles = Vec::new();
        for (to_agent, url) in agents.into_iter().take(PARALLEL_GET_AGENTS_COUNT) {
            let space = space.clone();
            let agent = agent.clone();
            let query = query.clone();
            let options = options.clone();
            let pending = self.pending.clone();
            let timeout = self.timeout;

            handles.push(tokio::spawn(async move {
                Self::send_get_agent_activity_request(
                    space, to_agent, url, agent, query, options, pending, timeout,
                )
                .await
            }));
        }

        let mut last_result = None;
        for handle in handles {
            match handle.await {
                Ok(Ok(Some(response))) if response.status != ChainStatus::Empty => {
                    return Ok(Some(response));
                }
                Ok(Ok(result)) => {
                    last_result = Some(Ok(result));
                }
                Ok(Err(e)) => {
                    debug!(?e, "Peer agent activity query failed");
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

    /// Get agent activity with must-get semantics from the DHT.
    ///
    /// Queries multiple peers near the agent's DHT location in parallel and returns
    /// the first Activity response.
    pub async fn must_get_agent_activity(
        &self,
        dna_hash: &DnaHash,
        agent: AgentPubKey,
        filter: ChainFilter,
    ) -> LinkerResult<Option<MustGetAgentActivityResponse>> {
        let space = self
            .gateway_kitsune
            .get_or_create_space(dna_hash)
            .await
            .map_err(LinkerError::Internal)?;

        let loc = agent.get_loc();
        let agents = self.get_peers_for_location(&space, loc).await?;

        if agents.is_empty() {
            info!(dna = %dna_hash, loc, "No peers found for must_get_agent_activity query");
            return Ok(None);
        }

        debug!(
            dna = %dna_hash,
            %agent,
            peer_count = agents.len(),
            "Querying peers for must_get_agent_activity"
        );

        let mut handles = Vec::new();
        for (to_agent, url) in agents.into_iter().take(PARALLEL_GET_AGENTS_COUNT) {
            let space = space.clone();
            let agent = agent.clone();
            let filter = filter.clone();
            let pending = self.pending.clone();
            let timeout = self.timeout;

            handles.push(tokio::spawn(async move {
                Self::send_must_get_agent_activity_request(
                    space, to_agent, url, agent, filter, pending, timeout,
                )
                .await
            }));
        }

        let mut last_result = None;
        for handle in handles {
            match handle.await {
                Ok(Ok(Some(response))) => {
                    if matches!(&response, MustGetAgentActivityResponse::Activity { .. }) {
                        return Ok(Some(response));
                    }
                    last_result = Some(Ok(Some(response)));
                }
                Ok(Ok(result)) => {
                    last_result = Some(Ok(result));
                }
                Ok(Err(e)) => {
                    debug!(?e, "Peer must_get_agent_activity query failed");
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

    /// Send a get_agent_activity request to a specific peer.
    async fn send_get_agent_activity_request(
        space: DynSpace,
        to_agent: AgentPubKey,
        to_url: Url,
        agent: AgentPubKey,
        query: ChainQueryFilter,
        options: GetActivityOptions,
        pending: PendingDhtResponses,
        timeout: Duration,
    ) -> LinkerResult<Option<AgentActivityResponse>> {
        let (msg_id, req) =
            WireMessage::get_agent_activity_req(to_agent.clone(), agent.clone(), query, options);

        debug!(
            msg_id,
            %to_url,
            %to_agent,
            %agent,
            "Preparing GetAgentActivityReq message"
        );

        let encoded = WireMessage::encode_batch(&[&req])
            .map_err(|e| LinkerError::Internal(format!("Failed to encode request: {e}")))?;

        let (tx, rx) = oneshot::channel();
        pending.register(msg_id, tx).await;

        let pending_cleanup = pending.clone();
        let timeout_msg_id = msg_id;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            pending_cleanup.remove(timeout_msg_id).await;
        });

        match space.send_notify(to_url.clone(), encoded).await {
            Ok(()) => {
                debug!(msg_id, %to_url, "GetAgentActivityReq send_notify completed");
            }
            Err(e) => {
                warn!(msg_id, %to_url, error = %e, "GetAgentActivityReq send_notify FAILED");
                return Err(LinkerError::Internal(format!(
                    "Failed to send request: {e}"
                )));
            }
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(WireMessage::GetAgentActivityRes { response, .. })) => {
                debug!(msg_id, "Got GetAgentActivityRes response");
                Ok(Some(response))
            }
            Ok(Ok(WireMessage::ErrorRes { error, .. })) => {
                warn!(msg_id, %error, "Got ErrorRes for get_agent_activity");
                Err(LinkerError::Internal(format!("Remote error: {error}")))
            }
            Ok(Ok(other)) => {
                warn!(
                    msg_id,
                    ?other,
                    "Got unexpected response type for get_agent_activity"
                );
                Err(LinkerError::Internal(format!(
                    "Unexpected response: {other:?}"
                )))
            }
            Ok(Err(_)) => Err(LinkerError::Internal("Response channel closed".to_string())),
            Err(_) => {
                warn!(
                    msg_id,
                    timeout_secs = timeout.as_secs(),
                    "GetAgentActivity request TIMED OUT"
                );
                Err(LinkerError::Internal("Request timed out".to_string()))
            }
        }
    }

    /// Send a must_get_agent_activity request to a specific peer.
    async fn send_must_get_agent_activity_request(
        space: DynSpace,
        to_agent: AgentPubKey,
        to_url: Url,
        agent: AgentPubKey,
        filter: ChainFilter,
        pending: PendingDhtResponses,
        timeout: Duration,
    ) -> LinkerResult<Option<MustGetAgentActivityResponse>> {
        let (msg_id, req) =
            WireMessage::must_get_agent_activity_req(to_agent.clone(), agent.clone(), filter);

        debug!(
            msg_id,
            %to_url,
            %to_agent,
            %agent,
            "Preparing MustGetAgentActivityReq message"
        );

        let encoded = WireMessage::encode_batch(&[&req])
            .map_err(|e| LinkerError::Internal(format!("Failed to encode request: {e}")))?;

        let (tx, rx) = oneshot::channel();
        pending.register(msg_id, tx).await;

        let pending_cleanup = pending.clone();
        let timeout_msg_id = msg_id;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            pending_cleanup.remove(timeout_msg_id).await;
        });

        match space.send_notify(to_url.clone(), encoded).await {
            Ok(()) => {
                debug!(msg_id, %to_url, "MustGetAgentActivityReq send_notify completed");
            }
            Err(e) => {
                warn!(msg_id, %to_url, error = %e, "MustGetAgentActivityReq send_notify FAILED");
                return Err(LinkerError::Internal(format!(
                    "Failed to send request: {e}"
                )));
            }
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(WireMessage::MustGetAgentActivityRes { response, .. })) => {
                debug!(msg_id, "Got MustGetAgentActivityRes response");
                Ok(Some(response))
            }
            Ok(Ok(WireMessage::ErrorRes { error, .. })) => {
                warn!(msg_id, %error, "Got ErrorRes for must_get_agent_activity");
                Err(LinkerError::Internal(format!("Remote error: {error}")))
            }
            Ok(Ok(other)) => {
                warn!(
                    msg_id,
                    ?other,
                    "Got unexpected response type for must_get_agent_activity"
                );
                Err(LinkerError::Internal(format!(
                    "Unexpected response: {other:?}"
                )))
            }
            Ok(Err(_)) => Err(LinkerError::Internal("Response channel closed".to_string())),
            Err(_) => {
                warn!(
                    msg_id,
                    timeout_secs = timeout.as_secs(),
                    "MustGetAgentActivity request TIMED OUT"
                );
                Err(LinkerError::Internal("Request timed out".to_string()))
            }
        }
    }

    /// Send a count_links request to a specific peer.
    async fn send_count_links_request(
        space: DynSpace,
        to_agent: AgentPubKey,
        to_url: Url,
        query: WireLinkQuery,
        pending: PendingDhtResponses,
        timeout: Duration,
    ) -> LinkerResult<Option<CountLinksResponse>> {
        let (msg_id, req) = WireMessage::count_links_req(to_agent.clone(), query.clone());

        info!(
            msg_id,
            %to_url,
            %to_agent,
            base = %query.base,
            "Preparing CountLinksReq message"
        );

        let encoded = WireMessage::encode_batch(&[&req])
            .map_err(|e| LinkerError::Internal(format!("Failed to encode request: {e}")))?;

        // Register pending request
        let (tx, rx) = oneshot::channel();
        pending.register(msg_id, tx).await;

        // Set up timeout to clean up pending request
        let pending_cleanup = pending.clone();
        let timeout_msg_id = msg_id;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            pending_cleanup.remove(timeout_msg_id).await;
        });

        // Send request
        let send_start = std::time::Instant::now();
        match space.send_notify(to_url.clone(), encoded).await {
            Ok(()) => {
                info!(
                    msg_id,
                    %to_url,
                    send_elapsed_ms = send_start.elapsed().as_millis(),
                    "CountLinksReq send_notify completed"
                );
            }
            Err(e) => {
                warn!(msg_id, %to_url, error = %e, "CountLinksReq send_notify FAILED");
                return Err(LinkerError::Internal(format!(
                    "Failed to send request: {e}"
                )));
            }
        }

        // Wait for response
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(WireMessage::CountLinksRes { response, .. })) => {
                info!(msg_id, "Got CountLinksRes response");
                Ok(Some(response))
            }
            Ok(Ok(WireMessage::ErrorRes { error, .. })) => {
                warn!(msg_id, %error, "Got ErrorRes for count_links");
                Err(LinkerError::Internal(format!("Remote error: {error}")))
            }
            Ok(Ok(other)) => {
                warn!(
                    msg_id,
                    ?other,
                    "Got unexpected response type for count_links"
                );
                Err(LinkerError::Internal(format!(
                    "Unexpected response: {other:?}"
                )))
            }
            Ok(Err(_)) => {
                warn!(msg_id, "Response channel closed (receiver dropped)");
                Err(LinkerError::Internal("Response channel closed".to_string()))
            }
            Err(_) => {
                warn!(
                    msg_id,
                    timeout_secs = timeout.as_secs(),
                    "CountLinks request TIMED OUT"
                );
                Err(LinkerError::Internal("Request timed out".to_string()))
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
    use holochain_types::link::{CountLinksResponse, WireLinkOps};
    use holochain_types::record::WireRecordOps;

    #[test]
    fn test_pending_dht_responses() {
        let pending = PendingDhtResponses::new();
        assert!(format!("{pending:?}").contains("PendingDhtResponses"));
    }

    #[test]
    fn test_is_wire_ops_empty() {
        let empty_entry = WireOps::Entry(WireEntryOps {
            creates: vec![],
            deletes: vec![],
            updates: vec![],
            entry: None,
        });
        assert!(is_wire_ops_empty(&empty_entry));

        let empty_record = WireOps::Record(WireRecordOps {
            action: None,
            deletes: vec![],
            updates: vec![],
            entry: None,
        });
        assert!(is_wire_ops_empty(&empty_record));
    }

    #[tokio::test]
    async fn test_route_get_res_to_pending_request() {
        let pending = PendingDhtResponses::new();
        let (tx, rx) = oneshot::channel();

        let msg_id = 42;
        pending.register(msg_id, tx).await;

        let response = WireOps::Record(WireRecordOps {
            action: None,
            deletes: vec![],
            updates: vec![],
            entry: None,
        });
        let wire_msg = WireMessage::get_res(msg_id, response);

        let routed = pending.route_response(wire_msg).await;
        assert!(routed);

        let received = rx.await.expect("channel should deliver response");
        match received {
            WireMessage::GetRes {
                msg_id: id,
                response: _,
            } => assert_eq!(id, msg_id),
            other => panic!("Expected GetRes, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_route_get_links_res_to_pending_request() {
        let pending = PendingDhtResponses::new();
        let (tx, rx) = oneshot::channel();

        let msg_id = 99;
        pending.register(msg_id, tx).await;

        let response = WireLinkOps {
            creates: vec![],
            deletes: vec![],
        };
        let wire_msg = WireMessage::get_links_res(msg_id, response);

        let routed = pending.route_response(wire_msg).await;
        assert!(routed);

        let received = rx.await.expect("channel should deliver response");
        match received {
            WireMessage::GetLinksRes {
                msg_id: id,
                response,
            } => {
                assert_eq!(id, msg_id);
                assert!(response.creates.is_empty());
            }
            other => panic!("Expected GetLinksRes, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_route_count_links_res_to_pending_request() {
        let pending = PendingDhtResponses::new();
        let (tx, rx) = oneshot::channel();

        let msg_id = 200;
        pending.register(msg_id, tx).await;

        let response = CountLinksResponse::new(vec![]);
        let wire_msg = WireMessage::count_links_res(msg_id, response);

        let routed = pending.route_response(wire_msg).await;
        assert!(routed);

        let received = rx.await.expect("channel should deliver response");
        match received {
            WireMessage::CountLinksRes { msg_id: id, .. } => assert_eq!(id, msg_id),
            other => panic!("Expected CountLinksRes, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_route_error_res_to_pending_request() {
        let pending = PendingDhtResponses::new();
        let (tx, rx) = oneshot::channel();

        let msg_id = 55;
        pending.register(msg_id, tx).await;

        let wire_msg = WireMessage::ErrorRes {
            msg_id,
            error: "test error".to_string(),
        };

        let routed = pending.route_response(wire_msg).await;
        assert!(routed);

        let received = rx.await.expect("channel should deliver response");
        match received {
            WireMessage::ErrorRes { msg_id: id, error } => {
                assert_eq!(id, msg_id);
                assert_eq!(error, "test error");
            }
            other => panic!("Expected ErrorRes, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_route_response_no_matching_pending() {
        let pending = PendingDhtResponses::new();

        let wire_msg = WireMessage::get_res(
            999,
            WireOps::Record(WireRecordOps {
                action: None,
                deletes: vec![],
                updates: vec![],
                entry: None,
            }),
        );

        let routed = pending.route_response(wire_msg).await;
        assert!(!routed);
    }

    #[tokio::test]
    async fn test_remove_cleans_up_pending_request() {
        let pending = PendingDhtResponses::new();
        let (tx, _rx) = oneshot::channel();

        let msg_id = 77;
        pending.register(msg_id, tx).await;

        // Simulate timeout cleanup
        pending.remove(msg_id).await;

        // Now routing should fail — no pending request
        let wire_msg = WireMessage::get_res(
            msg_id,
            WireOps::Record(WireRecordOps {
                action: None,
                deletes: vec![],
                updates: vec![],
                entry: None,
            }),
        );
        let routed = pending.route_response(wire_msg).await;
        assert!(!routed);
    }

    #[tokio::test]
    async fn test_route_response_after_receiver_dropped() {
        let pending = PendingDhtResponses::new();
        let (tx, rx) = oneshot::channel();

        let msg_id = 88;
        pending.register(msg_id, tx).await;

        // Drop the receiver before the response arrives
        drop(rx);

        let wire_msg = WireMessage::get_res(
            msg_id,
            WireOps::Record(WireRecordOps {
                action: None,
                deletes: vec![],
                updates: vec![],
                entry: None,
            }),
        );

        // route_response should still return true (it found the pending entry)
        // but the send will fail silently (logged as warning)
        let routed = pending.route_response(wire_msg).await;
        assert!(routed);
    }

    #[tokio::test]
    async fn test_multiple_pending_requests_independent() {
        let pending = PendingDhtResponses::new();

        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();

        pending.register(10, tx1).await;
        pending.register(20, tx2).await;

        // Route response only to msg_id=20
        let wire_msg = WireMessage::get_links_res(
            20,
            WireLinkOps {
                creates: vec![],
                deletes: vec![],
            },
        );
        let routed = pending.route_response(wire_msg).await;
        assert!(routed);

        // msg_id=20 should have received response
        let received = rx2.await.expect("rx2 should get response");
        assert!(matches!(
            received,
            WireMessage::GetLinksRes { msg_id: 20, .. }
        ));

        // msg_id=10 should still be pending (not consumed)
        // Route its response now
        let wire_msg2 = WireMessage::get_res(
            10,
            WireOps::Record(WireRecordOps {
                action: None,
                deletes: vec![],
                updates: vec![],
                entry: None,
            }),
        );
        let routed2 = pending.route_response(wire_msg2).await;
        assert!(routed2);

        let received2 = rx1.await.expect("rx1 should get response");
        assert!(matches!(received2, WireMessage::GetRes { msg_id: 10, .. }));
    }
}
