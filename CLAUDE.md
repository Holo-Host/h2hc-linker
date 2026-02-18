# h2hc-linker

> Holochain-to-Holochain Linker - Network edge gateway providing DHT access for lightweight clients

## Quick Context (READ FIRST)

**Current Step**: See [STEPS/index.md](./STEPS/index.md) for status registry

**What This Is**: A network edge gateway that allows lightweight clients (browsers, mobile) to interact with the Holochain DHT without running a full conductor. It provides selective access between clients and the DHT network.

**What This Is NOT**: This is not "Holochain Lite" - there is no source chain, no validation, no full node capabilities. Zero-arc only.

---

## Critical Rules

### Strong Typing (MANDATORY)

- **Never use plain `String`** for typed values (hashes, URLs, identifiers) - use proper newtypes
- **Reuse types from holochain crates** - check these before defining new structs:
  - `holo_hash`: `AgentPubKey`, `ActionHash`, `EntryHash`, `DnaHash`
  - `holochain_types`: `DhtOp`, `Action`, `Entry`, `Record`, `SignedAction`
  - `kitsune2_api`: `AgentInfoSigned`, `SpaceId`, `StoredOp`, `OpId`
  - `holochain_conductor_api`: API types, app info types
  - `holochain_p2p`: `WireMessage`, `WireOps`, `WireLinkOps` (wire types only, not the HolochainP2pDna trait)
- **Do not recreate structs** that already exist in dependencies - find and use them
- Use `#[serde(transparent)]` newtypes for domain-specific wrappers

### Reference Sources (Priority Order)

1. **Kitsune2 patterns**: `../holochain/crates/holochain_p2p/src/` - Holochain's kitsune2 integration
2. **Holochain types**: `../holochain/crates/holochain_types/src/`
3. **Hash types**: `../holochain/crates/holo_hash/src/`
4. **Gateway patterns**: `../hc-http-gw-fork/` - Original HTTP gateway (being superseded)
5. **Serialization lessons**: `../fishy/LESSONS_LEARNED.md` - msgpack/serialization pitfalls

**Avoid web searches** for implementation details - local repos have authoritative code.

---

## Architecture Overview

