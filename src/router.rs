//! Router configuration for hc-membrane

use axum::{routing::{get, post}, Router};
use tower_http::cors::{Any, CorsLayer};

use crate::routes::{
    dht_get_links, dht_get_record, dht_publish, health_check, kitsune_routes, test_signal,
    websocket::ws_handler,
};
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
        // Test endpoint for signal forwarding (development only)
        .route("/test/signal", post(test_signal))
        // DHT endpoints
        .route("/dht/{dna_hash}/record/{hash}", get(dht_get_record))
        .route("/dht/{dna_hash}/links", get(dht_get_links))
        // DHT publish endpoint (via kitsune2)
        .route("/dht/{dna_hash}/publish", post(dht_publish))
        // Kitsune direct API
        .nest(
            "/k2",
            kitsune_routes().with_state(app_state.kitsune_state.clone()),
        )
        .with_state(app_state)
        .layer(cors)
}
