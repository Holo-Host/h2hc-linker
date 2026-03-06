# Step Status Registry

> **Purpose**: Single source of truth for step completion status. Update this file when steps are completed.

## Quick Reference

| Step | Status | Description |
|------|--------|-------------|
| M1 | ✅ | Create repository skeleton |
| M2a | ✅ | WebSocket + Agent Registration |
| M2b | ✅ | Signal Forwarding |
| M2c | ✅ | DHT Read Endpoints |
| M2d | ✅ | DHT Publish Endpoint |
| M2e | ✅ | Zome Call Endpoint |
| M3 | ✅ | Add Kitsune liveness endpoints |
| M4 | ✅ | Direct DHT Operations via Kitsune2 (upgraded to 0.4.0-dev.2 + iroh) |
| M4.1 | ✅ | Preflight Agent Info + E2E Validation |
| M5 | ✅ | Authentication Layer |
| M5.1 | ✅ | Post-merge features (agent activity, transparent signing, reporting, rename) |
| M6 | 📋 | Migrate op construction to gateway |
| M7 | 📋 | Remove conductor dependency |
| M8 | 📋 | Deprecate hc-http-gw-fork |

**Legend**: ✅ Complete | ⏳ In Progress | 📋 Planned | ❌ Blocked

---

## Migration Steps (from hc-http-gw-fork)

### Step M1: Create repository skeleton
**Status**: ✅ Complete

- ✅ Initialized repo with Cargo.toml and workspace structure
- ✅ Created src/{lib.rs, config.rs, error.rs, router.rs, service.rs}
- ✅ Set up routes/{mod.rs, health.rs, kitsune.rs}
- ✅ Added binary at src/bin/h2hc-linker.rs
- ✅ Added all necessary dependencies (kitsune2, holochain_types, etc.)
- holo-web-conductor extension continues using hc-http-gw-fork during transition

### Step M2a: WebSocket + Agent Registration
**Status**: ✅ Complete
**Plan**: [M2a_PLAN.md](./M2a_PLAN.md) | **Completion**: [M2a_COMPLETION.md](./M2a_COMPLETION.md)

- ✅ WebSocket endpoint at `/ws`
- ✅ AgentProxyManager for connection tracking
- ✅ ProxyAgent (LocalAgent impl with remote signing)
- ✅ GatewayKitsune for space/agent lifecycle
- ✅ Test infrastructure (flake.nix, e2e test scripts)
- ✅ **Test**: Registered agents visible in conductor peer store

### Step M2b: Signal Forwarding
**Status**: ✅ Complete
**Completion**: [M2b_COMPLETION.md](./M2b_COMPLETION.md)

- ✅ ProxySpaceHandler.recv_notify() decodes WireMessage::RemoteSignalEvt
- ✅ Forward signals to browser via WebSocket (AgentProxyManager.send_signal)
- ✅ Added /test/signal endpoint for testing without kitsune2
- ✅ **Test**: 32 unit tests passing (4 new signal forwarding tests)

### Step M2c: DHT Read Endpoints
**Status**: ✅ Complete
**Completion**: [M2c_COMPLETION.md](./M2c_COMPLETION.md)

- ✅ GET /dht/{dna_hash}/record/{hash} - Get record by hash
- ✅ GET /dht/{dna_hash}/links - Get links from base hash
- ✅ Conductor connection module (AdminConn, AppConn)
- ✅ Simplified: no allowed_app_ids filtering (all apps allowed)
- ✅ **Test**: 32 unit tests passing

### Step M2d: DHT Publish Endpoint
**Status**: ✅ Complete
**Completion**: [M2d_COMPLETION.md](./M2d_COMPLETION.md)

- ✅ POST /dht/{dna_hash}/publish - Publish signed DhtOps
- ✅ TempOpStore for temporary op storage (60s TTL)
- ✅ GatewayKitsune.publish_ops() for kitsune2 publishing
- ✅ **Test**: 37 unit tests passing (5 new publish/op_store tests)

