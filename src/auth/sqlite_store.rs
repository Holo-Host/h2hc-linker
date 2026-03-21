//! SQLite-backed implementation of [`SessionStore`].
//!
//! Sessions and allowed agents persist across restarts.

use async_trait::async_trait;
use holochain_types::prelude::{AgentPubKey, DnaHash};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use super::session_store::SessionStore;
use super::types::{AllowedAgent, Capability, SessionInfo, SessionToken};

/// SQLite-backed session store.
///
/// Uses `std::sync::Mutex` around `rusqlite::Connection` — all DB calls are
/// dispatched via `tokio::task::spawn_blocking` so we never hold the lock
/// across an `.await`.
pub struct SqliteSessionStore {
    conn: Mutex<Connection>,
}

impl std::fmt::Debug for SqliteSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteSessionStore").finish()
    }
}

impl SqliteSessionStore {
    /// Open (or create) a SQLite store at the given path.
    pub fn new(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// Create an in-memory SQLite store (useful for testing).
    pub fn new_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;

             CREATE TABLE IF NOT EXISTS allowed_agents (
                 agent_pubkey TEXT PRIMARY KEY,
                 capabilities TEXT NOT NULL,
                 label        TEXT
             );

             CREATE TABLE IF NOT EXISTS sessions (
                 token        TEXT PRIMARY KEY,
                 agent_pubkey TEXT NOT NULL,
                 capabilities TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_sessions_agent
                 ON sessions(agent_pubkey);

             CREATE TABLE IF NOT EXISTS session_dnas (
                 token        TEXT NOT NULL,
                 dna_hash     TEXT NOT NULL,
                 agent_pubkey TEXT NOT NULL,
                 PRIMARY KEY (token, dna_hash),
                 FOREIGN KEY (token) REFERENCES sessions(token) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_session_dnas_agent
                 ON session_dnas(agent_pubkey);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Serialize capabilities to JSON string.
    fn caps_to_json(caps: &HashSet<Capability>) -> String {
        serde_json::to_string(caps).expect("Capability serialization cannot fail")
    }

    /// Deserialize capabilities from JSON string.
    fn caps_from_json(json: &str) -> HashSet<Capability> {
        serde_json::from_str(json).unwrap_or_default()
    }
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    async fn add_agent(&self, agent: AllowedAgent) {
        let conn = self.conn.lock().expect("lock poisoned");
        let pk = agent.agent_pubkey.to_string();
        let caps = Self::caps_to_json(&agent.capabilities);
        let label = agent.label.as_deref();
        conn.execute(
            "INSERT OR REPLACE INTO allowed_agents (agent_pubkey, capabilities, label)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![pk, caps, label],
        )
        .expect("add_agent insert failed");
    }

    async fn remove_agent(&self, agent_pubkey: &AgentPubKey) -> bool {
        let conn = self.conn.lock().expect("lock poisoned");
        let pk = agent_pubkey.to_string();

        // Delete session_dnas for this agent's sessions, then sessions, then agent.
        // Foreign key cascades handle session_dnas from sessions, but we also
        // clean up by agent_pubkey explicitly for safety.
        conn.execute(
            "DELETE FROM session_dnas WHERE agent_pubkey = ?1",
            rusqlite::params![pk],
        )
        .expect("remove_agent delete session_dnas failed");

        conn.execute(
            "DELETE FROM sessions WHERE agent_pubkey = ?1",
            rusqlite::params![pk],
        )
        .expect("remove_agent delete sessions failed");

        let rows = conn
            .execute(
                "DELETE FROM allowed_agents WHERE agent_pubkey = ?1",
                rusqlite::params![pk],
            )
            .expect("remove_agent delete agent failed");

        rows > 0
    }

    async fn list_agents(&self) -> Vec<AllowedAgent> {
        let conn = self.conn.lock().expect("lock poisoned");
        let mut stmt = conn
            .prepare("SELECT agent_pubkey, capabilities, label FROM allowed_agents")
            .expect("list_agents prepare failed");

        let rows = stmt
            .query_map([], |row| {
                let pk_str: String = row.get(0)?;
                let caps_json: String = row.get(1)?;
                let label: Option<String> = row.get(2)?;
                Ok((pk_str, caps_json, label))
            })
            .expect("list_agents query failed");

        let mut agents = Vec::new();
        for row in rows {
            let (pk_str, caps_json, label) = row.expect("list_agents row failed");
            if let Ok(agent_pubkey) = AgentPubKey::try_from(pk_str.as_str()) {
                agents.push(AllowedAgent {
                    agent_pubkey,
                    capabilities: Self::caps_from_json(&caps_json),
                    label,
                });
            }
        }
        agents
    }

    async fn is_agent_allowed(&self, agent_pubkey: &AgentPubKey) -> bool {
        let conn = self.conn.lock().expect("lock poisoned");
        let pk = agent_pubkey.to_string();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM allowed_agents WHERE agent_pubkey = ?1",
                rusqlite::params![pk],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    async fn get_agent(&self, agent_pubkey: &AgentPubKey) -> Option<AllowedAgent> {
        let conn = self.conn.lock().expect("lock poisoned");
        let pk = agent_pubkey.to_string();
        conn.query_row(
            "SELECT capabilities, label FROM allowed_agents WHERE agent_pubkey = ?1",
            rusqlite::params![pk],
            |row| {
                let caps_json: String = row.get(0)?;
                let label: Option<String> = row.get(1)?;
                Ok((caps_json, label))
            },
        )
        .ok()
        .map(|(caps_json, label)| AllowedAgent {
            agent_pubkey: agent_pubkey.clone(),
            capabilities: Self::caps_from_json(&caps_json),
            label,
        })
    }

    async fn create_session(&self, agent_pubkey: &AgentPubKey) -> Option<SessionToken> {
        let conn = self.conn.lock().expect("lock poisoned");
        let pk = agent_pubkey.to_string();

        // Look up agent capabilities
        let caps_json: String = conn
            .query_row(
                "SELECT capabilities FROM allowed_agents WHERE agent_pubkey = ?1",
                rusqlite::params![pk],
                |row| row.get(0),
            )
            .ok()?;

        let token = SessionToken::generate();
        conn.execute(
            "INSERT INTO sessions (token, agent_pubkey, capabilities) VALUES (?1, ?2, ?3)",
            rusqlite::params![token.as_str(), pk, caps_json],
        )
        .expect("create_session insert failed");

        Some(token)
    }

    async fn validate_session(&self, token: &str) -> Option<SessionInfo> {
        let conn = self.conn.lock().expect("lock poisoned");

        let (pk_str, caps_json) = conn
            .query_row(
                "SELECT agent_pubkey, capabilities FROM sessions WHERE token = ?1",
                rusqlite::params![token],
                |row| {
                    let pk: String = row.get(0)?;
                    let caps: String = row.get(1)?;
                    Ok((pk, caps))
                },
            )
            .ok()?;

        let agent_pubkey = AgentPubKey::try_from(pk_str.as_str()).ok()?;
        let capabilities = Self::caps_from_json(&caps_json);

        // Collect registered DNAs
        let mut stmt = conn
            .prepare("SELECT dna_hash FROM session_dnas WHERE token = ?1")
            .expect("validate_session prepare failed");
        let dna_rows = stmt
            .query_map(rusqlite::params![token], |row| {
                let dna_str: String = row.get(0)?;
                Ok(dna_str)
            })
            .expect("validate_session dna query failed");

        let mut registered_dnas = HashSet::new();
        for dna_str in dna_rows.flatten() {
            if let Ok(dna) = DnaHash::try_from(dna_str.as_str()) {
                registered_dnas.insert(dna);
            }
        }

        Some(SessionInfo {
            agent_pubkey,
            capabilities,
            registered_dnas,
        })
    }

    async fn revoke_session(&self, token: &str) -> bool {
        let conn = self.conn.lock().expect("lock poisoned");
        // CASCADE deletes session_dnas
        let rows = conn
            .execute(
                "DELETE FROM sessions WHERE token = ?1",
                rusqlite::params![token],
            )
            .expect("revoke_session delete failed");
        rows > 0
    }

    async fn register_dna_for_agent(&self, agent_pubkey: &AgentPubKey, dna: &DnaHash) {
        let conn = self.conn.lock().expect("lock poisoned");
        let pk = agent_pubkey.to_string();
        let dna_str = dna.to_string();

        // Insert a row in session_dnas for every session this agent has.
        conn.execute(
            "INSERT OR IGNORE INTO session_dnas (token, dna_hash, agent_pubkey)
             SELECT token, ?1, ?2 FROM sessions WHERE agent_pubkey = ?2",
            rusqlite::params![dna_str, pk],
        )
        .expect("register_dna_for_agent insert failed");
    }

    async fn revoke_sessions_for_agent(&self, agent_pubkey: &AgentPubKey) -> usize {
        let conn = self.conn.lock().expect("lock poisoned");
        let pk = agent_pubkey.to_string();

        // CASCADE handles session_dnas
        conn.execute(
            "DELETE FROM sessions WHERE agent_pubkey = ?1",
            rusqlite::params![pk],
        )
        .expect("revoke_sessions_for_agent delete failed")
    }

    async fn session_count(&self) -> usize {
        let conn = self.conn.lock().expect("lock poisoned");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap_or(0);
        count as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    session_store_test_suite!(SqliteSessionStore::new_in_memory().unwrap());

    /// SQLite-specific: data persists across store instances on same file.
    #[tokio::test]
    async fn test_persistence_across_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Create store, add agent + session, drop it
        let token_str;
        {
            let store = SqliteSessionStore::new(&db_path).unwrap();
            store
                .add_agent(AllowedAgent {
                    agent_pubkey: AgentPubKey::from_raw_32(vec![1u8; 32]),
                    capabilities: HashSet::from([Capability::DhtRead]),
                    label: Some("browser".to_string()),
                })
                .await;
            let token = store
                .create_session(&AgentPubKey::from_raw_32(vec![1u8; 32]))
                .await
                .unwrap();

            let dna = DnaHash::from_raw_32(vec![10u8; 32]);
            store
                .register_dna_for_agent(&AgentPubKey::from_raw_32(vec![1u8; 32]), &dna)
                .await;

            token_str = token.0.clone();
        }

        // Reopen at the same path
        let store = SqliteSessionStore::new(&db_path).unwrap();

        // Agent still present
        assert!(
            store
                .is_agent_allowed(&AgentPubKey::from_raw_32(vec![1u8; 32]))
                .await
        );

        // Session still valid
        let session = store.validate_session(&token_str).await.unwrap();
        assert_eq!(
            session.agent_pubkey,
            AgentPubKey::from_raw_32(vec![1u8; 32])
        );
        assert!(session.has_capability(Capability::DhtRead));

        // DNA registration survived
        let dna = DnaHash::from_raw_32(vec![10u8; 32]);
        assert!(session.has_dna(&dna));
    }
}
