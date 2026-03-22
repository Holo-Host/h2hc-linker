# WIP: No Peers Available Returns Error Instead of Empty Result

## What the uncommitted changes do

Across 6 query methods in `src/dht_query.rs` and one handler in `src/routes/dht.rs`:

1. **No-peers-found → proper errors instead of silent `Ok(None)`**: When no peers are found for a DHT location, the code now returns a `LinkerError::Network` error instead of silently returning `Ok(None)`. Log level bumped from `info` to `warn`.

2. **Task join errors preserved**: When async task joins fail and no successful result has been collected yet, the join error is now captured in `last_result` so it propagates instead of being silently swallowed.

3. **Serialization error propagated**: The `get_links` handler now uses `?` to propagate serialization errors instead of silently falling back to an empty `[]`.

## Impact on holo-web-conductor (hwc)

The hwc's `SyncXHRNetworkService` handles HTTP status codes from the linker as follows:

### Current behavior (before the change)

When no peers are available, the linker returns **`200 OK` with `null`** (or empty array for links). The hwc treats this as "data not found" — a normal, non-error condition.

### New behavior (with the uncommitted change)

When no peers are available, the linker now returns a **`502 Bad Gateway`** (`LinkerError::Network` maps to 502).

| Endpoint | Before (200 + null) | After (502) |
|---|---|---|
| **get record/details** | Returns `null` — zome sees "not found" | **Throws error** — zome call crashes |
| **get_links** | Returns `[]` — zome sees no links | **Throws error** — zome call crashes |
| **count_links** | Returns `0` | **Throws error** — zome call crashes |
| **agent_activity** | Returns `null` | Returns `null` (swallows errors gracefully) |
| **must_get_agent_activity** | Returns `null` | Returns `null` (swallows errors gracefully) |

## The real concern

The big difference: "no peers available" used to look like "data doesn't exist yet" to the hApp. Now it becomes a **hard failure** that throws through the host function layer. For hApps that can tolerate eventual consistency (data just hasn't propagated yet), this is a breaking change — a transient network condition now crashes zome calls instead of returning empty results.

The agent_activity endpoints are unaffected because the hwc already swallows non-200/404 errors and returns `null`.

## Whether this is good or bad depends on intent

- **Good**: Distinguishes "no data exists" from "we couldn't reach anyone to ask" — the old behavior silently hid network problems
- **Bad**: hApps aren't prepared to handle this. A temporary peer shortage (e.g., during bootstrap) will now cause errors instead of graceful empty results that resolve once peers appear
