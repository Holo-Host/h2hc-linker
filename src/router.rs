//! Router configuration for hc-membrane

use axum::{
    routing::{delete, get, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};

use crate::auth::admin::{add_agent, list_agents, remove_agent};
use crate::auth::middleware::{
    require_admin_secret, require_dht_read, require_dht_write, require_k2,
};
use crate::routes::{
    dht_get_details, dht_get_links, dht_get_record, dht_publish, health_check, kitsune_routes,
    test_signal, websocket::ws_handler, zome_call,
};
use crate::service::AppState;

/// Create the main router for hc-membrane
pub fn create_router(app_state: AppState) -> Router {
    // CORS configuration - allow all origins for development
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    if app_state.auth_store.is_some() {
        create_authenticated_router(app_state, cors)
    } else {
        create_open_router(app_state, cors)
    }
}

/// Router with no auth (current behavior when HC_MEMBRANE_ADMIN_SECRET is not set).
fn create_open_router(app_state: AppState, cors: CorsLayer) -> Router {
    Router::new()
        // Health check
        .route("/health", get(health_check))
        // WebSocket for browser extension connections
        .route("/ws", get(ws_handler))
        // Test endpoint for signal forwarding (development only)
        .route("/test/signal", post(test_signal))
        // DHT endpoints
        .route("/dht/{dna_hash}/record/{hash}", get(dht_get_record))
        .route("/dht/{dna_hash}/details/{hash}", get(dht_get_details))
        .route("/dht/{dna_hash}/links", get(dht_get_links))
        // DHT publish endpoint (via kitsune2)
        .route("/dht/{dna_hash}/publish", post(dht_publish))
        // Zome call endpoint (via conductor)
        .route("/api/{dna_hash}/{zome_name}/{fn_name}", get(zome_call))
        // Kitsune direct API
        .nest(
            "/k2",
            kitsune_routes().with_state(app_state.kitsune_state.clone()),
        )
        .with_state(app_state)
        .layer(cors)
}

/// Router with auth middleware (when HC_MEMBRANE_ADMIN_SECRET is set).
fn create_authenticated_router(app_state: AppState, cors: CorsLayer) -> Router {
    // DHT read routes
    let dht_read_routes = Router::new()
        .route("/dht/{dna_hash}/record/{hash}", get(dht_get_record))
        .route("/dht/{dna_hash}/details/{hash}", get(dht_get_details))
        .route("/dht/{dna_hash}/links", get(dht_get_links))
        .route_layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            require_dht_read,
        ));

    // DHT write routes
    let dht_write_routes = Router::new()
        .route("/dht/{dna_hash}/publish", post(dht_publish))
        .route_layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            require_dht_write,
        ));

    // K2 routes
    let k2_routes = kitsune_routes()
        .with_state(app_state.kitsune_state.clone())
        .route_layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            require_k2,
        ));

    // Admin routes
    let admin_routes = Router::new()
        .route("/agents", post(add_agent))
        .route("/agents", delete(remove_agent))
        .route("/agents", get(list_agents))
        .route_layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            require_admin_secret,
        ));

    Router::new()
        // Always open
        .route("/health", get(health_check))
        .route("/ws", get(ws_handler))
        .route("/test/signal", post(test_signal))
        // Protected routes
        .merge(dht_read_routes)
        .merge(dht_write_routes)
        .nest("/k2", k2_routes)
        .nest("/admin", admin_routes)
        // Zome call (deprecated, no auth)
        .route("/api/{dna_hash}/{zome_name}/{fn_name}", get(zome_call))
        .with_state(app_state)
        .layer(cors)
}
