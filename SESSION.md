# Current Session

**Last Updated**: 2026-02-13
**Current Step**: Step 19.3 - Kitsune2 Query-Response Timeout Diagnosis

---

## Active Work: DHT Query Round-Trip Failure

### Problem

Gateway sends `GetReq`/`GetLinksReq` via `space.send_notify()` to conductors, but conductors never respond. Every DHT query times out after 30 seconds. Publishing (fire-and-forget) works fine.

### Evidence from e2e test (fishy-step19)

- `send_notify completed successfully` for every query
- `time.idle=30.0s` for every query (no response within timeout)
- NO `GetRes` or `GetLinksRes` messages received in gateway's `recv_notify`
- Publishing works: 12 ops sent to 2 peers, all accepted
- Conductors have full arcs: `Arc(0, 4294967295)`
- Gateway finds 2 responsive remote agents for every query

### Root Cause Analysis (in progress)

**What's confirmed:**
1. Wire encoding is identical — hc-membrane uses the exact same `WireMessage::encode_batch()`/`decode_batch()` as the conductor (shared crate `holochain_p2p`)
2. Message format is correct — `WireMessage::get_req()` produces valid `GetReq` with proper `msg_id`, `to_agent`, `dht_hash`
3. Gateway sends successfully — `send_notify` returns `Ok(())`
4. Response routing works — `PendingDhtResponses` correctly maps `msg_id` to oneshot channels

**What's unknown (needs diagnosis):**
1. Does the conductor receive the `GetReq` messages at all? (transport layer)
2. If received, does it process them? (handler execution)
3. If processed, does `send_notify(from_peer, response)` succeed? (reverse transport)
4. If sent, does the gateway's kitsune2 route it to `recv_notify`? (space routing)

**Conductor response path** (from `holochain_p2p/src/spawn/actor.rs` lines 355-389):
```
recv_notify → decode_batch → match GetReq → handle_get(dna, agent, hash) →
  Ok → encode_batch(GetRes) → space.send_notify(from_peer, resp)
  Err → encode_batch(ErrorRes) → space.send_notify(from_peer, resp)
```

### Test Infrastructure Created

Two test scripts in `scripts/`:

1. **`test-dht-roundtrip.sh`** — Standalone end-to-end test:
   - Starts bootstrap, 2 conductors (holochain_p2p=TRACE), hc-membrane (trace)
   - Registers fake agent via WebSocket
   - Curls DHT endpoints with 10s timeout
   - Reports PASS/FAIL + log analysis
   - Shows recv_notify counts, timeout counts, conductor errors

2. **`test-ws-client.mjs`** — Node.js WebSocket client:
   - Connects to gateway, authenticates, registers fake agent for a DNA
   - Responds to sign requests with fake signatures
   - Keeps connection alive so gateway maintains kitsune2 space

**Usage:**
```bash
cd /home/eric/code/metacurrency/holochain/hc-membrane-kitsune-dht-ops
nix develop -c bash scripts/test-dht-roundtrip.sh
# Or with specific hApp:
nix develop -c bash scripts/test-dht-roundtrip.sh --happ=mewsfeed --fishy-dir=/home/eric/code/metacurrency/holochain/fishy-step19
```

**Note:** The test has NOT been run yet — was created but execution was interrupted.

### Next Diagnostic Steps

1. **Run `test-dht-roundtrip.sh`** and check conductor logs:
   - Look for `recv_notify` evidence in conductor TRACE logs
   - Look for `Error sending get response` in conductor DEBUG logs
   - Count `Received GetRes response` in gateway logs

2. **If conductor doesn't receive messages:**
   - Check kitsune2 iroh transport connection between gateway and conductor
   - The gateway connects to conductor URL, but does iroh establish a bidirectional channel?

3. **If conductor receives but can't send back:**
   - The `from_peer` URL from conductor's `recv_notify` might be unreachable
   - Gateway's iroh endpoint may not be listening properly
   - Try adding a log in kitsune2 transport for outgoing send_notify failures

4. **If conductor sends but gateway doesn't receive:**
   - Space routing issue — response might go to wrong space handler
   - Message decoding issue in gateway's recv_notify

