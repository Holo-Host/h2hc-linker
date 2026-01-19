//! DHT endpoints for hc-membrane.
//!
//! These endpoints provide DHT access via the conductor's dht_util zome.
//! This is temporary scaffolding that will be replaced with direct kitsune2
//! queries in M6.

use axum::extract::{Path, Query, State};
use axum::Json;
use holochain_types::prelude::{
    ActionHash, AgentPubKey, AnyDhtHash, AnyLinkableHash, EntryHash, ExternalHash, ExternIO,
};
use serde::{Deserialize, Serialize};

use crate::error::{HcMembraneError, HcMembraneResult};
use crate::service::AppState;

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
// Zome input types (must match dht_util zome)
// ============================================================================

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

#[derive(Debug, Serialize, Deserialize)]
pub struct GetLinksInput {
    pub base: AnyLinkableHash,
    #[serde(default)]
    pub link_type: Option<u16>,
    #[serde(default)]
    pub tag_prefix: Option<Vec<u8>>,
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
}

// ============================================================================
// Endpoints
// ============================================================================

/// GET /dht/{dna_hash}/record/{hash}
///
/// Get a record by action or entry hash.
#[tracing::instrument(skip(state))]
pub async fn dht_get_record(
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

    let result = app_conn.call_dht_util(&dna_hash, "dht_get_record", payload).await?;

    let json_value: serde_json::Value = result
        .decode()
        .map_err(|e| HcMembraneError::Serialization(format!("Failed to decode: {}", e)))?;

    Ok(Json(json_value))
}

/// GET /dht/{dna_hash}/details/{hash}
///
/// Get details for a hash (including updates and deletes).
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

    let result = app_conn.call_dht_util(&dna_hash, "dht_get_details", payload).await?;

    let json_value: serde_json::Value = result
        .decode()
        .map_err(|e| HcMembraneError::Serialization(format!("Failed to decode: {}", e)))?;

    Ok(Json(json_value))
}

/// GET /dht/{dna_hash}/links
///
/// Get links from a base hash.
#[tracing::instrument(skip(state))]
pub async fn dht_get_links(
    Path(path): Path<LinksPath>,
    Query(query): Query<LinksQuery>,
    State(state): State<AppState>,
) -> HcMembraneResult<Json<serde_json::Value>> {
    let app_conn = state
        .app_conn
        .as_ref()
        .ok_or_else(|| HcMembraneError::Internal("Conductor not configured".to_string()))?;

    let dna_hash = parse_dna_hash(&path.dna_hash)?;
    let base = parse_any_linkable_hash(&query.base)?;

    let tag_prefix = if let Some(tag) = query.tag {
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &tag)
            .map_err(|_| HcMembraneError::RequestMalformed("Invalid tag encoding".to_string()))?;
        Some(bytes)
    } else {
        None
    };

    let input = GetLinksInput {
        base,
        link_type: query.link_type,
        tag_prefix,
    };

    let payload = ExternIO::encode(input)
        .map_err(|e| HcMembraneError::Serialization(format!("Failed to encode: {}", e)))?;

    let result = app_conn.call_dht_util(&dna_hash, "dht_get_links", payload).await?;

    let json_value: serde_json::Value = result
        .decode()
        .map_err(|e| HcMembraneError::Serialization(format!("Failed to decode: {}", e)))?;

    Ok(Json(json_value))
}
