//! Error types for hc-membrane

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Result type for hc-membrane operations
pub type HcMembraneResult<T> = Result<T, HcMembraneError>;

/// Error type for hc-membrane operations
#[derive(Debug, thiserror::Error)]
pub enum HcMembraneError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Network error
    #[error("Network error: {0}")]
    Network(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Not found
    #[error("Not found: {0}")]
    NotFound(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

/// JSON error response
#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: u16,
}

impl IntoResponse for HcMembraneError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            HcMembraneError::Config(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            HcMembraneError::Network(msg) => (StatusCode::BAD_GATEWAY, msg.clone()),
            HcMembraneError::Serialization(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            HcMembraneError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            HcMembraneError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        };

        let body = serde_json::to_string(&ErrorResponse {
            error: message,
            code: status.as_u16(),
        })
        .unwrap_or_else(|_| r#"{"error":"serialization failed","code":500}"#.to_string());

        (status, body).into_response()
    }
}