See [ARCHITECTURE.md](./ARCHITECTURE.md) for the full diagram.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         h2hc-linker                                  │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │ DHT Endpoints (/dht/*)                                       │   │
│  │  GET  /dht/{dna}/record/{hash}   → get record by hash        │   │
│  │  GET  /dht/{dna}/details/{hash}  → record + updates/deletes  │   │
│  │  GET  /dht/{dna}/links           → get_links(base, type)     │   │
│  │  POST /dht/{dna}/publish         → publish signed DhtOps     │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │ Kitsune Direct API (/k2/*)                                   │   │
│  │  GET  /k2/status                 → overall network status     │   │
│  │  GET  /k2/peers                  → list all known peers       │   │
│  │  GET  /k2/space/{id}/status      → space-specific status     │   │
│  │  GET  /k2/space/{id}/peers       → peers in a space          │   │
│  │  GET  /k2/space/{id}/local-agents → local agents             │   │
│  │  GET  /k2/transport/stats        → transport statistics       │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │ DhtQuery (direct wire protocol) + kitsune2 (network layer)   │   │
│  └──────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │ Kitsune2 P2P (iroh/QUIC)
                                   ▼
                  ┌────────────────────────────────┐
                  │  DHT Authorities / Full Nodes  │
                  └────────────────────────────────┘
```

---

## Development Guidelines

### Build Environment (MANDATORY)

**Always use `nix develop` for all cargo and npm run commands.** The project has native dependencies (libdatachannel, etc.) that require the nix environment.

```bash
# Correct - always prefix with nix develop
nix develop --command cargo build
nix develop --command cargo test
nix develop --command cargo run

# Or enter the shell first
nix develop
cargo build && cargo test
```

### Before Coding

1. Check if the type/struct already exists in holochain crates
2. Reference `../holochain/crates/holochain_p2p/` for kitsune2 patterns
3. Write tests before implementation

### Serialization

- Use `holochain_serialized_bytes` for msgpack serialization
- Hash types are 39 bytes (32 core + 3 type prefix + 4 location)
- See `../fishy/LESSONS_LEARNED.md` for detailed debugging strategies

### Commit Hygiene

- No claude co-authored messages
- Run `cargo fmt` and `cargo clippy` before commits

### Communication Style

- No emotional tags or punctuation
- Code-related information only

---

## Testing and Integration

### Test Infrastructure

Testing is done via the **fishy** browser extension and **ziptest** hApp:

- **Test scripts**: `../fishy/scripts/e2e-test-setup.sh` - starts conductors and gateway
- **Test app**: ziptest hApp at `../fishy/fixtures/ziptest.happ`
- **E2E test page**: `../fishy/packages/extension/test/e2e-gateway-test.html`

### Testing h2hc-linker Changes

Testing requirements vary by step:

**M1-M3 (before DHT endpoints)**:
```bash
# Build and test unit tests (always use nix develop)
nix develop --command cargo build --release
nix develop --command cargo test

# Test liveness endpoints manually
nix develop --command cargo run -- --port 8090 &
curl http://localhost:8090/health
curl http://localhost:8090/k2/status
```

**M2+ (with DHT endpoints)**: Full ziptest integration
```bash
# 1. Build h2hc-linker (always use nix develop)
nix develop --command cargo build --release

# 2. Run e2e setup with h2hc-linker (requires --gateway=membrane flag, added in M2)
cd ../fishy && ./scripts/e2e-test-setup.sh start --happ=ziptest --gateway=membrane

# 3. Load fishy extension in browser, test with ziptest UI

# 4. Run fishy integration tests
cd ../fishy && npm run test:integration
```

### e2e-test-setup.sh Adaptation

As h2hc-linker development progresses, `../fishy/scripts/e2e-test-setup.sh` should be adapted:

1. **M2**: Add `--gateway=membrane` flag to switch between hc-http-gw-fork and h2hc-linker
2. **M6**: Default to h2hc-linker, deprecate hc-http-gw-fork path
3. **M7**: Remove hc-http-gw-fork support entirely

### Test Checkpoints

Each migration step should verify:
- Gateway starts and responds to `/health`
- Conductor connects successfully
- ziptest basic operations work (create entry, get entry, get links)
- Remote signals work between browser and conductor

---

## Key Dependencies

```toml
holo_hash = "0.6.1-rc.0"              # Hash types
holochain_types = "0.6.1-rc.0"        # Core Holochain types
holochain_p2p = "0.6.1-rc.0"          # P2P/Kitsune semantic layer
kitsune2_api = "0.4.0-dev.2"          # Kitsune2 API types
kitsune2_core = "0.4.0-dev.2"         # Kitsune2 implementations
kitsune2_transport_iroh = "0.4.0-dev.2"  # Iroh transport (replaces tx5/webrtc)
```

---

## Type Lookup Reference

| Need | Crate | Type |
|------|-------|------|
| Agent key | `holo_hash` | `AgentPubKey` |
| Entry hash | `holo_hash` | `EntryHash` |
| Action hash | `holo_hash` | `ActionHash` |
| DHT operation | `holochain_types` | `DhtOp` |
| Signed action | `holochain_types` | `SignedAction` |
| Agent info | `kitsune2_api` | `AgentInfoSigned` |
| Space ID | `kitsune2_api` | `SpaceId` |
| Op storage | `kitsune2_api` | `StoredOp` |
| Wire messages | `holochain_p2p` | `WireMessage`, `WireOps` |

---

## Kitsune2 Reference Files

For implementing kitsune2 integration, study these in `../holochain/crates/holochain_p2p/src/`:

- `spawn/actor.rs` - Kitsune2 lifecycle management
- `op_store.rs` - Op storage interface
- `local_agent.rs` - Agent management
- `types/wire.rs` - Wire protocol types

---

## Documentation Structure

| File | Purpose |
|------|---------|
| `CLAUDE.md` | This file - core rules and quick context |
| `SESSION.md` | Current step focus and how to resume |
| `ARCHITECTURE.md` | System architecture diagram |
| `STEPS/index.md` | Step status registry |
| `STEPS/X_PLAN.md` | Detailed plan for step X |
| `STEPS/X_COMPLETION.md` | Completion notes for step X |
| `STEPS/GATEWAY_ARCHITECTURE_ANALYSIS.md` | Detailed architecture analysis |

---

## Workflow

### Starting a New Step
1. Create `STEPS/X_PLAN.md` with detailed sub-tasks
2. Update `SESSION.md` to show current step
3. Update `STEPS/index.md` status

### Completing a Step
1. Create `STEPS/X_COMPLETION.md` with summary, test results, issues fixed
2. Update `SESSION.md` to next step
3. Update `STEPS/index.md` status
4. Commit: `docs: Step X complete`

### How to Resume (SESSION Pattern)

When starting work on a different computer or after a break:

```bash
# 1. Check current state
cat SESSION.md
cat STEPS/index.md

# 2. Read the current step plan
cat STEPS/<current>_PLAN.md

# 3. Run tests to verify state
cargo test
```

The `SESSION.md` file serves as the single source of truth for:
- What step is currently in progress
- What was just completed
- What comes next
- Quick links to relevant files
