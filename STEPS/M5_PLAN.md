# Step M5: Authentication Layer for hc-membrane

**Branch**: `step-m5-auth`

## Context

All hc-membrane endpoints are currently open. The WebSocket `Auth` message accepts any token without validation. There is no way for an operator to control who can connect or what they can do. This step adds an auth layer gated on the presence of `HC_MEMBRANE_ADMIN_SECRET` -- when absent, all endpoints remain open (current behavior, backwards compatible).

## Design Decisions (from user)

- **Agent management**: Admin API protected by shared secret env var
- **HTTP sessions**: WS-issued session token (challenge-response creates token, HTTP uses it as Bearer)
- **Identity proof**: Cryptographic challenge (sign random nonce with ed25519 key)
- **Admin protection**: Shared secret from `HC_MEMBRANE_ADMIN_SECRET`
- **No WS re-auth**: Once a WS connection is authenticated, it stays authenticated for its lifetime. No need to re-auth over an open WS.
- **Agent removal drops WS**: When an agent is removed via admin API, forcefully close all open WS connections for that agent and revoke all sessions.

## Methodology

**TDD**: For each implementation sub-step, write tests first, confirm they fail, then implement to make them pass.

## Capabilities

Per-agent capability set, any combination of:
- `dht_read` -- GET `/dht/*/record`, `/dht/*/details`, `/dht/*/links`
- `dht_write` -- POST `/dht/*/publish`
- `k2` -- GET `/k2/*`

Always open: `/health`, `/ws` (auth inside WS protocol), `/api/*` (deprecated, skip)

## WS Auth Flow

```
Client                              Server
  |  {"type":"auth",                   |
  |   "agent_pubkey":"uhCAk..."}       |
  |  --------------------------------> |  check allowed list
  |                                    |  generate 32-byte nonce
  |  {"type":"auth_challenge",         |
  |   "challenge":"a1b2c3...hex"}      |
  |  <-------------------------------- |
  |  sign(challenge) with ed25519      |
  |  {"type":"auth_challenge_response",|
  |   "signature":"...base64 64B..."}  |
  |  --------------------------------> |  verify sig with agent pubkey
  |                                    |  create session in AuthStore
  |  {"type":"auth_ok",                |
  |   "session_token":"deadbeef..."}   |
  |  <-------------------------------- |
```

When auth disabled: `Auth` message immediately returns `AuthOk` with empty token (current behavior).

Session token is then usable as `Authorization: Bearer <token>` on HTTP endpoints.

## Implementation Plan (TDD order)

### 1. Create `src/auth/types.rs` -- data structures

**Tests first**: serialization roundtrips for `Capability`, `AllowedAgent`

- `Capability` enum: `DhtRead`, `DhtWrite`, `K2` (serde rename_all snake_case)
- `AllowedAgent`: `agent_pubkey: AgentPubKey`, `capabilities: HashSet<Capability>`, `label: Option<String>`
- `SessionToken`: newtype over String, `generate()` creates 32 random bytes hex-encoded
- `SessionInfo`: `agent_pubkey`, `capabilities`, `created_at: Instant`, `ttl: Duration`, `is_expired()`, `has_capability()`
- `AuthContext`: injected into axum request extensions by middleware

### 2. Create `src/auth/store.rs` -- thread-safe auth store

**Tests first**: add/remove agents, session lifecycle, agent-removal-revokes-sessions, expiration cleanup, agent-removal-signals-ws-disconnect

`AuthStore` wrapping `Arc<RwLock<AuthStoreInner>>`:
- `allowed_agents: HashMap<AgentPubKey, AllowedAgent>`
- `sessions: HashMap<String, SessionInfo>`
- `ws_senders: HashMap<AgentPubKey, Vec<WsSender>>` -- track WS connections per agent for forced disconnect

Methods:
- `add_agent()`, `remove_agent()` (revokes sessions AND closes WS connections), `list_agents()`, `is_agent_allowed()`
- `create_session()`, `validate_session()`, `revoke_session()`
- `register_ws_sender()`, `unregister_ws_sender()` -- called by WS handler on connect/disconnect
- `cleanup_expired_sessions()`, `start_cleanup_task()` (every 60s)

On `remove_agent()`:
1. Remove from allowed_agents
2. Remove all sessions for that agent
3. Close all WS connections for that agent (drop senders, which causes receiver end to see closed)

### 3. Create `src/auth/mod.rs` + modify `src/lib.rs`

Module root with re-exports. Add `pub mod auth` to lib.rs.

### 4. Modify `src/config.rs`

**Tests first**: config parsing with new env vars

Add fields to `Configuration`:
- `admin_secret: Option<String>` from `HC_MEMBRANE_ADMIN_SECRET`
- `session_ttl: Duration` from `HC_MEMBRANE_SESSION_TTL_SECS` (default 3600)
- `auth_enabled() -> bool` helper

### 5. Modify `src/error.rs`

Add `Forbidden(String)` variant mapping to 403 status code.

### 6. Modify `src/service.rs`

Add `auth_store: Option<AuthStore>` to `AppState`. Initialize when `config.auth_enabled()`.

### 7. Create `src/auth/middleware.rs` -- axum middleware

