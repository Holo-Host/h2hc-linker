//! Router configuration for hc-membrane

use axum::{routing::get, Router};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use crate::routes::{health_check, kitsune::KitsuneState, kitsune_routes};

/// Create the main router for hc-membrane
pub fn create_router(kitsune_state: Arc<KitsuneState>) -> Router {
    // CORS configuration - allow all origins for development
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health check
        .route("/health", get(health_check))
        // Kitsune direct API
        .nest("/k2", kitsune_routes().with_state(kitsune_state))
        // TODO: Holochain semantic API (/hc/*) will be added in M2
        .layer(cors)
}