### Step M2e: Zome Call Endpoint
**Status**: ✅ Complete
**Completion**: [M2e_COMPLETION.md](./M2e_COMPLETION.md)

- ✅ GET /api/{dna_hash}/{zome_name}/{fn_name} - Call zome function
- ✅ AppConn.call_zome() for general zome calls
- ✅ Base64 URL-safe JSON payload encoding
- ✅ **Test**: 42 unit tests passing (5 new zome call tests)

### Step M3: Add Kitsune liveness endpoints
**Status**: ✅ Complete
**Enables**: holo-web-conductor Step 14 liveness UI (once M2 provides DHT operations)

Kitsune Direct API endpoints for network status:
- ✅ GET /k2/status - overall network connection status
- ✅ GET /k2/peers - list all known peers across spaces
- ✅ GET /k2/space/{space_id}/status - space-specific status
- ✅ GET /k2/space/{space_id}/peers - list peers in a space
- ✅ GET /k2/space/{space_id}/local-agents - list local agents in a space
- ✅ GET /k2/transport/stats - network transport stats

Implementation:
- ✅ Created `kitsune.rs` with `KitsuneBuilder` and `MinimalKitsuneHandler`
- ✅ Wired Kitsune2 instance to `KitsuneState` in `service.rs`
- ✅ Endpoints return real data when Kitsune is configured

### Step M4: Direct DHT Operations via Kitsune2
**Status**: ✅ Complete (upgraded to kitsune2 0.4.0-dev.2 + iroh transport)
**Details**: [M4_STATUS.md](./M4_STATUS.md)

Direct wire protocol working with kitsune2 0.4.0-dev.2:

- ✅ Upgraded to kitsune2 0.4.0-dev.2 (matching Holochain 0.6.1-rc.0)
- ✅ Switched from tx5/webrtc to iroh transport
- ✅ Created `DhtQuery` module (`src/dht_query.rs`)
  - `PendingDhtResponses` for shared response routing
  - `DhtQuery.get()` and `DhtQuery.get_links()` via wire protocol
  - Parallel peer querying with first-non-empty-response selection
- ✅ Updated `ProxySpaceHandler` to route GetRes/GetLinksRes/ErrorRes
- ✅ Added `conductor-dht` feature flag for M2 compatibility
- ✅ Feature-flagged DHT route implementations
- ✅ 44 unit tests passing
- ✅ Both build modes compile

### Step M4.1: Preflight Agent Info + E2E Validation
**Status**: ✅ Complete

- ✅ Added `PreflightCache` (`src/wire_preflight.rs`)
- ✅ Added `BootstrapWrapperFactory` to intercept `Bootstrap::put()` calls
- ✅ Integrated `preflight_cache` into `KitsuneProxy`
- ✅ Gateway exchanges preflights with conductors
- ✅ Conductors grant access to gateway URLs

### Step M5: Authentication Layer
**Status**: ✅ Complete
**Plan**: [M5_PLAN.md](./M5_PLAN.md)

Auth layer gated on `H2HC_LINKER_ADMIN_SECRET` env var. When absent, all endpoints remain open (backwards compatible).

- ✅ Auth types (`Capability`, `AllowedAgent`, `SessionToken`, `SessionInfo`, `AuthContext`)
- ✅ Auth store (thread-safe store with agent/session/WS management)
- ✅ Config (`admin_secret`, `session_ttl`, `auth_enabled()`)
- ✅ Error types (`Forbidden(String)` → 403)
- ✅ Middleware (`require_dht_read`, `require_dht_write`, `require_k2`, `require_admin_secret`)
- ✅ Admin API (`POST/DELETE/GET /admin/agents`)
- ✅ Router (conditional middleware: open vs authenticated)
- ✅ WS challenge-response with ed25519 signature verification
- ✅ 84 unit tests passing
- ✅ Validation Op fixtures (JS-to-Rust cross-deserialization test vectors)

