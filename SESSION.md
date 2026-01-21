# Current Session

**Last Updated**: 2026-01-21
**Current Step**: M4 BLOCKED - Direct wire protocol not working

---

## Critical Status

### M4 Direct DHT Operations - BLOCKED

**The direct kitsune2 wire protocol does NOT work.** Conductors do not respond to GetReq/GetLinksReq messages from the gateway.

- See [M4_STATUS.md](./STEPS/M4_STATUS.md) for full details
- **Workaround**: Use `--features conductor-dht` to fall back to zome calls

### What Works vs What Doesn't

| Feature | Status | Notes |
|---------|--------|-------|
| conductor-dht mode | ✅ Works | Uses zome calls via conductor |
| Direct wire protocol | ❌ Blocked | Conductors don't respond |
| Signal forwarding | ✅ Works | Via kitsune2 |
| Publishing | ✅ Works | Via kitsune2 |
| Extension with gateway | ⚠️ Depends | Works ONLY with conductor-dht feature |

---

## Test Commands

### Working Configuration (conductor-dht)

```bash
# Build with conductor-dht feature
cd /home/eric/code/metacurrency/holochain/hc-membrane
nix develop --command cargo build --release --features conductor-dht

# Start test environment
cd /home/eric/code/metacurrency/holochain/fishy
nix develop --command ./scripts/e2e-test-setup.sh start --happ=ziptest --gateway=membrane

# Extension should work
```

### Broken Configuration (direct mode)

```bash
# Build without conductor-dht (default)
nix develop --command cargo build --release

# This will cause extension to freeze/crash on DHT queries
```

---

## Root Cause

The gateway sends GetReq/GetLinksReq wire messages via `space.send_notify()`, but conductors never respond. All queries time out after 30 seconds, which freezes the extension's synchronous XHR.

Investigation needed in:
- How holochain_p2p sends these messages
- Whether conductor needs special configuration
- Wire message format compatibility

---

## Code Changes Made This Session

1. **wire_link_ops_to_links** in `src/routes/dht.rs`:
   - Converts WireLinkOps to Vec<Link> with proper ActionHash
   - Uses `HashableContentExtSync::to_hash()` on reconstructed CreateLink
   - Cannot test until direct mode works

2. **Extension parsing** in ribosome-worker.ts:
   - Added WireLinkOps format handling
   - May not be needed since gateway returns Vec<Link>

---

## Next Session Goals

1. Investigate why conductors don't respond to wire protocol queries
2. Compare gateway's wire message encoding with holochain_p2p
3. Check conductor logs for incoming messages
4. Fix direct wire protocol OR document it as a known limitation

---

## Quick Links

- [M4 Status](./STEPS/M4_STATUS.md) - Full details on blocked state
- [Step Registry](./STEPS/index.md) - All step statuses
- [Architecture](./ARCHITECTURE.md) - System architecture
