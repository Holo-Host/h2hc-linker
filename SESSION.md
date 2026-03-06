# Current Session

**Last Updated**: 2026-03-06
**Current Step**: M6 (Planned) — All prior steps complete through M5.1

---

## Completed Work Summary

All features through M5.1 are merged into `main`. The project was renamed from **hc-membrane** to **h2hc-linker** (commit 30d6a9f).

### Implemented Features

| Area | Details |
|------|---------|
| **DHT Endpoints** | `GET /dht/{dna}/record/{hash}`, `GET /dht/{dna}/details/{hash}`, `GET /dht/{dna}/links`, `GET /dht/{dna}/count_links`, `POST /dht/{dna}/publish` |
| **Agent Activity** | `GET /dht/{dna}/agent_activity/{agent}`, `POST /dht/{dna}/must_get_agent_activity` |
| **Kitsune API** | `/k2/status`, `/k2/peers`, `/k2/space/{id}/status`, `/k2/space/{id}/peers`, `/k2/space/{id}/local-agents`, `/k2/transport/stats` |
| **Zome Calls** | `GET /api/{dna}/{zome}/{fn}` (requires conductor) |
| **WebSocket** | Agent registration, signal forwarding, remote signing, transparent signing protocol |
| **Auth Layer** | Admin API (`/admin/agents`), session tokens, capabilities (`dht_read`, `dht_write`, `k2`), WS challenge-response |
| **Kitsune2 Direct** | Wire protocol queries (DhtQuery), TempOpStore, PreflightCache, iroh/QUIC transport |
| **Reporting** | JSONL usage reporting (`linker_report.rs`) |
| **CI/CD** | GitHub Actions CI, cross-platform release workflow |

### Recent Commits (since last session update)

- `9506270` fix: use base64 strings for agent_pubkey in admin API
- `b2651f3` docs: added CAL License
- `30d6a9f` refactor: rename hc-membrane to h2hc-linker throughout codebase
- `6b4f223` feat: add kitsune2 usage reporting (hc-report JSONL)
- `6b4c646` feat: transparent signing protocol for agent info
- `d4e8e52` fix: convert WireOps to flat Record format in get endpoint
- `9b6d8ac` feat: add get_agent_activity and must_get_agent_activity endpoints

---

## Next Steps

### M6: Migrate op construction to gateway
- Add `produce_ops_from_record` in h2hc-linker
- Update POST /dht/{dna}/publish to accept Record
- holo-web-conductor extension sends Records instead of ops
- Keep old ops endpoint for backwards compat

### M7: Remove conductor dependency
### M8: Deprecate hc-http-gw-fork

---

## Test Commands

```bash
# Build h2hc-linker
nix develop --command cargo build

# Run unit tests
nix develop --command cargo test

# Run with auth enabled
H2HC_LINKER_ADMIN_SECRET=test-secret nix develop --command cargo run -- --port 8090

# Run without auth (backwards compatible)
nix develop --command cargo run -- --port 8090
```

---

## Key Files

| File | Purpose |
|------|---------|
| `src/auth/` | Authentication layer (types, store, middleware, admin) |
| `src/dht_query.rs` | DHT query methods (get, get_links, count_links) + PendingDhtResponses |
| `src/gateway_kitsune.rs` | KitsuneProxy, ProxySpaceHandler (recv_notify), GatewayKitsune |
| `src/routes/dht.rs` | HTTP endpoints, wire_ops conversion |
| `src/router.rs` | Route registration (open + authenticated modes) |
| `src/service.rs` | Service setup, DhtQuery + PendingDhtResponses wiring |
| `src/linker_report.rs` | Kitsune2 JSONL usage reporting |
| `src/wire_preflight.rs` | PreflightCache, BootstrapWrapperFactory |
| `tests/validation_op_fixtures.rs` | JS-to-Rust validation Op cross-deserialization tests |

---

## Quick Links

- [Step Registry](./STEPS/index.md) - All step statuses
- [Architecture](./ARCHITECTURE.md) - System architecture
- [M5 Plan](./STEPS/M5_PLAN.md) - Auth layer plan