### Step M5.1: Post-merge features
**Status**: ✅ Complete

Features added after M5 merge into main:

- ✅ `get_agent_activity` and `must_get_agent_activity` endpoints
- ✅ Transparent signing protocol for agent info
- ✅ WireOps → flat Record format conversion in get endpoint
- ✅ CI: GitHub Actions workflow, release workflow (cross-platform binaries)
- ✅ Kitsune2 usage reporting (`hc-report` JSONL via `linker_report.rs`)
- ✅ Renamed from hc-membrane to h2hc-linker throughout codebase
- ✅ Base64 strings for agent_pubkey in admin API
- ✅ CAL License added
- ✅ `count_links` endpoint

### Step M6: Migrate op construction to gateway
**Status**: 📋 Planned

- Add produce_ops_from_record in h2hc-linker
- Update POST /dht/{dna}/publish to accept Record
- holo-web-conductor extension sends Records instead of ops
- Keep old ops endpoint for backwards compat
- **Test**: ziptest passes, publishing verified

### Step M7: Remove conductor dependency
**Status**: 📋 Planned

- Remove dht_util zome routing
- Remove AppConnPool
- h2hc-linker is standalone Kitsune2 peer
- **Test**: ziptest passes against h2hc-linker only

### Step M8: Deprecate hc-http-gw-fork
**Status**: 📋 Planned

- Update holo-web-conductor to require h2hc-linker
- Archive hc-http-gw-fork repo
- **Test**: Full integration test suite

---

## Feature Phases (Parallel with Migration)

| Phase | Focus | Risk | Corresponding Steps |
|-------|-------|------|---------------------|
| 1 | Kitsune liveness API | Low | M3 |
| 2 | RPC unification | Low | Future |
| 3 | holochain_p2p integration | Medium | M4, M5 |
| 4 | Remove conductor | Medium | M6 |
| 5 | Optimization | Low | Future |

---

## Testing Strategy

Each migration step must pass integration tests with **holo-web-conductor browser extension** and **ziptest hApp**.

### Test Levels

1. **Unit tests** - `nix develop --command cargo test` in h2hc-linker
2. **Integration tests** - `../holo-web-conductor && npm run test:integration`
3. **E2E tests** - holo-web-conductor extension + ziptest full flow
4. **Regression check** - Compare behavior with previous step

### Test Commands

```bash
# 1. Build and test h2hc-linker (always use nix develop)
nix develop --command cargo test
nix develop --command cargo build --release

# 2. Run e2e setup (uses hc-http-gw-fork initially, will switch to h2hc-linker)
cd ../holo-web-conductor && ./scripts/e2e-test-setup.sh start --happ=ziptest

# 3. Run holo-web-conductor integration tests
cd ../holo-web-conductor && npm run test:integration
```

### e2e-test-setup.sh Adaptation Plan

The holo-web-conductor test script `../holo-web-conductor/scripts/e2e-test-setup.sh` needs to be updated to support h2hc-linker:

- **Step M2**: Add `--gateway=membrane` flag to switch between gateways
- **Step M6**: Default to h2hc-linker
- **Step M7**: Remove hc-http-gw-fork support

### Test Fixtures

- **ziptest.happ**: `../holo-web-conductor/fixtures/ziptest.happ`
- **fixture1.happ**: `../hc-http-gw-fork/fixture/package/happ1/fixture1.happ`
- **E2E test page**: `../holo-web-conductor/packages/extension/test/e2e-gateway-test.html`

---

## Documentation Files

| File | Purpose |
|------|---------|
| `../CLAUDE.md` | Core rules and quick context |
| `../ARCHITECTURE.md` | System architecture diagram |
| `index.md` | This file - step registry |
| `X_PLAN.md` | Detailed plan for step X |
| `X_COMPLETION.md` | Completion notes for step X |
| `GATEWAY_ARCHITECTURE_ANALYSIS.md` | Detailed architecture analysis |
