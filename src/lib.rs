//! # Holochain Membrane (hc-membrane)
//!
//! Network edge gateway providing DHT access for lightweight browser clients.
//!
//! ## What is Holochain Membrane?
//!
//! Holochain Membrane is a network edge - like a cell membrane, it provides selective
//! access between lightweight browser clients and the Holochain DHT network.
//!
//! This is NOT a "lite conductor" - it has no:
//! - Source chain (no local chain storage)
//! - Validation (no validation workflows)
//! - Full node capabilities (zero-arc, no DHT storage)
//!
//! It provides:
//! - Holochain semantic API (/hc/*): get, get_links, publish, signals
//! - Kitsune direct API (/k2/*): network status, peer info, liveness

mod config;
mod error;
mod router;
mod service;

// Routes
pub mod routes;

pub use config::Configuration;
pub use error::{HcMembraneError, HcMembraneResult};
pub use router::create_router;
pub use service::HcMembraneService;

// Re-export common types
pub use holo_hash;
pub use holochain_types;
