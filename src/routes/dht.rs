//! DHT endpoints for hc-membrane.
//!
//! These endpoints query the DHT directly via kitsune2 wire protocol.

use axum::extract::{Path, Query, State};
use axum::Json;
use holochain_types::prelude::{
    ActionHash, AgentPubKey, AnyDhtHash, AnyLinkableHash, EntryHash, ExternIO, ExternalHash,
};
use serde::{Deserialize, Serialize};

use crate::error::{HcMembraneError, HcMembraneResult};
use crate::service::AppState;

// For direct DHT queries
use holo_hash::HashableContentExtSync;
use holochain_types::link::WireLinkKey;
use holochain_types::prelude::{Action, CreateLink, LinkTag, LinkTypeFilter};
use holochain_zome_types::link::Link;

// ============================================================================
// Hash parsing helpers
// ============================================================================

fn parse_dna_hash(s: &str) -> HcMembraneResult<holochain_types::dna::DnaHash> {
    holochain_types::dna::DnaHash::try_from(s)
        .map_err(|_| HcMembraneError::RequestMalformed(format!("Invalid DNA hash: {}", s)))
}

fn parse_any_dht_hash(s: &str) -> HcMembraneResult<AnyDhtHash> {
    if let Ok(hash) = EntryHash::try_from(s) {
        return Ok(AnyDhtHash::from(hash));
    }
    if let Ok(hash) = ActionHash::try_from(s) {
        return Ok(AnyDhtHash::from(hash));
    }
    Err(HcMembraneError::RequestMalformed(format!(
        "Invalid DHT hash: {}",
        s
    )))
}

fn parse_any_linkable_hash(s: &str) -> HcMembraneResult<AnyLinkableHash> {
    if let Ok(hash) = AgentPubKey::try_from(s) {
        return Ok(AnyLinkableHash::from(hash));
    }
    if let Ok(hash) = EntryHash::try_from(s) {
        return Ok(AnyLinkableHash::from(hash));
    }
    if let Ok(hash) = ActionHash::try_from(s) {
        return Ok(AnyLinkableHash::from(hash));
    }
    if let Ok(hash) = ExternalHash::try_from(s) {
        return Ok(AnyLinkableHash::from(hash));
    }
    Err(HcMembraneError::RequestMalformed(format!(
        "Invalid linkable hash: {}",
        s
    )))
}

// ============================================================================
// HTTP request/response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct RecordPath {
    pub dna_hash: String,
    pub hash: String,
}

#[derive(Debug, Deserialize)]
pub struct LinksPath {
    pub dna_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct LinksQuery {
    pub base: String,
    #[serde(rename = "type")]
    pub link_type: Option<u16>,
    pub tag: Option<String>,
    /// Optional zome index for filtering links. Required when filtering by link_type.
    /// If omitted and no link_type filter, returns links from all zomes.
    pub zome_index: Option<u8>,
}

// ============================================================================
// Endpoints
// ============================================================================

/// GET /dht/{dna_hash}/record/{hash}
///
/// Get a record by action or entry hash.
/// Queries the DHT directly via kitsune2.
#[tracing::instrument(skip(state))]
pub async fn dht_get_record(
    Path(path): Path<RecordPath>,
    State(state): State<AppState>,
) -> HcMembraneResult<Json<serde_json::Value>> {
    let dht_query = state
        .dht_query
        .as_ref()
        .ok_or_else(|| HcMembraneError::Internal("DHT queries not available".to_string()))?;

    let dna_hash = parse_dna_hash(&path.dna_hash)?;
    let hash = parse_any_dht_hash(&path.hash)?;

    let result = dht_query.get(&dna_hash, hash).await?;

    match result {
        Some(wire_ops) => {
            let json_value = wire_ops_to_json(&wire_ops);
            Ok(Json(json_value))
        }
        None => Ok(Json(serde_json::Value::Null)),
    }
}

/// GET /dht/{dna_hash}/details/{hash}
///
/// Get details for a hash (including updates and deletes).
/// Uses the conductor's dht_util zome (requires conductor connection).
#[tracing::instrument(skip(state))]
pub async fn dht_get_details(
    Path(path): Path<RecordPath>,
    State(state): State<AppState>,
) -> HcMembraneResult<Json<serde_json::Value>> {
    let app_conn = state
        .app_conn
        .as_ref()
        .ok_or_else(|| HcMembraneError::Internal("Conductor not configured".to_string()))?;

    let dna_hash = parse_dna_hash(&path.dna_hash)?;
    let hash = parse_any_dht_hash(&path.hash)?;

    let input = GetRecordInput {
        hash,
        options: GetOptionsInput {
            strategy: GetStrategyInput::Network,
        },
    };

    let payload = ExternIO::encode(input)
        .map_err(|e| HcMembraneError::Serialization(format!("Failed to encode: {}", e)))?;

    let result = app_conn
        .call_dht_util(&dna_hash, "dht_get_details", payload)
        .await?;

    let json_value: serde_json::Value = result
        .decode()
        .map_err(|e| HcMembraneError::Serialization(format!("Failed to decode: {}", e)))?;

    Ok(Json(json_value))
}

/// GET /dht/{dna_hash}/links
///
/// Get links from a base hash.
/// Queries the DHT directly via kitsune2.
#[tracing::instrument(skip(state))]
pub async fn dht_get_links(
    Path(path): Path<LinksPath>,
    Query(query): Query<LinksQuery>,
    State(state): State<AppState>,
) -> HcMembraneResult<Json<serde_json::Value>> {
    let dht_query = state
        .dht_query
        .as_ref()
        .ok_or_else(|| HcMembraneError::Internal("DHT queries not available".to_string()))?;

    let dna_hash = parse_dna_hash(&path.dna_hash)?;
    let base = parse_any_linkable_hash(&query.base)?;

    let tag = if let Some(tag_str) = query.tag {
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &tag_str)
            .map_err(|_| HcMembraneError::RequestMalformed("Invalid tag encoding".to_string()))?;
        Some(LinkTag::new(bytes))
    } else {
        None
    };