**Tests first**: missing header returns 401, invalid token returns 401, expired token returns 401, wrong capability returns 403, valid token passes through with AuthContext injected

Helper `extract_bearer_token(req) -> Option<&str>`.

Generic `check_capability(auth_store, req, cap) -> Result<(), Response>`:
- Extract Bearer token or 401
- Validate session or 401
- Check capability or 403
- Inject `AuthContext` into request extensions

Middleware fns (all take `State(state): State<AppState>`):
- `require_dht_read`, `require_dht_write`, `require_k2`
- `require_admin_secret` (compares Bearer against config)

### 8. Create `src/auth/admin.rs` -- admin API handlers

**Tests first**: request deserialization, add/remove/list agent round-trip

Handlers take `State(state): State<AppState>`:
- `POST /admin/agents` -- add/update allowed agent
- `DELETE /admin/agents` -- remove agent + revoke sessions + close WS connections
- `GET /admin/agents` -- list all allowed agents

### 9. Modify `src/router.rs` -- conditional middleware

If `app_state.auth_store.is_some()`:
- Group DHT read routes with `require_dht_read` route_layer
- Group DHT write routes with `require_dht_write` route_layer
- Group K2 routes with `require_k2` route_layer
- Add admin routes with `require_admin_secret` route_layer
- `/health`, `/ws`, `/test/signal` unprotected

If `app_state.auth_store.is_none()`:
- Build router exactly as today

### 10. Modify `src/routes/websocket.rs` -- challenge-response

**Tests first**: new ClientMessage/ServerMessage serde, challenge-response logic, auth-disabled auto-accept

**ClientMessage changes:**
- `Auth { session_token }` becomes `Auth { agent_pubkey }` (breaking WS protocol change)
- Add `AuthChallengeResponse { signature }` variant

**ServerMessage changes:**
- `AuthOk` (unit) becomes `AuthOk { session_token }` (breaking serialization change)
- Add `AuthChallenge { challenge }` variant

**ConnectionState additions:**
- `pending_auth_agent: Option<AgentPubKey>`
- `pending_challenge: Option<Vec<u8>>`
- `authenticated_agent: Option<AgentPubKey>`

**Auth handler:**
- If auth disabled: auto-accept, return `AuthOk { session_token: "" }`
- Parse agent_pubkey, check allowed list, generate 32-byte nonce, return `AuthChallenge`

**AuthChallengeResponse handler:**
- Verify signature with `ed25519_dalek::VerifyingKey` using `agent.get_raw_32()` (32 core bytes from holo_hash)
- On success: `auth_store.create_session()`, register WS sender with auth_store, return `AuthOk { session_token }`

**Register handler** (when auth enabled):
- Verify registered agent matches authenticated agent (prevent impersonation)

**Cleanup on disconnect:**
- `auth_store.unregister_ws_sender()` for the authenticated agent

### 11. Update step documentation

- Create `STEPS/M5_PLAN.md` with this plan
- Update `STEPS/index.md` to show M5 in progress (rename current M5 to M6, etc.)
- Update `SESSION.md`

## Files Summary

| File | Action |
|------|--------|
| `src/auth/mod.rs` | Create |
| `src/auth/types.rs` | Create |
| `src/auth/store.rs` | Create |
| `src/auth/middleware.rs` | Create |
| `src/auth/admin.rs` | Create |
| `src/config.rs` | Modify (add admin_secret, session_ttl) |
| `src/error.rs` | Modify (add Forbidden variant) |
| `src/service.rs` | Modify (add auth_store to AppState) |
| `src/lib.rs` | Modify (add auth module) |
| `src/router.rs` | Modify (conditional middleware, admin routes) |
| `src/routes/websocket.rs` | Modify (challenge-response flow) |
| `STEPS/M5_PLAN.md` | Create |
| `STEPS/index.md` | Modify (renumber steps) |
| `SESSION.md` | Modify |

## Dependencies

All already in Cargo.toml: `ed25519-dalek = "2"`, `rand = "0.9"`, `hex = "0.4"`

## Edge Cases

- **Token expires during WS session**: WS connection stays alive (already authenticated). Token expiry only affects HTTP requests. Client reconnects WS to get a new token.
- **Multiple WS per agent**: Each gets its own session token. All coexist.
- **Agent removed via admin**: All sessions revoked, all WS connections forcefully closed. Immediate effect.
- **Challenge replay**: Challenge stored per-connection, consumed on response (`take()`). Cannot be replayed.

## Verification

```bash
# Build
nix develop --command cargo build

# Unit tests (should be extensive given TDD approach)
nix develop --command cargo test

# Manual test with auth enabled
HC_MEMBRANE_ADMIN_SECRET=test-secret nix develop --command cargo run -- --port 8090

# Add agent
curl -X POST http://localhost:8090/admin/agents \
  -H "Authorization: Bearer test-secret" \
  -H "Content-Type: application/json" \
  -d '{"agent_pubkey":"uhCAk...","capabilities":["dht_read","k2"]}'

# Verify 401 without token
curl http://localhost:8090/dht/uhC0k.../links?base=uhCAk...

# Verify auth disabled mode
nix develop --command cargo run -- --port 8090
curl http://localhost:8090/dht/uhC0k.../links?base=uhCAk...
# Works as before
```
