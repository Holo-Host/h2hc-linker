//! HTTP routes for hc-membrane
//!
//! ## API Structure
//!
//! - `/hc/*` - Holochain semantic API (get, get_links, publish, etc.)
//! - `/k2/*` - Kitsune direct API (network status, peers, liveness)
//! - `/health` - Health check endpoint

pub mod health;
pub mod kitsune;

pub use health::health_check;
pub use kitsune::kitsune_routes;
