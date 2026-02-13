//! Thread-safe authentication store.
//!
//! Manages allowed agents, sessions, and WS connection tracking.

use super::types::*;
use crate::agent_proxy::WsSender;
use holochain_types::prelude::AgentPubKey;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::debug;

/// The shared authentication store.
#[derive(Debug, Clone)]
pub struct AuthStore {
    inner: Arc<RwLock<AuthStoreInner>>,
    session_ttl: Duration,
}

#[derive(Debug, Default)]
struct AuthStoreInner {
    allowed_agents: HashMap<AgentPubKey, AllowedAgent>,
    sessions: HashMap<String, SessionInfo>,
    ws_senders: HashMap<AgentPubKey, Vec<WsSender>>,
}

impl AuthStore {
    pub fn new(session_ttl: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(AuthStoreInner::default())),
            session_ttl,
        }
    }

    // --- Agent management ---

    pub async fn add_agent(&self, agent: AllowedAgent) {
        let mut inner = self.inner.write().await;
        inner
            .allowed_agents
            .insert(agent.agent_pubkey.clone(), agent);
    }

    /// Remove an agent. Revokes all sessions and closes all WS connections.
    pub async fn remove_agent(&self, agent_pubkey: &AgentPubKey) -> bool {
        let mut inner = self.inner.write().await;
        let removed = inner.allowed_agents.remove(agent_pubkey).is_some();
        if removed {
            // Revoke all sessions for this agent
            inner
                .sessions
                .retain(|_, s| &s.agent_pubkey != agent_pubkey);
            // Drop all WS senders for this agent (closes the connections)
            inner.ws_senders.remove(agent_pubkey);
        }
        removed
    }

    pub async fn list_agents(&self) -> Vec<AllowedAgent> {
        let inner = self.inner.read().await;
        inner.allowed_agents.values().cloned().collect()
    }

    pub async fn is_agent_allowed(&self, agent_pubkey: &AgentPubKey) -> bool {
        let inner = self.inner.read().await;
        inner.allowed_agents.contains_key(agent_pubkey)
    }

    pub async fn get_agent(&self, agent_pubkey: &AgentPubKey) -> Option<AllowedAgent> {
        let inner = self.inner.read().await;
        inner.allowed_agents.get(agent_pubkey).cloned()
    }

    // --- Session management ---

    pub async fn create_session(&self, agent_pubkey: &AgentPubKey) -> Option<SessionToken> {
        let inner = self.inner.read().await;
        let allowed = inner.allowed_agents.get(agent_pubkey)?;
        let capabilities = allowed.capabilities.clone();
        drop(inner);

        let token = SessionToken::generate();
        let info = SessionInfo {
            agent_pubkey: agent_pubkey.clone(),
            capabilities,
            created_at: std::time::Instant::now(),
            ttl: self.session_ttl,
        };

        let mut inner = self.inner.write().await;
        inner.sessions.insert(token.0.clone(), info);
        Some(token)
    }

    pub async fn validate_session(&self, token: &str) -> Option<SessionInfo> {
        let inner = self.inner.read().await;
        let session = inner.sessions.get(token)?;
        if session.is_expired() {
            return None;
        }
        Some(session.clone())
    }

    pub async fn revoke_session(&self, token: &str) -> bool {
        let mut inner = self.inner.write().await;
        inner.sessions.remove(token).is_some()
    }

    /// Remove expired sessions. Returns count removed.
    pub async fn cleanup_expired_sessions(&self) -> usize {
        let mut inner = self.inner.write().await;
        let before = inner.sessions.len();
        inner.sessions.retain(|_, s| !s.is_expired());
        before - inner.sessions.len()
    }

    /// Start background cleanup task (every 60s).
    pub fn start_cleanup_task(&self) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let removed = store.cleanup_expired_sessions().await;
                if removed > 0 {
                    debug!("Cleaned up {} expired auth sessions", removed);
                }
            }
        });
    }

    // --- WS connection tracking ---

    /// Register a WS sender for an agent (called on successful WS auth).
    pub async fn register_ws_sender(&self, agent_pubkey: &AgentPubKey, sender: WsSender) {
        let mut inner = self.inner.write().await;
        inner
            .ws_senders
            .entry(agent_pubkey.clone())
            .or_default()
            .push(sender);
    }

    /// Unregister a WS sender for an agent (called on WS disconnect).
    /// Removes senders that are closed.
    pub async fn unregister_ws_sender(&self, agent_pubkey: &AgentPubKey, sender: &WsSender) {
        let mut inner = self.inner.write().await;
        if let Some(senders) = inner.ws_senders.get_mut(agent_pubkey) {
            // Remove the specific sender by comparing channel identity
            // mpsc::Sender has same_channel() for this
            senders.retain(|s| !s.same_channel(sender));
            if senders.is_empty() {
                inner.ws_senders.remove(agent_pubkey);
            }
        }
    }

    /// Get count of active WS connections for an agent.
    pub async fn ws_connection_count(&self, agent_pubkey: &AgentPubKey) -> usize {
        let inner = self.inner.read().await;
        inner.ws_senders.get(agent_pubkey).map_or(0, |v| v.len())
    }

    /// Get total session count (for testing/monitoring).
    pub async fn session_count(&self) -> usize {
        let inner = self.inner.read().await;
        inner.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn test_agent(seed: u8) -> AgentPubKey {
        AgentPubKey::from_raw_32(vec![seed; 32])
    }

    fn test_allowed_agent(seed: u8, caps: &[Capability]) -> AllowedAgent {
        AllowedAgent {
            agent_pubkey: test_agent(seed),
            capabilities: caps.iter().copied().collect(),
            label: None,
        }
    }

    #[tokio::test]
    async fn test_add_and_list_agents() {
        let store = AuthStore::new(Duration::from_secs(3600));
        assert!(store.list_agents().await.is_empty());

        store
            .add_agent(test_allowed_agent(1, &[Capability::DhtRead]))
            .await;
        store
            .add_agent(test_allowed_agent(
                2,
                &[Capability::DhtWrite, Capability::K2],
            ))
            .await;

        let agents = store.list_agents().await;
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn test_is_agent_allowed() {
        let store = AuthStore::new(Duration::from_secs(3600));
        store
            .add_agent(test_allowed_agent(1, &[Capability::DhtRead]))
            .await;

        assert!(store.is_agent_allowed(&test_agent(1)).await);
        assert!(!store.is_agent_allowed(&test_agent(2)).await);
    }

    #[tokio::test]
    async fn test_remove_agent() {
        let store = AuthStore::new(Duration::from_secs(3600));
        store
            .add_agent(test_allowed_agent(1, &[Capability::DhtRead]))
            .await;

        assert!(store.remove_agent(&test_agent(1)).await);
        assert!(!store.is_agent_allowed(&test_agent(1)).await);
        // Removing again returns false
        assert!(!store.remove_agent(&test_agent(1)).await);
    }

    #[tokio::test]
    async fn test_create_session_for_allowed_agent() {
        let store = AuthStore::new(Duration::from_secs(3600));
        store
            .add_agent(test_allowed_agent(
                1,
                &[Capability::DhtRead, Capability::K2],
            ))
            .await;

        let token = store.create_session(&test_agent(1)).await;
        assert!(token.is_some());

        let token = token.unwrap();
        let session = store.validate_session(token.as_str()).await;
        assert!(session.is_some());

        let session = session.unwrap();
        assert_eq!(session.agent_pubkey, test_agent(1));
        assert!(session.has_capability(Capability::DhtRead));
        assert!(session.has_capability(Capability::K2));
        assert!(!session.has_capability(Capability::DhtWrite));
    }

    #[tokio::test]
    async fn test_create_session_for_unknown_agent_returns_none() {
        let store = AuthStore::new(Duration::from_secs(3600));
        let token = store.create_session(&test_agent(99)).await;
        assert!(token.is_none());
    }

    #[tokio::test]
    async fn test_validate_expired_session_returns_none() {
        // Use a very short TTL
        let store = AuthStore::new(Duration::from_millis(1));
        store
            .add_agent(test_allowed_agent(1, &[Capability::DhtRead]))
            .await;

        let token = store.create_session(&test_agent(1)).await.unwrap();

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(10)).await;

        let session = store.validate_session(token.as_str()).await;
        assert!(session.is_none());
    }

    #[tokio::test]
    async fn test_validate_invalid_token_returns_none() {
        let store = AuthStore::new(Duration::from_secs(3600));
        let session = store.validate_session("bogus-token").await;
        assert!(session.is_none());
    }

    #[tokio::test]
    async fn test_revoke_session() {
        let store = AuthStore::new(Duration::from_secs(3600));
        store
            .add_agent(test_allowed_agent(1, &[Capability::DhtRead]))
            .await;

        let token = store.create_session(&test_agent(1)).await.unwrap();
        assert!(store.validate_session(token.as_str()).await.is_some());

        assert!(store.revoke_session(token.as_str()).await);
        assert!(store.validate_session(token.as_str()).await.is_none());

        // Revoking again returns false
        assert!(!store.revoke_session(token.as_str()).await);
    }

    #[tokio::test]
    async fn test_remove_agent_revokes_all_sessions() {
        let store = AuthStore::new(Duration::from_secs(3600));
        store
            .add_agent(test_allowed_agent(1, &[Capability::DhtRead]))
            .await;

        let token1 = store.create_session(&test_agent(1)).await.unwrap();
        let token2 = store.create_session(&test_agent(1)).await.unwrap();

        assert!(store.validate_session(token1.as_str()).await.is_some());
        assert!(store.validate_session(token2.as_str()).await.is_some());

        store.remove_agent(&test_agent(1)).await;

        assert!(store.validate_session(token1.as_str()).await.is_none());
        assert!(store.validate_session(token2.as_str()).await.is_none());
    }

    #[tokio::test]
    async fn test_cleanup_expired_sessions() {
        let store = AuthStore::new(Duration::from_millis(1));
        store
            .add_agent(test_allowed_agent(1, &[Capability::DhtRead]))
            .await;

        store.create_session(&test_agent(1)).await.unwrap();
        store.create_session(&test_agent(1)).await.unwrap();
        assert_eq!(store.session_count().await, 2);

        tokio::time::sleep(Duration::from_millis(10)).await;

        let removed = store.cleanup_expired_sessions().await;
        assert_eq!(removed, 2);
        assert_eq!(store.session_count().await, 0);
    }

    #[tokio::test]
    async fn test_ws_sender_register_and_unregister() {
        let store = AuthStore::new(Duration::from_secs(3600));
        let (tx, _rx) = mpsc::channel(1);

        store.register_ws_sender(&test_agent(1), tx.clone()).await;
        assert_eq!(store.ws_connection_count(&test_agent(1)).await, 1);

        store.unregister_ws_sender(&test_agent(1), &tx).await;
        assert_eq!(store.ws_connection_count(&test_agent(1)).await, 0);
    }

    #[tokio::test]
    async fn test_remove_agent_drops_ws_senders() {
        let store = AuthStore::new(Duration::from_secs(3600));
        store
            .add_agent(test_allowed_agent(1, &[Capability::DhtRead]))
            .await;

        let (tx, mut rx) = mpsc::channel(1);
        store.register_ws_sender(&test_agent(1), tx.clone()).await;
        assert_eq!(store.ws_connection_count(&test_agent(1)).await, 1);

        store.remove_agent(&test_agent(1)).await;
        assert_eq!(store.ws_connection_count(&test_agent(1)).await, 0);

        // The sender was dropped, so trying to send should fail
        // (well, tx is still alive since we cloned it above, but the store's copy is gone)
        // What matters is the store no longer tracks it
    }

    #[tokio::test]
    async fn test_multiple_ws_connections_per_agent() {
        let store = AuthStore::new(Duration::from_secs(3600));
        let (tx1, _rx1) = mpsc::channel(1);
        let (tx2, _rx2) = mpsc::channel(1);

        store.register_ws_sender(&test_agent(1), tx1.clone()).await;
        store.register_ws_sender(&test_agent(1), tx2.clone()).await;
        assert_eq!(store.ws_connection_count(&test_agent(1)).await, 2);

        // Unregister one
        store.unregister_ws_sender(&test_agent(1), &tx1).await;
        assert_eq!(store.ws_connection_count(&test_agent(1)).await, 1);
    }
}
