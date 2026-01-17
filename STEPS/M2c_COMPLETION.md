# M2c Completion: DHT Read Endpoints

**Completed**: 2026-01-17

## Summary

Added DHT read endpoints that work via conductor's dht_util zome. This is temporary scaffolding that will be replaced with direct kitsune2 queries in M6.

## Implementation Details

### New Files

| File | Purpose |
|------|---------|
| `src/conductor/mod.rs` | Conductor connection module |
| `src/conductor/admin_conn.rs` | Admin websocket with auto-reconnection |
| `src/conductor/app_conn.rs` | App websocket for zome calls |
| `src/routes/dht.rs` | DHT endpoints |

### Modified Files

| File | Changes |
|------|---------|
| `src/config.rs` | Added `zome_call_timeout` |
| `src/error.rs` | Added conductor error variants |
| `src/lib.rs` | Added `conductor` module |
| `src/routes/mod.rs` | Added `dht` module |
| `src/router.rs` | Added DHT routes |
| `src/service.rs` | Added `AppConn` to `AppState` |

### Endpoints Added

```
GET /dht/{dna_hash}/record/{hash}  - Get record by action/entry hash
GET /dht/{dna_hash}/links          - Get links from base hash
```

### Configuration

```bash
# Enable conductor connection
export HC_MEMBRANE_ADMIN_WS_URL="127.0.0.1:4444"

# Optional: zome call timeout (default 10s)
export HC_MEMBRANE_ZOME_CALL_TIMEOUT_MS="10000"
```

## Architecture

```
HTTP Request
    │
    ▼
/dht/{dna}/record/{hash}
    │
    ▼
AppConn.call_dht_util()
    │
    ├── find_app_with_dna() ── AdminConn.list_apps()
    │
    └── AppWebsocket.call_zome("dht_util", fn_name, payload)
    │
    ▼
JSON Response
```

## Test Results

```
running 32 tests
test result: ok. 32 passed
```

## Simplifications

Per user request, the following hc-http-gw-fork features were NOT ported:
- `allowed_app_ids` filtering - all apps allowed
- `allowed_fns` filtering - all functions allowed
- Session/auth verification - not needed for testing

This code is temporary and will be removed in M6 when direct kitsune2 queries are implemented.

## Next Step

M2d: DHT Write Endpoints
- POST /dht/{dna}/publish
