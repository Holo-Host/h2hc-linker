# Plan: M2a - Agent Registration via WebSocket

## Goal
Fishy extension can register agents via hc-membrane WebSocket, and those agents become visible in conductor agent_infos. This is a minimal test that proves kitsune2 integration works without needing DHT read/write endpoints.

## Current State
- M3 complete (uncommitted): Kitsune liveness endpoints + `KitsuneBuilder` with `MinimalKitsuneHandler`
- Missing: WebSocket endpoint, agent registration, GatewayKitsune, AgentProxyManager

## What to Copy from hc-http-gw-fork

The code is well-structured and can be copied with minimal adaptation:

| Source File | Target File | What It Does |
|-------------|-------------|--------------|
| `agent_proxy.rs` | `src/agent_proxy.rs` | WebSocket connection tracking, remote signing |
| `proxy_agent.rs` | `src/proxy_agent.rs` | LocalAgent impl with remote signing |
| `kitsune_proxy.rs` | `src/gateway_kitsune.rs` | GatewayKitsune (space/agent lifecycle), KitsuneProxy handler |
| `wire_preflight.rs` | `src/wire_preflight.rs` | Preflight message format for holochain compat |
| `routes/websocket.rs` | `src/routes/websocket.rs` | WebSocket handler (auth, register, unregister) |

## Implementation Steps

### 1. Copy core modules (minimal changes needed)
- `agent_proxy.rs` - Copy as-is, update imports
- `proxy_agent.rs` - Copy as-is, update imports
- `wire_preflight.rs` - Copy as-is

### 2. Create gateway_kitsune.rs
Extract from `kitsune_proxy.rs`:
- `KitsuneProxy` struct (KitsuneHandler impl)
- `ProxySpaceHandler` struct (SpaceHandler impl)
- `KitsuneProxyBuilder` struct
- `GatewayKitsune` struct (agent join/leave lifecycle)

### 3. Update kitsune.rs
Replace `MinimalKitsuneHandler` with `KitsuneProxy` from gateway_kitsune.rs

### 4. Create routes/websocket.rs
Copy WebSocket handler with:
- `ClientMessage` enum (Auth, Register, Unregister, Ping, SignResponse)
- `ServerMessage` enum (AuthOk, Registered, Signal, SignRequest, etc.)
- `ws_handler` and `handle_socket` functions
- Remove `SendRemoteSignal` for now (add in M2b)

### 5. Update service.rs
Add to `HcMembraneService`:
- `AgentProxyManager` field
- `GatewayKitsune` field (Optional)
- Initialize both when kitsune is enabled

### 6. Update router.rs
Add WebSocket route: `.route("/ws", get(ws_handler))`

### 7. Update config.rs
Add `WebSocketConfig` struct with heartbeat/timeout settings

### 8. Create test script
`scripts/e2e-test-membrane.sh`:
```bash
#!/bin/bash
# 1. Start bootstrap server
# 2. Start conductor(s) with ziptest
# 3. Build and start hc-membrane
# 4. Test WebSocket registration
# 5. Query conductor agent_info to verify
```

## Files to Modify

```
src/
  lib.rs              # Add: mod agent_proxy, proxy_agent, gateway_kitsune, wire_preflight
  config.rs           # Add: WebSocketConfig
  service.rs          # Add: AgentProxyManager, GatewayKitsune initialization
  router.rs           # Add: /ws route
  kitsune.rs          # Replace MinimalKitsuneHandler with KitsuneProxy
  agent_proxy.rs      # NEW - copy from hc-http-gw-fork
  proxy_agent.rs      # NEW - copy from hc-http-gw-fork
  gateway_kitsune.rs  # NEW - extract from kitsune_proxy.rs
  wire_preflight.rs   # NEW - copy from hc-http-gw-fork
  routes/
    mod.rs            # Add: pub mod websocket
    websocket.rs      # NEW - copy from hc-http-gw-fork

scripts/
  e2e-test-membrane.sh  # NEW - test script
```

## Dependencies to Add

```toml
# Already have most - just need to verify:
futures = "0.3"  # For StreamExt, SinkExt in websocket
```

## Test Verification

```bash
# 1. Start infrastructure
./scripts/e2e-test-membrane.sh start

# 2. Test WebSocket (using wscat or test-gateway-websocket.mjs)
wscat -c ws://localhost:8090/ws
> {"type":"auth","session_token":""}
< {"type":"auth_ok"}
> {"type":"register","dna_hash":"uhC0k...","agent_pubkey":"uhCAk..."}
< {"type":"registered","dna_hash":"uhC0k...","agent_pubkey":"uhCAk..."}

# 3. Verify in conductor
hc sandbox call --running=8888 agent_info
# Should show the registered agent pubkey
```

## Success Criteria

1. `cargo build` succeeds
2. `cargo test` passes
3. `/health` returns OK
4. `/ws` accepts WebSocket connections
5. Auth message returns `auth_ok`
6. Register message returns `registered`
7. Registered agent appears in `hc sandbox call agent_info` output

## Revised Step Breakdown

| Step | Description | Test |
|------|-------------|------|
| M2a | WebSocket + Agent Registration | Agent visible in conductor |
| M2b | Signal Forwarding | Remote signal reaches browser |
| M2c | DHT Read Endpoints | GET /dht/{dna}/record works |
| M2d | DHT Publish Endpoint | POST /dht/{dna}/publish works |
| M2e | Zome Call Endpoint | /{dna}/{app}/{zome}/{fn} works |

This keeps each step small and testable.
