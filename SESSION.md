# Current Session

**Last Updated**: 2026-02-13
**Current Step**: M5 - Authentication Layer
**Branch**: `step-m5-auth`

---

## Summary

Adding an authentication layer to hc-membrane, gated on `HC_MEMBRANE_ADMIN_SECRET` env var. When absent, all endpoints remain open (backwards compatible). When set, enables admin API for agent management, WS challenge-response auth, and session-based HTTP auth with per-agent capabilities.

---

## Current Status

| Feature | Status | Notes |
|---------|--------|-------|
| Auth types | ✅ Done | `Capability`, `AllowedAgent`, `SessionToken`, `SessionInfo`, `AuthContext` |
| Auth store | ✅ Done | Thread-safe store with agent/session/WS management |
| Config | ✅ Done | `admin_secret`, `session_ttl`, `auth_enabled()` |
| Error types | ✅ Done | `Forbidden(String)` → 403 |
| Service | ✅ Done | `auth_store: Option<AuthStore>` in AppState |
| Middleware | ✅ Done | `require_dht_read`, `require_dht_write`, `require_k2`, `require_admin_secret` |
| Admin API | ✅ Done | `POST/DELETE/GET /admin/agents` |
| Router | ✅ Done | Conditional middleware (open vs authenticated) |
| WS auth | ✅ Done | Challenge-response with ed25519 signature verification |
| Unit tests | ✅ 84 passing | All tests pass |

---

## Files Created/Modified

| File | Action |
|------|--------|
| `src/auth/mod.rs` | Created - module root |
| `src/auth/types.rs` | Created - auth data structures (8 tests) |
| `src/auth/store.rs` | Created - thread-safe auth store (13 tests) |
| `src/auth/middleware.rs` | Created - axum middleware (8 tests) |
| `src/auth/admin.rs` | Created - admin API handlers (5 tests) |
| `src/lib.rs` | Modified - added `pub mod auth` |
| `src/config.rs` | Modified - added `admin_secret`, `session_ttl` (3 tests) |
| `src/error.rs` | Modified - added `Forbidden` variant |
| `src/service.rs` | Modified - added `auth_store` to AppState |
| `src/router.rs` | Modified - conditional auth middleware + admin routes |
| `src/routes/websocket.rs` | Modified - challenge-response auth flow (3 new tests) |
| `STEPS/M5_PLAN.md` | Created - step plan |
| `STEPS/index.md` | Modified - added M5, renumbered M6-M8 |

---

## Test Commands

```bash
# Build hc-membrane
nix develop --command cargo build

# Run unit tests
nix develop --command cargo test

# Run with auth enabled
HC_MEMBRANE_ADMIN_SECRET=test-secret nix develop --command cargo run -- --port 8090

# Add an agent
curl -X POST http://localhost:8090/admin/agents \
  -H "Authorization: Bearer test-secret" \
  -H "Content-Type: application/json" \
  -d '{"agent_pubkey":"uhCAk...","capabilities":["dht_read","k2"]}'

# Run without auth (backwards compatible)
nix develop --command cargo run -- --port 8090
```

---

## Next Steps

1. E2E testing with fishy extension (WS auth flow)
2. Update fishy extension to use new WS auth protocol
3. Commit all changes

---

## Quick Links

- [M5 Plan](./STEPS/M5_PLAN.md) - Auth layer plan
- [Step Registry](./STEPS/index.md) - All step statuses
- [Architecture](./ARCHITECTURE.md) - System architecture
