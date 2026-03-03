//! Configuration for h2hc-linker

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Configure Kitsune2 Reporting.
///
/// Matches the conductor's `ReportConfig` enum so the log-collector
/// can process linker and conductor reports identically.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportConfig {
    /// No reporting (default).
    #[default]
    None,

    /// Write daily-rotated JSONL report files (`hc-report.YYYY-MM-DD.jsonl`).
    JsonLines {
        /// How many days worth of report files to retain.
        days_retained: u32,

        /// How often to report Fetched-Op aggregated data in seconds.
        fetched_op_interval_s: u32,
    },
}

/// Default timeout for zome calls
pub const DEFAULT_ZOME_CALL_TIMEOUT: Duration = Duration::from_secs(10);

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

    /// Bootstrap server URL for Kitsune2 (required)
    pub bootstrap_url: String,

    /// Iroh relay server URL for Kitsune2
    pub relay_url: Option<String>,

    /// Maximum payload size in bytes
    pub payload_limit_bytes: usize,

    /// WebSocket configuration
    pub websocket: WebSocketConfig,

    /// Timeout for zome calls
    pub zome_call_timeout: Duration,

    /// Admin secret for authentication (from H2HC_LINKER_ADMIN_SECRET)
    /// When set, enables the auth layer.
    pub admin_secret: Option<String>,

    /// Session token TTL (from H2HC_LINKER_SESSION_TTL_SECS, default 3600)
    pub session_ttl: Duration,

    /// Kitsune2 report configuration (from H2HC_LINKER_REPORT)
    pub report: ReportConfig,

    /// Directory path for report files (from H2HC_LINKER_REPORT_PATH)
    pub report_path: PathBuf,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            admin_socket_addr: None,
            bootstrap_url: String::new(),
            relay_url: None,
            payload_limit_bytes: 10 * 1024 * 1024, // 10MB default
            websocket: WebSocketConfig::default(),
            zome_call_timeout: DEFAULT_ZOME_CALL_TIMEOUT,
            admin_secret: None,
            session_ttl: Duration::from_secs(3600),
            report: ReportConfig::None,
            report_path: PathBuf::from("/tmp/h2hc-linker-reports"),
        }
    }
}

impl Configuration {
    /// Create a new configuration from environment variables
    pub fn from_env() -> anyhow::Result<Self> {
        let mut config = Self::default();

        // Optional admin WebSocket for migration period
        if let Ok(url) = std::env::var("H2HC_LINKER_ADMIN_WS_URL") {
            config.admin_socket_addr = Some(url.parse()?);
        }

        // Kitsune2 configuration (bootstrap URL is required)
        match std::env::var("H2HC_LINKER_BOOTSTRAP_URL") {
            Ok(url) => config.bootstrap_url = url,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "H2HC_LINKER_BOOTSTRAP_URL is required. \
                     h2hc-linker cannot operate without kitsune2 networking. \
                     Set it to your bootstrap server URL (e.g. http://127.0.0.1:PORT)"
                ));
            }
        }
        if let Ok(url) = std::env::var("H2HC_LINKER_RELAY_URL") {
            config.relay_url = Some(url);
        }

        // Payload limit
        if let Ok(limit) = std::env::var("H2HC_LINKER_PAYLOAD_LIMIT_BYTES") {
            config.payload_limit_bytes = limit.parse()?;
        }

        // Zome call timeout
        if let Ok(timeout) = std::env::var("H2HC_LINKER_ZOME_CALL_TIMEOUT_MS") {
            config.zome_call_timeout = Duration::from_millis(timeout.parse()?);
        }

        // Report configuration
        if let Ok(report_type) = std::env::var("H2HC_LINKER_REPORT") {
            match report_type.to_lowercase().as_str() {
                "json_lines" | "jsonlines" | "jsonl" => {
                    let days_retained: u32 = std::env::var("H2HC_LINKER_REPORT_DAYS_RETAINED")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(5);
                    let fetched_op_interval_s: u32 = std::env::var("H2HC_LINKER_REPORT_INTERVAL_S")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(60);
                    config.report = ReportConfig::JsonLines {
                        days_retained,
                        fetched_op_interval_s,
                    };
                }
                "none" | "" => {}
                other => {
                    return Err(anyhow::anyhow!(
                        "Unknown H2HC_LINKER_REPORT value: '{}'. Use 'json_lines' or 'none'.",
                        other
                    ));
                }
            }
        }
        if let Ok(path) = std::env::var("H2HC_LINKER_REPORT_PATH") {
            config.report_path = PathBuf::from(path);
        }

        // Auth configuration
        if let Ok(secret) = std::env::var("H2HC_LINKER_ADMIN_SECRET") {
            config.admin_secret = Some(secret);
        }
        if let Ok(ttl) = std::env::var("H2HC_LINKER_SESSION_TTL_SECS") {
            config.session_ttl = Duration::from_secs(ttl.parse()?);
        }

        Ok(config)
    }

    /// Kitsune2 is always enabled (bootstrap URL is required at startup).
    /// This method exists for backwards compatibility with code that checks it.
    pub fn kitsune_enabled(&self) -> bool {
        true
    }

    /// Check if conductor integration is configured
    pub fn conductor_enabled(&self) -> bool {
        self.admin_socket_addr.is_some()
    }

    /// Check if authentication is enabled (admin secret is set)
    pub fn auth_enabled(&self) -> bool {
        self.admin_secret.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_auth_disabled() {
        let config = Configuration::default();
        assert!(!config.auth_enabled());
        assert!(config.admin_secret.is_none());
        assert_eq!(config.session_ttl, Duration::from_secs(3600));
    }

    #[test]
    fn test_auth_enabled_with_secret() {
        let mut config = Configuration::default();
        config.admin_secret = Some("test-secret".to_string());
        assert!(config.auth_enabled());
    }

    #[test]
    fn test_session_ttl_default() {
        let config = Configuration::default();
        assert_eq!(config.session_ttl, Duration::from_secs(3600));
    }
}
