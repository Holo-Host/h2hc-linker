//! DHT endpoints for h2hc-linker.
//!
//! These endpoints query the DHT directly via kitsune2 wire protocol.

use axum::extract::{Path, Query, State};
use axum::Json;
use holochain_types::prelude::{
    ActionHash, AgentPubKey, AnyDhtHash, AnyLinkableHash, EntryHash, ExternalHash,
};
use serde::{Deserialize, Serialize};

use crate::error::{HcMembraneError, HcMembraneResult};
use crate::service::AppState;

// For direct DHT queries
use holo_hash::HashableContentExtSync;
use holochain_types::link::WireLinkKey;
use holochain_types::prelude::{Action, CreateLink, LinkTag, LinkTypeFilter};
use holochain_zome_types::link::Link;

// For wire_ops_to_details_json
use holochain_types::action::WireNewEntryAction;
use holochain_types::dht_op::WireOps;
use holochain_types::entry::WireEntryOps;
use holochain_types::record::WireRecordOps;
use holochain_zome_types::metadata::{Details, EntryDetails, EntryDhtStatus, RecordDetails};
use holochain_types::prelude::ActionHashed;
use holochain_zome_types::record::{Record, SignedActionHashed};
use holochain_zome_types::validate::ValidationStatus;

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
/// Queries the DHT directly via kitsune2 and converts WireOps to the
/// Details format matching Holochain's get_details return type.
#[tracing::instrument(skip(state))]
pub async fn dht_get_details(
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
            let json_value = wire_ops_to_details_json(&wire_ops);
            Ok(Json(json_value))
        }
        None => Ok(Json(serde_json::Value::Null)),
    }
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
// Additional kitsune endpoints
// ============================================================================

/// GET /dht/{dna_hash}/count_links
///
/// Count links from a base hash.
/// Queries the DHT directly via kitsune2.
/// Returns the count of matching links as a JSON number.
#[tracing::instrument(skip(state))]
pub async fn dht_count_links(
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

    let tag_prefix = if let Some(tag_str) = query.tag {
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &tag_str)
            .map_err(|_| HcMembraneError::RequestMalformed("Invalid tag encoding".to_string()))?;
        Some(LinkTag::new(bytes))
    } else {
        None
    };

    let type_query = match (query.zome_index, query.link_type) {
        (Some(zome_idx), Some(link_type)) => {
            LinkTypeFilter::single_type(zome_idx.into(), (link_type as u8).into())
        }
        (Some(zome_idx), None) => LinkTypeFilter::single_dep(zome_idx.into()),
        (None, Some(_)) => {
            return Err(HcMembraneError::RequestMalformed(
                "zome_index is required when filtering by link type".to_string(),
            ));
        }
        (None, None) => LinkTypeFilter::Types(Vec::new()),
    };

    let query = holochain_types::link::WireLinkQuery {
        base,
        link_type: type_query,
        tag_prefix,
        before: None,
        after: None,
        author: None,
    };

    let result = dht_query.count_links(&dna_hash, query).await?;

    match result {
        Some(count_response) => {
            let json_value = serde_json::to_value(&count_response).unwrap_or_else(|e| {
                tracing::warn!("Failed to serialize count_links response: {}", e);
                serde_json::json!(0)
            });
            Ok(Json(json_value))
        }
        None => Ok(Json(serde_json::json!(0))),
    }
}

