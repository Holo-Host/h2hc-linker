//! Configuration for hc-membrane

use std::net::SocketAddr;
use std::time::Duration;

/// WebSocket configuration
#[derive(Debug, Clone)]
pub struct WebSocketConfig {
    /// Interval between heartbeat pings
    pub heartbeat_interval: Duration,
    /// Timeout for heartbeat responses
    pub heartbeat_timeout: Duration,
    /// Idle timeout for connections
    pub idle_timeout: Duration,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(30),
            heartbeat_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(300), // 5 minutes
        }
    }
}

/// Configuration for the Holochain Membrane gateway
#[derive(Debug, Clone)]
pub struct Configuration {
    /// Address of the Holochain admin WebSocket (for conductor integration during migration)
    pub admin_socket_addr: Option<SocketAddr>,

    /// Bootstrap server URL for Kitsune2
    pub bootstrap_url: Option<String>,

    /// WebRTC signal server URL for Kitsune2
    pub signal_url: Option<String>,

    /// Maximum payload size in bytes
    pub payload_limit_bytes: usize,

    /// WebSocket configuration
    pub websocket: WebSocketConfig,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            admin_socket_addr: None,
            bootstrap_url: None,
            signal_url: None,
            payload_limit_bytes: 10 * 1024 * 1024, // 10MB default
            websocket: WebSocketConfig::default(),
        }
    }
}

impl Configuration {
    /// Create a new configuration from environment variables
    pub fn from_env() -> anyhow::Result<Self> {
        let mut config = Self::default();

        // Optional admin WebSocket for migration period
        if let Ok(url) = std::env::var("HC_MEMBRANE_ADMIN_WS_URL") {
            config.admin_socket_addr = Some(url.parse()?);
        }

        // Kitsune2 configuration
        if let Ok(url) = std::env::var("HC_MEMBRANE_BOOTSTRAP_URL") {
            config.bootstrap_url = Some(url);
        }
        if let Ok(url) = std::env::var("HC_MEMBRANE_SIGNAL_URL") {
            config.signal_url = Some(url);
        }

        // Payload limit
        if let Ok(limit) = std::env::var("HC_MEMBRANE_PAYLOAD_LIMIT_BYTES") {
            config.payload_limit_bytes = limit.parse()?;
        }

        Ok(config)
    }

    /// Check if Kitsune2 is configured
    pub fn kitsune_enabled(&self) -> bool {
        self.bootstrap_url.is_some() && self.signal_url.is_some()
    }
}
