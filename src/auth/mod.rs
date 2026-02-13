//! Authentication and authorization layer for hc-membrane.
//!
//! Gated on `HC_MEMBRANE_ADMIN_SECRET` -- when absent, all endpoints remain open.

pub mod admin;
pub mod middleware;
pub mod store;
pub mod types;

pub use store::AuthStore;
pub use types::{AllowedAgent, AuthContext, Capability, SessionInfo, SessionToken};
