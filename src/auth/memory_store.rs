//! In-memory implementation of [`SessionStore`].

use async_trait::async_trait;
use holochain_types::prelude::{AgentPubKey, DnaHash};
use std::collections::HashMap;
use tokio::sync::RwLock;

use super::session_store::SessionStore;
use super::types::{AllowedAgent, SessionInfo, SessionToken};

/// Stores agents and sessions in memory (no persistence across restarts).
#[derive(Debug)]
pub struct MemorySessionStore {
    inner: RwLock<MemoryStoreInner>,
}

#[derive(Debug, Default)]
struct MemoryStoreInner {
    allowed_agents: HashMap<AgentPubKey, AllowedAgent>,
    sessions: HashMap<String, SessionInfo>,
}

impl Default for MemorySessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemorySessionStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(MemoryStoreInner::default()),
        }
    }
}

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn add_agent(&self, agent: AllowedAgent) {
        let mut inner = self.inner.write().await;
        inner
            .allowed_agents
            .insert(agent.agent_pubkey.clone(), agent);
    }

    async fn remove_agent(&self, agent_pubkey: &AgentPubKey) -> bool {
        let mut inner = self.inner.write().await;
        let removed = inner.allowed_agents.remove(agent_pubkey).is_some();
        if removed {
            inner
                .sessions
                .retain(|_, s| &s.agent_pubkey != agent_pubkey);
        }
        removed
    }

    async fn list_agents(&self) -> Vec<AllowedAgent> {
        let inner = self.inner.read().await;
        inner.allowed_agents.values().cloned().collect()
    }

    async fn is_agent_allowed(&self, agent_pubkey: &AgentPubKey) -> bool {
        let inner = self.inner.read().await;
        inner.allowed_agents.contains_key(agent_pubkey)
    }

    async fn get_agent(&self, agent_pubkey: &AgentPubKey) -> Option<AllowedAgent> {
        let inner = self.inner.read().await;
        inner.allowed_agents.get(agent_pubkey).cloned()
    }

    async fn create_session(&self, agent_pubkey: &AgentPubKey) -> Option<SessionToken> {
        let inner = self.inner.read().await;
        let allowed = inner.allowed_agents.get(agent_pubkey)?;
        let capabilities = allowed.capabilities.clone();
        drop(inner);

        let token = SessionToken::generate();
        let info = SessionInfo {
            agent_pubkey: agent_pubkey.clone(),
            capabilities,
            registered_dnas: std::collections::HashSet::new(),
        };

        let mut inner = self.inner.write().await;
        inner.sessions.insert(token.0.clone(), info);
        Some(token)
    }

    async fn validate_session(&self, token: &str) -> Option<SessionInfo> {
        let inner = self.inner.read().await;
        inner.sessions.get(token).cloned()
    }

    async fn revoke_session(&self, token: &str) -> bool {
        let mut inner = self.inner.write().await;
        inner.sessions.remove(token).is_some()
    }

    async fn register_dna_for_agent(&self, agent_pubkey: &AgentPubKey, dna: &DnaHash) {
        let mut inner = self.inner.write().await;
        for session in inner.sessions.values_mut() {
            if &session.agent_pubkey == agent_pubkey {
                session.registered_dnas.insert(dna.clone());
            }
        }
    }

    async fn revoke_sessions_for_agent(&self, agent_pubkey: &AgentPubKey) -> usize {
        let mut inner = self.inner.write().await;
        let before = inner.sessions.len();
        inner
            .sessions
            .retain(|_, s| &s.agent_pubkey != agent_pubkey);
        before - inner.sessions.len()
    }

    async fn session_count(&self) -> usize {
        let inner = self.inner.read().await;
        inner.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    session_store_test_suite!(MemorySessionStore::new());
}