// ============================================================================
// Agent activity endpoints
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct AgentActivityPath {
    pub dna_hash: String,
    pub agent_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct AgentActivityQuery {
    /// "status" or "full" (default: "full")
    pub request: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MustGetAgentActivityBody {
    pub agent: String,
    pub chain_top: String,
    #[serde(default)]
    pub include_cached_entries: bool,
}

fn parse_agent_pubkey(s: &str) -> HcMembraneResult<AgentPubKey> {
    AgentPubKey::try_from(s)
        .map_err(|_| HcMembraneError::RequestMalformed(format!("Invalid agent pubkey: {}", s)))
}

fn parse_action_hash(s: &str) -> HcMembraneResult<ActionHash> {
    ActionHash::try_from(s)
        .map_err(|_| HcMembraneError::RequestMalformed(format!("Invalid action hash: {}", s)))
}

/// GET /dht/{dna_hash}/agent_activity/{agent_hash}
///
/// Get agent activity (chain status and action hashes) for an agent.
/// Queries the DHT directly via kitsune2.
#[tracing::instrument(skip(state))]
pub async fn dht_get_agent_activity(
    Path(path): Path<AgentActivityPath>,
    Query(query): Query<AgentActivityQuery>,
    State(state): State<AppState>,
) -> HcMembraneResult<Json<serde_json::Value>> {
    let dht_query = state
        .dht_query
        .as_ref()
        .ok_or_else(|| HcMembraneError::Internal("DHT queries not available".to_string()))?;

    let dna_hash = parse_dna_hash(&path.dna_hash)?;
    let agent = parse_agent_pubkey(&path.agent_hash)?;

    let is_full = query.request.as_deref() != Some("status");
    let options = holochain_p2p::event::GetActivityOptions {
        include_valid_activity: is_full,
        include_rejected_activity: is_full,
        include_warrants: true,
        include_full_records: false,
    };

    let chain_query = holochain_zome_types::query::ChainQueryFilter::new();

    let result = dht_query
        .get_agent_activity(&dna_hash, agent, chain_query, options)
        .await?;

    match result {
        Some(response) => {
            let json_value = serde_json::to_value(&response).unwrap_or(serde_json::Value::Null);
            Ok(Json(json_value))
        }
        None => Ok(Json(serde_json::Value::Null)),
    }
}

/// POST /dht/{dna_hash}/must_get_agent_activity
///
/// Get agent activity with must-get semantics (chain filter walk).
/// Queries the DHT directly via kitsune2.
#[tracing::instrument(skip(state))]
pub async fn dht_must_get_agent_activity(
    Path(path): Path<LinksPath>,
    State(state): State<AppState>,
    Json(body): Json<MustGetAgentActivityBody>,
) -> HcMembraneResult<Json<serde_json::Value>> {
    let dht_query = state
        .dht_query
        .as_ref()
        .ok_or_else(|| HcMembraneError::Internal("DHT queries not available".to_string()))?;

    let dna_hash = parse_dna_hash(&path.dna_hash)?;
    let agent = parse_agent_pubkey(&body.agent)?;
    let chain_top = parse_action_hash(&body.chain_top)?;

    let filter = holochain_zome_types::chain::ChainFilter {
        chain_top,
        limit_conditions: holochain_zome_types::chain::LimitConditions::ToGenesis,
        include_cached_entries: body.include_cached_entries,
    };

    let result = dht_query
        .must_get_agent_activity(&dna_hash, agent, filter)
        .await?;

    match result {
        Some(response) => {
            let json_value = serde_json::to_value(&response).unwrap_or(serde_json::Value::Null);
            Ok(Json(json_value))
        }
        None => Ok(Json(serde_json::Value::Null)),
    }
}

// ============================================================================
// Wire type to JSON conversion (for direct DHT mode)
// ============================================================================

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

/// Convert WireOps to the Details JSON format matching Holochain's get_details return type.
///
/// WireOps are the condensed wire protocol format. This function reconstructs the full
/// Details structure (adjacently tagged enum with {type, content}) that Holochain returns
/// from get_details calls.
fn wire_ops_to_details_json(ops: &WireOps) -> serde_json::Value {
    match ops {
        WireOps::Record(record_ops) => wire_record_ops_to_details(record_ops),
        WireOps::Entry(entry_ops) => wire_entry_ops_to_details(entry_ops),
        WireOps::Warrant(_) => serde_json::Value::Null,
    }
}

/// Convert WireRecordOps to Details::Record JSON.
///
/// Reconstructs the full Record (signed action + entry) and metadata (deletes, updates)
/// from the condensed wire format.
fn wire_record_ops_to_details(record_ops: &WireRecordOps) -> serde_json::Value {
    let judged_signed_action = match &record_ops.action {
        Some(a) => a,
        None => return serde_json::Value::Null,
    };

    let signed_action = &judged_signed_action.data;
    let action: &Action = signed_action.data();
    let signature = signed_action.signature().clone();

    // Compute action hash and build SignedActionHashed
    let action_hashed = ActionHashed::from_content_sync(action.clone());
    let signed_action_hashed =
        SignedActionHashed::with_presigned(action_hashed, signature);

    // Build record with entry
    let record = Record::new(signed_action_hashed, record_ops.entry.clone());

    let validation_status = judged_signed_action
        .status
        .unwrap_or(ValidationStatus::Valid);

    // Convert deletes: WireDelete → SignedActionHashed
    let deletes: Vec<SignedActionHashed> = record_ops
        .deletes
        .iter()
        .map(|d| {
            let delete_action = Action::Delete(d.data.delete.clone());
            let delete_hashed = ActionHashed::from_content_sync(delete_action);
            SignedActionHashed::with_presigned(delete_hashed, d.data.signature.clone())
        })
        .collect();

    // Convert updates: WireUpdateRelationship → SignedActionHashed
    // Need the original entry address from the record's action
    let original_entry_address = action.entry_data().map(|(eh, _)| eh.clone());
    let updates: Vec<SignedActionHashed> = record_ops
        .updates
        .iter()
        .filter_map(|u| {
            let original_entry_addr = original_entry_address.clone()?;
            let update = holochain_zome_types::action::Update {
                author: u.data.author.clone(),
                timestamp: u.data.timestamp,
                action_seq: u.data.action_seq,
                prev_action: u.data.prev_action.clone(),
                original_action_address: u.data.original_action_address.clone(),
                original_entry_address: original_entry_addr,
                entry_type: u.data.new_entry_type.clone(),
                entry_hash: u.data.new_entry_address.clone(),
                weight: u.data.weight.clone(),
            };
            let update_action = Action::Update(update);
            let update_hashed = ActionHashed::from_content_sync(update_action);
            Some(SignedActionHashed::with_presigned(
                update_hashed,
                u.data.signature.clone(),
            ))
        })
        .collect();

    let record_details = RecordDetails {
        record,
        validation_status,
        deletes,
        updates,
    };

    let details = Details::Record(record_details);
    serde_json::to_value(&details).unwrap_or(serde_json::Value::Null)
}

/// Convert WireEntryOps to Details::Entry JSON.
///
/// Reconstructs the full Entry details including all create actions, deletes, and updates
/// from the condensed wire format. Entry data and entry type are shared across all creates.
fn wire_entry_ops_to_details(entry_ops: &WireEntryOps) -> serde_json::Value {
    let entry_data = match &entry_ops.entry {
        Some(ed) => ed,
        None => return serde_json::Value::Null,
    };

    let entry = entry_data.entry.clone();
    let entry_type = entry_data.entry_type.clone();

    // Compute entry hash for reconstructing actions
    let entry_hash = EntryHash::with_data_sync(&entry);

    // Convert creates to SignedActionHashed, separating valid from rejected
    let mut actions = Vec::new();
    let mut rejected_actions = Vec::new();

    for judged_create in &entry_ops.creates {
        let (full_action, signature) = match &judged_create.data {
            WireNewEntryAction::Create(wire_create) => {
                let create = holochain_zome_types::action::Create {
                    author: wire_create.author.clone(),
                    timestamp: wire_create.timestamp,
                    action_seq: wire_create.action_seq,
                    prev_action: wire_create.prev_action.clone(),
                    entry_type: entry_type.clone(),
                    entry_hash: entry_hash.clone(),
                    weight: wire_create.weight.clone(),
                };
                (Action::Create(create), wire_create.signature.clone())
            }
            WireNewEntryAction::Update(wire_update) => {
                let update = holochain_zome_types::action::Update {
                    author: wire_update.author.clone(),
                    timestamp: wire_update.timestamp,
                    action_seq: wire_update.action_seq,
                    prev_action: wire_update.prev_action.clone(),
                    original_entry_address: wire_update.original_entry_address.clone(),
                    original_action_address: wire_update.original_action_address.clone(),
                    entry_type: entry_type.clone(),
                    entry_hash: entry_hash.clone(),
                    weight: wire_update.weight.clone(),
                };
                (Action::Update(update), wire_update.signature.clone())
            }
        };

        let action_hashed = ActionHashed::from_content_sync(full_action);
        let signed_action_hashed =
            SignedActionHashed::with_presigned(action_hashed, signature);

        let is_valid = judged_create
            .status
            .map(|s| s == ValidationStatus::Valid)
            .unwrap_or(true);

        if is_valid {
            actions.push(signed_action_hashed);
        } else {
            rejected_actions.push(signed_action_hashed);
        }
    }

    // Convert deletes: WireDelete → SignedActionHashed
    let deletes: Vec<SignedActionHashed> = entry_ops
        .deletes
        .iter()
        .map(|d| {
            let delete_action = Action::Delete(d.data.delete.clone());
            let delete_hashed = ActionHashed::from_content_sync(delete_action);
            SignedActionHashed::with_presigned(delete_hashed, d.data.signature.clone())
        })
        .collect();

    // Convert updates: WireUpdateRelationship → SignedActionHashed
    // For entry details, original_entry_address is the entry we're querying
    let updates: Vec<SignedActionHashed> = entry_ops
        .updates
        .iter()
        .map(|u| {
            let update = holochain_zome_types::action::Update {
                author: u.data.author.clone(),
                timestamp: u.data.timestamp,
                action_seq: u.data.action_seq,
                prev_action: u.data.prev_action.clone(),
                original_action_address: u.data.original_action_address.clone(),
                original_entry_address: entry_hash.clone(),
                entry_type: u.data.new_entry_type.clone(),
                entry_hash: u.data.new_entry_address.clone(),
                weight: u.data.weight.clone(),
            };
            let update_action = Action::Update(update);
            let update_hashed = ActionHashed::from_content_sync(update_action);
            SignedActionHashed::with_presigned(update_hashed, u.data.signature.clone())
        })
        .collect();

    // Entry is Live if it has non-rejected creates and not all are deleted
    let entry_dht_status = if !actions.is_empty() && deletes.len() < actions.len() {
        EntryDhtStatus::Live
    } else if !actions.is_empty() {
        EntryDhtStatus::Dead
    } else {
        EntryDhtStatus::Live // Default to Live if no status info
    };

    let entry_details = EntryDetails {
        entry,
        actions,
        rejected_actions,
        deletes,
        updates,
        entry_dht_status,
    };

    let details = Details::Entry(entry_details);
    serde_json::to_value(&details).unwrap_or(serde_json::Value::Null)
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

#[cfg(test)]
mod tests {
    use holochain_types::action::{
        WireCreate, WireDelete, WireNewEntryAction, WireUpdate, WireUpdateRelationship,
    };
    use holochain_types::dht_op::WireOps;
    use holochain_types::entry::{EntryData, WireEntryOps};
    use holochain_types::prelude::*;
    use holochain_types::record::WireRecordOps;
    use holochain_zome_types::judged::Judged;

    use super::{wire_entry_ops_to_details, wire_ops_to_details_json, wire_record_ops_to_details};

    fn test_agent() -> AgentPubKey {
        AgentPubKey::from_raw_32(vec![1; 32])
    }

    fn test_entry_hash() -> EntryHash {
        EntryHash::from_raw_32(vec![2; 32])
    }

    fn test_action_hash() -> ActionHash {
        ActionHash::from_raw_32(vec![3; 32])
    }

    fn test_prev_action() -> ActionHash {
        ActionHash::from_raw_32(vec![4; 32])
    }

    fn test_signature() -> Signature {
        Signature::from([0xaa; 64])
    }

    fn test_entry_type() -> EntryType {
        EntryType::App(AppEntryDef {
            entry_index: 0.into(),
            zome_index: 0.into(),
            visibility: EntryVisibility::Public,
        })
    }

    fn test_entry() -> Entry {
        let entry_bytes = UnsafeBytes::from(vec![1u8, 2, 3]);
        Entry::App(AppEntryBytes(SerializedBytes::from(entry_bytes)))
    }

    fn test_create_action() -> Action {
        Action::Create(Create {
            author: test_agent(),
            timestamp: Timestamp::from_micros(1_000_000),
            action_seq: 5,
            prev_action: test_prev_action(),
            entry_type: test_entry_type(),
            entry_hash: test_entry_hash(),
            weight: EntryRateWeight::default(),
        })
    }

    fn test_wire_create() -> WireCreate {
        WireCreate {
            timestamp: Timestamp::from_micros(1_000_000),
            author: test_agent(),
            action_seq: 5,
            prev_action: test_prev_action(),
            signature: test_signature(),
            weight: EntryRateWeight::default(),
        }
    }

    fn test_wire_delete() -> WireDelete {
        WireDelete {
            delete: Delete {
                author: test_agent(),
                timestamp: Timestamp::from_micros(2_000_000),
                action_seq: 6,
                prev_action: test_action_hash(),
                deletes_address: test_action_hash(),
                deletes_entry_address: test_entry_hash(),
                weight: RateWeight::default(),
            },
            signature: Signature::from([0xbb; 64]),
        }
    }

    fn test_wire_update_relationship() -> WireUpdateRelationship {
        WireUpdateRelationship {
            timestamp: Timestamp::from_micros(2_000_000),
            author: test_agent(),
            action_seq: 6,
            prev_action: test_action_hash(),
            original_action_address: test_action_hash(),
            new_entry_address: EntryHash::from_raw_32(vec![5; 32]),
            new_entry_type: test_entry_type(),
            signature: Signature::from([0xcc; 64]),
            weight: EntryRateWeight::default(),
        }
    }

    // ========================================================================
    // wire_record_ops_to_details tests
    // ========================================================================

    #[test]
    fn test_wire_record_ops_to_details_basic() {
        let action = test_create_action();
        let signed_action = SignedAction::new(action, test_signature());

        let record_ops = WireRecordOps {
            action: Some(Judged::valid(signed_action)),
            deletes: vec![],
            updates: vec![],
            entry: Some(test_entry()),
        };

        let result = wire_record_ops_to_details(&record_ops);

        let obj = result.as_object().expect("should be object");
        assert_eq!(obj["type"], "Record");

        let content = &obj["content"];
        assert!(content["record"].is_object());
        assert_eq!(content["validation_status"], "Valid");
        assert!(content["deletes"].as_array().unwrap().is_empty());
        assert!(content["updates"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_wire_record_ops_to_details_no_action_returns_null() {
        let record_ops = WireRecordOps {
            action: None,
            deletes: vec![],
            updates: vec![],
            entry: None,
        };

        let result = wire_record_ops_to_details(&record_ops);
        assert!(result.is_null());
    }

    #[test]
    fn test_wire_record_ops_to_details_with_deletes() {
        let action = test_create_action();
        let signed_action = SignedAction::new(action, test_signature());

        let record_ops = WireRecordOps {
            action: Some(Judged::valid(signed_action)),
            deletes: vec![Judged::valid(test_wire_delete())],
            updates: vec![],
            entry: Some(test_entry()),
        };

        let result = wire_record_ops_to_details(&record_ops);

        let content = &result["content"];
        let deletes = content["deletes"].as_array().unwrap();
        assert_eq!(deletes.len(), 1);
    }

    #[test]
    fn test_wire_record_ops_to_details_with_updates() {
        let action = test_create_action();
        let signed_action = SignedAction::new(action, test_signature());

        let record_ops = WireRecordOps {
            action: Some(Judged::valid(signed_action)),
            deletes: vec![],
            updates: vec![Judged::valid(test_wire_update_relationship())],
            entry: Some(test_entry()),
        };

        let result = wire_record_ops_to_details(&record_ops);

        let content = &result["content"];
        let updates = content["updates"].as_array().unwrap();
        assert_eq!(updates.len(), 1);
    }

    #[test]
    fn test_wire_record_ops_to_details_no_entry() {
        // Record with action but no entry (e.g., Delete action)
        let action = test_create_action();
        let signed_action = SignedAction::new(action, test_signature());

        let record_ops = WireRecordOps {
            action: Some(Judged::valid(signed_action)),
            deletes: vec![],
            updates: vec![],
            entry: None,
        };

        let result = wire_record_ops_to_details(&record_ops);

        // Should still produce valid Details::Record with null entry in the record
        let obj = result.as_object().expect("should be object");
        assert_eq!(obj["type"], "Record");
    }

    // ========================================================================
    // wire_entry_ops_to_details tests
    // ========================================================================

    #[test]
    fn test_wire_entry_ops_to_details_basic() {
        let entry_data = EntryData {
            entry: test_entry(),
            entry_type: test_entry_type(),
        };

        let entry_ops = WireEntryOps {
            creates: vec![Judged::valid(WireNewEntryAction::Create(
                test_wire_create(),
            ))],
            deletes: vec![],
            updates: vec![],
            entry: Some(entry_data),
        };

        let result = wire_entry_ops_to_details(&entry_ops);

        let obj = result.as_object().expect("should be object");
        assert_eq!(obj["type"], "Entry");

        let content = &obj["content"];
        assert!(content["entry"].is_object());
        assert_eq!(content["actions"].as_array().unwrap().len(), 1);
        assert!(content["rejected_actions"].as_array().unwrap().is_empty());
        assert!(content["deletes"].as_array().unwrap().is_empty());
        assert!(content["updates"].as_array().unwrap().is_empty());
        assert_eq!(content["entry_dht_status"], "Live");
    }

    #[test]
    fn test_wire_entry_ops_to_details_no_entry_returns_null() {
        let entry_ops = WireEntryOps {
            creates: vec![],
            deletes: vec![],
            updates: vec![],
            entry: None,
        };

        let result = wire_entry_ops_to_details(&entry_ops);
        assert!(result.is_null());
    }

    #[test]
    fn test_wire_entry_ops_to_details_dead_status() {
        let entry = test_entry();
        let entry_hash = EntryHash::with_data_sync(&entry);

        let entry_data = EntryData {
            entry,
            entry_type: test_entry_type(),
        };

        // One create + one delete → Dead (deletes >= actions)
        let wire_delete = WireDelete {
            delete: Delete {
                author: test_agent(),
                timestamp: Timestamp::from_micros(2_000_000),
                action_seq: 6,
                prev_action: test_action_hash(),
                deletes_address: test_action_hash(),
                deletes_entry_address: entry_hash,
                weight: RateWeight::default(),
            },
            signature: Signature::from([0xbb; 64]),
        };

        let entry_ops = WireEntryOps {
            creates: vec![Judged::valid(WireNewEntryAction::Create(
                test_wire_create(),
            ))],
            deletes: vec![Judged::valid(wire_delete)],
            updates: vec![],
            entry: Some(entry_data),
        };

        let result = wire_entry_ops_to_details(&entry_ops);

        let content = &result["content"];
        assert_eq!(content["entry_dht_status"], "Dead");
    }

    #[test]
    fn test_wire_entry_ops_to_details_with_rejected_creates() {
        let valid_create = test_wire_create();

        let rejected_create = WireCreate {
            timestamp: Timestamp::from_micros(2_000_000),
            author: AgentPubKey::from_raw_32(vec![9; 32]),
            action_seq: 3,
            prev_action: ActionHash::from_raw_32(vec![8; 32]),
            signature: Signature::from([0xcc; 64]),
            weight: EntryRateWeight::default(),
        };

        let entry_data = EntryData {
            entry: test_entry(),
            entry_type: test_entry_type(),
        };

        let entry_ops = WireEntryOps {
            creates: vec![
                Judged::valid(WireNewEntryAction::Create(valid_create)),
                Judged::new(
                    WireNewEntryAction::Create(rejected_create),
                    ValidationStatus::Rejected,
                ),
            ],
            deletes: vec![],
            updates: vec![],
            entry: Some(entry_data),
        };

        let result = wire_entry_ops_to_details(&entry_ops);

        let content = &result["content"];
        assert_eq!(content["actions"].as_array().unwrap().len(), 1);
        assert_eq!(content["rejected_actions"].as_array().unwrap().len(), 1);
        assert_eq!(content["entry_dht_status"], "Live");
    }

    #[test]
    fn test_wire_entry_ops_to_details_with_update_create() {
        // Test WireNewEntryAction::Update variant (entry created via an update action)
        let wire_update = WireUpdate {
            timestamp: Timestamp::from_micros(1_000_000),
            author: test_agent(),
            action_seq: 5,
            prev_action: test_prev_action(),
            original_entry_address: EntryHash::from_raw_32(vec![7; 32]),
            original_action_address: ActionHash::from_raw_32(vec![8; 32]),
            signature: test_signature(),
            weight: EntryRateWeight::default(),
        };

        let entry_data = EntryData {
            entry: test_entry(),
            entry_type: test_entry_type(),
        };

        let entry_ops = WireEntryOps {
            creates: vec![Judged::valid(WireNewEntryAction::Update(wire_update))],
            deletes: vec![],
            updates: vec![],
            entry: Some(entry_data),
        };

        let result = wire_entry_ops_to_details(&entry_ops);

        let obj = result.as_object().expect("should be object");
        assert_eq!(obj["type"], "Entry");

        let content = &obj["content"];
        assert_eq!(content["actions"].as_array().unwrap().len(), 1);
        assert_eq!(content["entry_dht_status"], "Live");
    }

    #[test]
    fn test_wire_entry_ops_to_details_with_deletes_and_updates() {
        let entry_data = EntryData {
            entry: test_entry(),
            entry_type: test_entry_type(),
        };

        // Two creates, one delete (still Live), one update
        let create2 = WireCreate {
            timestamp: Timestamp::from_micros(1_500_000),
            author: AgentPubKey::from_raw_32(vec![6; 32]),
            action_seq: 3,
            prev_action: ActionHash::from_raw_32(vec![7; 32]),
            signature: Signature::from([0xdd; 64]),
            weight: EntryRateWeight::default(),
        };

        let entry_ops = WireEntryOps {
            creates: vec![
                Judged::valid(WireNewEntryAction::Create(test_wire_create())),
                Judged::valid(WireNewEntryAction::Create(create2)),
            ],
            deletes: vec![Judged::valid(test_wire_delete())],
            updates: vec![Judged::valid(test_wire_update_relationship())],
            entry: Some(entry_data),
        };

        let result = wire_entry_ops_to_details(&entry_ops);

        let content = &result["content"];
        assert_eq!(content["actions"].as_array().unwrap().len(), 2);
        assert_eq!(content["deletes"].as_array().unwrap().len(), 1);
        assert_eq!(content["updates"].as_array().unwrap().len(), 1);
        // 2 creates, 1 delete → still Live (deletes < actions)
        assert_eq!(content["entry_dht_status"], "Live");
    }

    // ========================================================================
    // wire_ops_to_details_json dispatcher tests
    // ========================================================================

    #[test]
    fn test_wire_ops_to_details_json_record_variant() {
        let action = test_create_action();
        let signed_action = SignedAction::new(action, test_signature());

        let record_ops = WireRecordOps {
            action: Some(Judged::valid(signed_action)),
            deletes: vec![],
            updates: vec![],
            entry: Some(test_entry()),
        };

        let result = wire_ops_to_details_json(&WireOps::Record(record_ops));
        assert_eq!(result["type"], "Record");
    }

    #[test]
    fn test_wire_ops_to_details_json_entry_variant() {
        let entry_data = EntryData {
            entry: test_entry(),
            entry_type: test_entry_type(),
        };

        let entry_ops = WireEntryOps {
            creates: vec![Judged::valid(WireNewEntryAction::Create(
                test_wire_create(),
            ))],
            deletes: vec![],
            updates: vec![],
            entry: Some(entry_data),
        };

        let result = wire_ops_to_details_json(&WireOps::Entry(entry_ops));
        assert_eq!(result["type"], "Entry");
    }

}
