# M2e Completion: Zome Call Endpoint

**Completed**: 2026-01-17

## Summary

Added zome call endpoint for executing zome functions via the conductor. This allows HTTP clients to call any zome function with JSON payloads.

## Implementation Details

### New Files

| File | Purpose |
|------|---------|
| `src/routes/zome_call.rs` | Zome call endpoint handler |

### Modified Files

| File | Changes |
|------|---------|
| `src/conductor/app_conn.rs` | Added general `call_zome()` method |
| `src/routes/mod.rs` | Added `zome_call` module, export handler |
| `src/router.rs` | Added `GET /api/{dna_hash}/{zome_name}/{fn_name}` route |

### Endpoint Added

```
GET /api/{dna_hash}/{zome_name}/{fn_name}?payload={base64_url_safe_json}
```

**Path Parameters**:
- `dna_hash` - The DNA hash (base64 encoded)
- `zome_name` - The zome name
- `fn_name` - The function name

**Query Parameters**:
- `payload` - Optional base64 URL-safe encoded JSON payload

**Response**:
Returns the zome function result as JSON string.

**Example**:
```bash
# Call a zome function with no payload
curl "http://localhost:8090/api/uhC0k.../my_zome/get_all"

# Call with payload (base64 URL-safe encoded JSON)
curl "http://localhost:8090/api/uhC0k.../my_zome/create_entry?payload=eyJjb250ZW50IjoiaGVsbG8ifQ"
```

## Architecture

```
HTTP Request
    │
    ▼
GET /api/{dna}/{zome}/{fn}?payload=...
    │
    ├─► Parse DNA hash
    ├─► Decode base64 JSON payload to ExternIO
    │
    ▼
AppConn.call_zome()
    │
    ├─► find_app_with_dna() ── AdminConn.list_apps()
    │
    └─► AppWebsocket.call_zome(cell_id, zome, fn, payload)
    │
    ▼
JSON Response (ExternIO decoded to JSON)
```

## Configuration

Requires conductor connection:
```bash
export HC_MEMBRANE_ADMIN_WS_URL="127.0.0.1:4444"
```

## Test Results

```
running 42 tests
test result: ok. 42 passed
```

New tests:
- `test_base64_json_to_extern_io_none`
- `test_base64_json_to_extern_io_valid`
- `test_base64_json_to_extern_io_invalid_base64`
- `test_base64_json_to_extern_io_invalid_json`
- `test_extern_io_to_json`

## Simplifications

Per user request, the following hc-http-gw-fork features were NOT ported:
- `allowed_app_ids` filtering - all apps allowed
- `allowed_fns` filtering - all functions allowed
- `coordinator_identifier` path param - not needed (use DNA hash directly)
- Payload size limit - not enforced

This keeps the implementation simple for testing.

## Next Step

M2 series complete. Next is M4: Integrate holochain_p2p (direct kitsune2 queries).
