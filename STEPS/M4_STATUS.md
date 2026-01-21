# M4 Status: Direct DHT Operations - BLOCKED

**Last Updated**: 2026-01-21
**Status**: BLOCKED - Direct wire protocol not working

---

## Summary

M4 implementation is complete but **direct kitsune2 wire protocol queries do not work**. The conductors do not respond to GetReq/GetLinksReq wire messages sent by the gateway. Only the `conductor-dht` feature flag mode (which uses zome calls via conductor) works.

---

## What Works

1. **conductor-dht mode** (using zome calls):
   - `cargo build --features conductor-dht`
   - Uses conductor's dht_util zome for DHT queries
   - This is what hc-http-gw-fork uses and it works

2. **Gateway infrastructure**:
   - Gateway connects to kitsune2 network
   - Discovers peers (conductors) with full arcs
   - Registers browser agents in spaces
   - Signal forwarding works
   - Publishing works

---

## What Does NOT Work

**Direct kitsune2 wire protocol queries**:

```
Gateway sends: GetReq/GetLinksReq via space.send_notify()
Expected: Conductor responds with GetRes/GetLinksRes
Actual: No response - request times out after 30 seconds
```

### Symptoms

1. Gateway logs show requests being sent to conductor URLs
2. No responses received - all queries time out
3. Extension popup freezes/crashes due to 30s synchronous XHR timeout
4. Extension becomes unusable

### Gateway Logs (example)

```
Querying peers for get_links dna=uhC0k... base=uhCEk... peer_count=2
Sent get_links request msg_id=11 to_url=ws://127.0.0.1:45021/... to_agent=uhCAk...
Peer query failed e=Internal("Request timed out")
```

---

## Root Cause Investigation Needed

Possible reasons conductors don't respond:

1. **Wire message format mismatch** - Gateway encoding may differ from what conductor expects
2. **Peer authentication** - Conductor may not recognize gateway as valid peer for queries
3. **send_notify vs other method** - Perhaps a different send method is needed
4. **Protocol version mismatch** - Wire protocol version incompatibility
5. **Target agent routing** - Conductor may not route to the correct handler

### Files to Investigate

- `holochain/crates/holochain_p2p/src/spawn/actor.rs` - How holochain sends GetReq
- `holochain/crates/holochain_p2p/src/types/wire.rs` - Wire message format
- `hc-membrane/src/dht_query.rs` - Current gateway implementation

---

## Code Changes Made (Untested)

### 1. wire_link_ops_to_links function (src/routes/dht.rs)

Converts WireLinkOps to Vec<Link> with proper ActionHash computation:

```rust
fn wire_link_ops_to_links(
    ops: &WireLinkOps,
    base: &AnyLinkableHash,
    query_tag: Option<&LinkTag>,
) -> Vec<Link> {
    // Reconstructs CreateLink action from WireCreateLink
    // Computes ActionHash via action.to_hash()
    // Returns Vec<Link> matching conductor format
}
```

**Purpose**: Ensure network links have same ActionHash as local links for deduplication in cascade.ts

**Status**: Cannot test until direct wire protocol works

### 2. Extension WireLinkOps parsing

- `ribosome-worker.ts`: Added parseWireLinkOps method
- `sync-xhr-service.ts`: Same changes in @fishy/core

**Note**: Gateway now returns Vec<Link> format, so WireLinkOps parsing may not be needed. But kept for flexibility.

---

## How to Test

### Test with conductor-dht (WORKS)

```bash
# 1. Build with conductor-dht feature (IMPORTANT: do this FIRST)
cd /home/eric/code/metacurrency/holochain/hc-membrane
nix develop --command cargo build --release --features conductor-dht

# 2. Start test environment (uses the binary you just built)
cd /home/eric/code/metacurrency/holochain/fishy
nix develop --command ./scripts/e2e-test-setup.sh start --happ=ziptest --gateway=membrane

# 3. Load fishy extension in browser
# 4. Extension should work - profiles load, entries visible
```

**Note**: The e2e-test-setup.sh uses whatever binary exists at `../hc-membrane/target/release/hc-membrane`. It doesn't rebuild or specify features. Always rebuild with the desired features before starting the test environment.

### Test direct mode (FAILS)

```bash
# Build without conductor-dht (default)
nix develop --command cargo build --release

# Start test environment
cd ../fishy && nix develop --command ./scripts/e2e-test-setup.sh start --happ=ziptest --gateway=membrane

# Extension will freeze/crash on any DHT query
```

---

## Next Steps

1. **Debug wire protocol**: Add logging to conductor to see if it receives messages
2. **Compare with holochain_p2p**: Understand exactly how GetReq is sent and handled
3. **Check if conductor needs configuration**: Maybe conductor needs to be configured to accept queries from external peers
4. **Consider alternative approach**: Maybe query via different wire message type

---

## Temporary Workaround

Use conductor-dht feature until direct wire protocol is fixed:

```bash
# In Cargo.toml, could make conductor-dht the default:
[features]
default = ["conductor-dht"]
conductor-dht = []
```

Or always build with:
```bash
cargo build --release --features conductor-dht
```
