# M4 Status: Direct DHT Operations - WORKING

**Last Updated**: 2026-02-05
**Status**: WORKING - kitsune2 0.4.0-dev.2 + iroh + PreflightCache

---

## Summary

Direct wire protocol working with kitsune2 0.4.0-dev.2 and iroh transport, matching Holochain 0.6.1-rc.0.

**M4.1 Update (2026-02-05)**: Added PreflightCache to include registered agent infos in preflight messages, enabling conductor authorization.

---

## What Works

### 1. Direct Wire Protocol (default mode)
- Gateway sends GetReq/GetLinksReq via `space.send_notify()`
- Conductor receives, processes, and responds
- Gateway receives GetRes/GetLinksRes responses
- Response routing to pending requests works

### 2. Preflight Agent Info (NEW - M4.1)
- PreflightCache captures AgentInfoSigned from Bootstrap::put()
- Preflights include all registered browser agents
- Conductors accept connections from gateway
- Both GetReq and GetLinksReq responses received

### 3. conductor-dht mode (fallback)
- `cargo build --features conductor-dht`
- Uses conductor's dht_util zome for DHT queries

### 4. Gateway Infrastructure
- Gateway connects to kitsune2 network via iroh
- Discovers peers (conductors) with full arcs
- Registers browser agents in spaces
- Signal forwarding works (conductor → gateway)
- Publishing works (via kitsune2 publish mechanism)

---

## Evidence from Test Run (2026-02-05)

### Agent Registration & Preflight
```
Agent AgentPubKey(...BEmDzl...) registered for DNA (total: 1)
Agent AgentPubKey(...IfxZGx...) registered for DNA (total: 2)
Updated preflight cache with agent infos agent_count=2
Sending preflight to peer proto_ver=2 agent_count=2
Validated incoming preflight proto_ver=2 agent_count=1
```

### Direct Wire Protocol
```
send_notify completed successfully, waiting for response... msg_id=223
>>> Received GetLinksRes response msg_id=223
Got GetLinksRes response msg_id=223 creates_count=1 deletes_count=0
Converted WireCreateLink to Link target=uhCEkBEmDzlNWBWcEDEuVkCev96p5puZtzYPCyTxD2q9c-FC61fUJ
```

### Publishing
```
Publishing ops to peers dna=uhC0k... op_count=1 peer_count=2 basis_loc=1568180260
Published ops to peer url=http://127.0.0.1:38553/6559742f...
Published ops to peer url=http://127.0.0.1:38553/4d8453ee...
Publish request completed queued=9 failed=0 published=9 success=true
```

---

## Current E2E Test Status

### With ziptest + membrane (2026-02-05)
- ✅ Both browser agents register with gateway
- ✅ Gateway exchanges preflights with both conductors
- ✅ Preflights include 2 agent infos
- ✅ Conductors grant access to gateway URLs
- ✅ Profiles published to both conductors
- ✅ get_links returns correct data (both profiles found)
- ⚠️ One browser window shows other agent's profile
- ❌ Second browser window times out waiting for "active" agent

### Remaining Issue
The "active" status detection in ziptest UI relies on ping/signal responses between agents. Investigation needed:
1. Are browser agents sending pings (remote signals) to each other?
2. Is gateway forwarding browser-to-browser signals?
3. Ziptest UI "active" status logic

---

## Files Modified (M4.1)

| File | Change |
|------|--------|
| `src/wire_preflight.rs` | NEW: PreflightCache, BootstrapWrapper, BootstrapWrapperFactory |
| `src/gateway_kitsune.rs` | Integrated preflight_cache into KitsuneProxy |
| `src/service.rs` | Pass BootstrapWrapperFactory to kitsune builder |
| `src/kitsune.rs` | API updates for kitsune2 0.4.x |
| `src/config.rs` | relay_url config (renamed from signal_url) |
| `Cargo.toml` | Dependencies updated for 0.6.1-rc.0 |

---

## Test Commands

### Direct Mode (Default)

```bash
# Build hc-membrane
cd /home/eric/code/metacurrency/holochain/hc-membrane
nix develop -c cargo build --release

# Run unit tests
nix develop -c cargo test

# Run e2e tests (from fishy repo)
cd /home/eric/code/metacurrency/holochain/fishy
npm run e2e:env -- start --happ=ziptest --gateway=membrane
npm run e2e:test
```

### With Debug Logging

```bash
# Gateway with debug logs
RUST_LOG=hc_membrane=debug nix develop -c cargo run --release

# Or via e2e setup
RUST_LOG=hc_membrane=debug npm run e2e:env -- start --happ=ziptest --gateway=membrane
```

---

## Next Steps

1. **Diagnose "active" agent detection**:
   - Check signal flow between browser agents
   - Verify gateway forwards browser-to-browser signals
   - Review ziptest UI "active" status logic

2. **After e2e passes**:
   - Commit all changes
   - Update documentation
   - Consider M5 (migrate op construction to gateway)
