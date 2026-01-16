# M2a Completion: WebSocket + Agent Registration

**Completed**: 2026-01-16

## Summary

Implemented WebSocket endpoint for browser extension connections and kitsune2 agent registration. Browser agents can now connect to hc-membrane, authenticate, register for DNAs, and have their agent info broadcast to the holochain network.

## Implementation Details

### New Files

| File | Purpose |
|------|---------|
| `src/agent_proxy.rs` | `AgentProxyManager` - tracks WebSocket connections, routes signals, handles remote signing requests |
| `src/proxy_agent.rs` | `ProxyAgent` - implements `LocalAgent` + `Signer` traits for browser agents |
| `src/gateway_kitsune.rs` | `KitsuneProxy` (handler), `KitsuneProxyBuilder`, `GatewayKitsune` (agent lifecycle) |
| `src/routes/websocket.rs` | WebSocket upgrade handler, message parsing, client/server message types |
| `src/wire_preflight.rs` | `WirePreflightMessage` for kitsune2 peer handshake |

### Modified Files

| File | Changes |
|------|---------|
| `src/config.rs` | Added `WebSocketConfig` (heartbeat, idle timeouts) |
| `src/service.rs` | Added `AppState` with `AgentProxyManager` and `GatewayKitsune` |
| `src/router.rs` | Added `/ws` route |
| `src/lib.rs` | Added new modules |
| `src/routes/mod.rs` | Added websocket module |

### Test Infrastructure

| File | Purpose |
|------|---------|
| `flake.nix` | Nix flake with holochain, hc, bootstrap-srv |
| `scripts/e2e-test-membrane.sh` | Start/stop bootstrap, conductor, hc-membrane |

## WebSocket Protocol

### Client Messages
- `auth` - Authenticate (session_token, currently unused)
- `register` - Register agent for DNA
- `unregister` - Unregister agent
- `ping` - Heartbeat
- `sign_response` - Return signature from browser

### Server Messages
- `auth_ok` - Authentication success
- `registered` - Registration confirmed
- `signal` - Forwarded signal (M2b)
- `sign_request` - Request browser to sign data
- `pong` - Heartbeat response
- `error` - Error message

## Remote Signing Protocol

1. Kitsune2 calls `ProxyAgent::sign()`
2. `AgentProxyManager::request_signature()` sends `sign_request` to browser
3. Browser signs with Lair keystore
4. Browser sends `sign_response` back
5. `AgentProxyManager::deliver_signature()` completes the future
6. Signature returned to kitsune2

## Test Results

### Unit Tests
```
running 28 tests
test result: ok. 28 passed
```

### E2E Verification

1. Started bootstrap server, conductor, hc-membrane
2. Loaded fishy extension with ziptest hApp
3. Verified in membrane.log:
   - Register messages received
   - Agents joined kitsune2 space
   - Remote signing succeeded (64-byte signatures)
   - Agent info broadcast to peers

4. Verified in conductor:
   - `hc sandbox call list-agents` shows gateway-registered agents
   - Agents have gateway URL (different from conductor URL)
   - Note: `agent_pub_key` shows null for external agents (expected)

## Known Limitations

1. **Agent refresh requires active WebSocket**: Kitsune2 refreshes agent info every ~30 seconds. If browser disconnects, refresh signing fails and agent is removed from space.

2. **No signal forwarding yet**: `recv_notify` logs signals but doesn't forward to browser (M2b).

## Next Step

M2b: Signal Forwarding - decode `WireMessage::Signal` in `ProxySpaceHandler::recv_notify` and route to browser agents.
