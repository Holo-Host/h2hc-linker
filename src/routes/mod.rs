//! HTTP routes for hc-membrane
//!
//! ## API Structure
//!
//! - `/hc/*` - Holochain semantic API (get, get_links, publish, etc.)
//! - `/k2/*` - Kitsune direct API (network status, peers, liveness)
//! - `/ws` - WebSocket for browser extension connections
//! - `/test/*` - Test endpoints for development
//! - `/health` - Health check endpoint

pub mod health;
pub mod kitsune;
pub mod test_signal;
pub mod websocket;

pub use health::health_check;
pub use kitsune::kitsune_routes;
pub use test_signal::test_signal;
