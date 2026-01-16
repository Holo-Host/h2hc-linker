//! Router configuration for hc-membrane

use axum::{routing::get, Router};
use tower_http::cors::{Any, CorsLayer};

use crate::routes::{health_check, kitsune_routes, websocket::ws_handler};
use crate::service::AppState;

/// Create the main router for hc-membrane
pub fn create_router(app_state: AppState) -> Router {
    // CORS configuration - allow all origins for development
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health check
        .route("/health", get(health_check))
        // WebSocket for browser extension connections
        .route("/ws", get(ws_handler))
        // Kitsune direct API
        .nest(
            "/k2",
            kitsune_routes().with_state(app_state.kitsune_state.clone()),
        )
        // TODO: Holochain semantic API (/hc/*) will be added in M2c-M2e
        .with_state(app_state)
        .layer(cors)
}
