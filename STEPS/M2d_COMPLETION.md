# M2d Completion: DHT Publish Endpoint

**Completed**: 2026-01-17

## Summary

Added DHT publish endpoint that allows browser extension agents to publish their source chain data to the DHT via kitsune2. This is the direct-to-kitsune path (not conductor-based like the read endpoints).

## Implementation Details

### New Files

| File | Purpose |
|------|---------|
| `src/temp_op_store.rs` | Temporary OpStore for browser extension publishing |
| `src/routes/publish.rs` | Publish endpoint handler |

### Modified Files

| File | Changes |
|------|---------|
| `src/lib.rs` | Added `temp_op_store` module |
| `src/gateway_kitsune.rs` | Added `with_op_store()` to builder, `publish_ops()` to GatewayKitsune |
| `src/service.rs` | Added TempOpStoreHandle to AppState, create/wire TempOpStore |
| `src/routes/mod.rs` | Added `publish` module, export `dht_publish` |
| `src/router.rs` | Added `POST /dht/{dna_hash}/publish` route |

### Endpoint Added

```
POST /dht/{dna_hash}/publish
```

**Request Body**:
```json
{
  "ops": [
    {
      "op_data": "<base64 msgpack encoded DhtOp>",
      "signature": "<base64 64-byte Ed25519 signature>"
    }
  ]
}
```

**Response Body**:
```json
{
  "success": true,
  "queued": 3,
  "failed": 0,
  "published": 3,
  "results": [
    { "success": true },
    { "success": true },
    { "success": true }
  ]
}
```

## Architecture

```
Browser Extension
    │
    ▼
POST /dht/{dna}/publish (SignedDhtOps)
    │
    ├─► Decode ops from base64/msgpack
    ├─► Store in TempOpStore (60s TTL)
    ├─► Group by basis location
    │
    ▼
GatewayKitsune.publish_ops()
    │
    ├─► Find peers near basis location
    ├─► Send op IDs via kitsune2
    │
    ▼
DHT Authorities fetch from TempOpStore
    │
    ▼
Ops stored permanently on DHT
```

## Key Components

### TempOpStore

- In-memory op store with 60-second TTL
- Implements kitsune2 `OpStore` trait
- Used by kitsune2 when peers fetch ops
- Background cleanup task removes expired ops

### GatewayKitsune.publish_ops()

- Finds responsive peers near basis location
- Uses `get_responsive_remote_agents_near_location`
- Sends op IDs via `space.publish().publish_ops()`
- Returns count of peers that received the ops

## Test Results

```
running 37 tests
test result: ok. 37 passed
```

New tests:
- `test_publish_request_deserialization`
- `test_publish_response_serialization`
- `test_store_and_retrieve_op`
- `test_clear_ops`
- `test_cleanup_expired`

## Next Step

M2e: Zome Call Endpoint
- GET /{dna}/{app}/{zome}/{fn}
- Uses conductor app websocket for zome calls