    // Build the WireLinkKey for the query
    // - If zome_index + link_type provided: filter by specific zome and type
    // - If zome_index only: all types from that zome
    // - If neither: match all links (Types(Vec::new()))
    // - If link_type without zome_index: error (zome_index required for type filtering)
    let type_query = match (query.zome_index, query.link_type) {
        (Some(zome_idx), Some(link_type)) => {
            // Specific zome and link type
            LinkTypeFilter::single_type(zome_idx.into(), (link_type as u8).into())
        }
        (Some(zome_idx), None) => {
            // All link types from specific zome
            LinkTypeFilter::single_dep(zome_idx.into())
        }
        (None, Some(_)) => {
            // link_type without zome_index - can't filter properly
            return Err(HcMembraneError::RequestMalformed(
                "zome_index is required when filtering by link type".to_string(),
            ));
        }
        (None, None) => {
            // No filtering - match all links from all zomes
            LinkTypeFilter::Types(Vec::new())
        }
    };

    let link_key = WireLinkKey {
        base: base.clone(),
        type_query,
        tag: tag.clone(),
        after: None,
        before: None,
        author: None, // Don't filter by author
    };

    let result = dht_query.get_links(&dna_hash, link_key).await?;

    match result {
        Some(wire_link_ops) => {
            tracing::debug!(
                creates_count = wire_link_ops.creates.len(),
                deletes_count = wire_link_ops.deletes.len(),
                "Direct DHT get_links response"
            );
            let links = wire_link_ops_to_links(&wire_link_ops, &base, tag.as_ref());
            let json_value = serde_json::to_value(&links).unwrap_or_else(|e| {
                tracing::warn!("Failed to serialize links: {}", e);
                serde_json::json!([])
            });
            Ok(Json(json_value))
        }
        None => Ok(Json(serde_json::json!([]))),
    }
}

// ============================================================================
// Wire type to JSON conversion
// ============================================================================

// Types used by dht_get_details (conductor endpoint)
#[derive(Debug, Default, Serialize, Deserialize)]
pub enum GetStrategyInput {
    Local,
    #[default]
    Network,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GetOptionsInput {
    #[serde(default)]
    pub strategy: GetStrategyInput,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetRecordInput {
    pub hash: AnyDhtHash,
    #[serde(default)]
    pub options: GetOptionsInput,
}

/// Convert WireOps to JSON.
fn wire_ops_to_json(ops: &holochain_types::dht_op::WireOps) -> serde_json::Value {
    // Serialize the WireOps directly to JSON
    // This preserves the full structure for debugging and compatibility
    match serde_json::to_value(ops) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!("Failed to serialize WireOps: {}", e);
            serde_json::Value::Null
        }
    }
}

/// Convert WireLinkOps to Vec<Link> with properly computed ActionHash.
///
/// This converts the wire protocol format to the same format that the conductor
/// returns, ensuring consistency.
fn wire_link_ops_to_links(
    ops: &holochain_types::link::WireLinkOps,
    base: &AnyLinkableHash,
    query_tag: Option<&LinkTag>,
) -> Vec<Link> {
    ops.creates
        .iter()
        .map(|wire_create| {
            // Get the tag from the wire create or fall back to query tag
            // If neither is available, use empty tag (the link exists but tag was optimized away)
            // Note: This may produce incorrect ActionHash if tag is missing
            let tag = wire_create
                .tag
                .clone()
                .or_else(|| query_tag.cloned())
                .unwrap_or_else(|| LinkTag::new(Vec::new()));

            // Reconstruct the full CreateLink action to compute its hash
            let create_link = CreateLink {
                author: wire_create.author.clone(),
                timestamp: wire_create.timestamp,
                action_seq: wire_create.action_seq,
                prev_action: wire_create.prev_action.clone(),
                base_address: base.clone(),
                target_address: wire_create.target_address.clone(),
                zome_index: wire_create.zome_index,
                link_type: wire_create.link_type,
                tag: tag.clone(),
                weight: wire_create.weight.clone(),
            };

            // Compute the ActionHash by hashing the Action
            let action = Action::CreateLink(create_link);
            let action_hash: ActionHash = action.to_hash();

            tracing::debug!(
                target = %wire_create.target_address,
                computed_hash = %action_hash,
                has_tag = wire_create.tag.is_some(),
                "Converted WireCreateLink to Link"
            );

            Link {
                author: wire_create.author.clone(),
                base: base.clone(),
                target: wire_create.target_address.clone(),
                timestamp: wire_create.timestamp,
                zome_index: wire_create.zome_index,
                link_type: wire_create.link_type,
                tag,
                create_link_hash: action_hash,
            }
        })
        .collect()
}