---

## Previous Work (2026-02-12)

### get_details and count_links kitsune endpoints (Step 19.2)

Implemented two new kitsune DHT operations:

1. **`get_details`** — kitsune variant of `/dht/{dna}/details/{hash}`
   - Uses existing `DhtQuery::get()` (same GetReq/GetRes wire protocol as `get`)
   - Added `wire_ops_to_details_json()` to convert WireOps → Details format
   - Handles both `WireRecordOps` → `Details::Record` and `WireEntryOps` → `Details::Entry`
   - Unit tests for all conversion cases

2. **`count_links`** — new endpoint `/dht/{dna}/count_links`
   - New wire protocol: `CountLinksReq`/`CountLinksRes`
   - Added `DhtQuery::count_links()` method
   - Added `CountLinksRes` to `recv_notify` handler
   - Route registered in router

Commits on kitsune-dht-ops branch:
- `98a2e3f` test: add unit tests for wire_ops_to_details conversion functions
- `3330aba` feat: implement kitsune get_details and count_links DHT operations

---

## Architecture Notes

### Query Flow

```
HTTP request → dht.rs endpoint → DhtQuery::get/get_links/count_links
  → get_peers_for_location (peer_store query)
  → send_get_request (per peer, parallel)
    → WireMessage::encode_batch([GetReq])
    → space.send_notify(peer_url, encoded)
    → register PendingDhtResponses(msg_id, oneshot_tx)
    → timeout(30s, oneshot_rx)
    → match response: GetRes → Ok(WireOps), ErrorRes → Err, timeout → Err
```

### Response Routing

```
Conductor sends GetRes via send_notify(gateway_url, encoded)
  → Gateway kitsune2 iroh transport receives
  → ProxySpaceHandler::recv_notify(from_peer, space, data)
  → WireMessage::decode_batch(data)
  → match GetRes/GetLinksRes/CountLinksRes/ErrorRes
  → PendingDhtResponses::route_response(msg)
  → oneshot_tx.send(msg) → unblocks the waiting DhtQuery
```

---

## Key Files

| File | Purpose |
|------|---------|
| `src/dht_query.rs` | DHT query methods (get, get_links, count_links) + PendingDhtResponses |
| `src/gateway_kitsune.rs` | KitsuneProxy, ProxySpaceHandler (recv_notify), GatewayKitsune |
| `src/routes/dht.rs` | HTTP endpoints, wire_ops conversion |
| `src/router.rs` | Route registration |
| `src/service.rs` | Service setup, DhtQuery + PendingDhtResponses wiring |
| `scripts/test-dht-roundtrip.sh` | Standalone round-trip test |
| `scripts/test-ws-client.mjs` | WebSocket agent registration helper |

## Conductor-Side Reference

| File (in holochain repo) | Purpose |
|--------------------------|---------|
| `crates/holochain_p2p/src/spawn/actor.rs:263-389` | SpaceHandler::recv_notify, GetReq/GetLinksReq handlers |
| `crates/holochain_p2p/src/types/wire.rs` | WireMessage enum, encode/decode_batch |
| `crates/holochain/src/conductor/cell.rs:412-478` | Cell::handle_get, handle_get_links, handle_count_links |

---

## Coordination with fishy-step19

This work is part of fishy Step 19 (Mewsfeed E2E Integration). Fishy repo: `/home/eric/code/metacurrency/holochain/fishy-step19`

**Fishy sub-steps:**
| Step | Status | Description |
|------|--------|-------------|
| 19.1 | Open | get_joining_timestamp deserialization (non-blocking) |
| 19.2 | Done | hc-membrane kitsune DHT ops (get_details, count_links) |
| 19.3 | **Active** | Kitsune2 query-response timeout (THIS) |
| 19.4 | Blocked | Published DhtOps not queryable (depends on 19.3) |
| 19.5 | Open | Sync XHR timeout reduction (fishy-side, independent) |
| 19.6 | Open | get_agent_activity host function (fishy-side, independent) |

See `/home/eric/code/metacurrency/holochain/fishy-step19/SESSION.md` for fishy-side status.
