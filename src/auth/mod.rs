//! Authentication and authorization layer for h2hc-linker.
//!
//! Gated on `H2HC_LINKER_ADMIN_SECRET` -- when absent, all endpoints remain open.

pub mod admin;
pub mod middleware;
pub mod store;
pub mod types;

pub use store::AuthStore;
pub use types::{AllowedAgent, AuthContext, Capability, SessionInfo, SessionToken};
