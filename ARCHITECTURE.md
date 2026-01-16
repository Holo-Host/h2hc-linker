# hc-membrane Architecture

> Holochain Membrane - Network edge gateway providing DHT access for lightweight clients

---

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                            CLIENTS                                   │
│                                                                      │
│   ┌─────────────────┐    ┌─────────────────┐    ┌────────────────┐  │
│   │ Browser (Fishy) │    │  Mobile App     │    │   CLI Tool     │  │
│   │                 │    │                 │    │                │  │
│   │ Sync XHR for    │    │ HTTP/WS for     │    │ HTTP for       │  │
│   │ DHT queries     │    │ all operations  │    │ DHT queries    │  │
│   │ WS for signals  │    │                 │    │                │  │
│   └────────┬────────┘    └────────┬────────┘    └───────┬────────┘  │
│            │                      │                      │           │
└────────────┼──────────────────────┼──────────────────────┼───────────┘
             │                      │                      │
             └──────────────────────┼──────────────────────┘
                                    │
                     HTTP (sync) + WebSocket (async)
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│                          hc-membrane                                 │
│                                                                      │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ HTTP API Layer (axum)                                         │  │
│  │                                                                │  │
│  │ Holochain Semantic API (/hc/*)                                │  │
│  │  GET  /hc/{dna}/record/{hash}     → get record by hash        │  │
│  │  GET  /hc/{dna}/entry/{hash}      → get entry by hash         │  │
│  │  GET  /hc/{dna}/links             → get_links(base, type)     │  │
│  │  GET  /hc/{dna}/links/count       → count_links(base, type)   │  │
│  │  GET  /hc/{dna}/agent-activity    → get_agent_activity        │  │
│  │  POST /hc/{dna}/publish           → publish Record → DhtOps   │  │
│  │  POST /hc/{dna}/signal            → send_remote_signal        │  │
│  │                                                                │  │
│  │ Kitsune Direct API (/k2/*)                                    │  │
│  │  GET  /k2/{space}/status          → network connection status │  │
│  │  GET  /k2/{space}/peers           → list known peers          │  │
│  │  GET  /k2/{space}/peer/{agent}    → get specific agent info   │  │
│  │  GET  /k2/{space}/local-agents    → list local agents         │  │
│  │  GET  /k2/transport/stats         → network transport stats   │  │
│  │                                                                │  │
│  │ WebSocket (/ws)                                               │  │
│  │  ← signal(dna, from_agent, payload)  deliver signal to client │  │
│  │  ← sign(agent, message)              request signature        │  │
│  │  → register(dna, agent)              register for signals     │  │
│  │  → sign_response(request_id, sig)    return signature         │  │
│  │                                                                │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                    │                                 │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ holochain_p2p (Holochain Semantic Layer)                      │  │
│  │                                                                │  │
│  │  HolochainP2pDna::get(hash) → queries network peers           │  │
│  │  HolochainP2pDna::get_links(key) → queries network peers      │  │
│  │  HolochainP2pDna::publish(...) → announces to authorities     │  │
│  │  Wire protocol (WireMessage) for peer communication           │  │
│  │                                                                │  │
│  │  Uses produce_ops_from_record() for op construction           │  │
│  │  (guarantees byte-identical serialization with Holochain)     │  │
│  │                                                                │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                    │                                 │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ TempOpStore (Zero-Arc Publishing)                             │  │
│  │                                                                │  │
│  │  Implements kitsune2_api::OpStore                             │  │
│  │  Holds ops until authorities fetch (60s TTL)                  │  │
│  │  Enables publishing without persistent storage                │  │
│  │  Automatic cleanup every 10s                                  │  │
│  │                                                                │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                    │                                 │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ AgentProxyManager                                             │  │
│  │                                                                │  │
│  │  Track browser agents per DNA                                 │  │
│  │  Route signals to correct WebSocket                           │  │
│  │  Manage sign request/response flow                            │  │
│  │  ProxyAgent: zero-arc, remote signing via WS                  │  │
│  │                                                                │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                    │                                 │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ Kitsune2 Core                                                 │  │
│  │                                                                │  │
│  │  Spaces (one per DNA)                                         │  │
│  │  Peer discovery (bootstrap + gossip)                          │  │
│  │  Op fetch (request from authorities)                          │  │
│  │  Transport (tx5 - WebRTC/QUIC)                                │  │
│  │                                                                │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                      │
└──────────────────────────────────┬───────────────────────────────────┘
                                   │
                                   │ Kitsune2 P2P Protocol
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      HOLOCHAIN NETWORK                               │
│                                                                      │
│   ┌─────────────┐    ┌─────────────┐    ┌─────────────┐             │
│   │ Conductor 1 │    │ Conductor 2 │    │ Conductor N │             │
│   │             │    │             │    │             │             │
│   │ DHT Storage │    │ DHT Storage │    │ DHT Storage │             │
│   │ Full Arc    │    │ Full Arc    │    │ Full Arc    │             │
│   └─────────────┘    └─────────────┘    └─────────────┘             │
│                                                                      │
│   (hc-membrane clients are zero-arc: store nothing, fetch all)      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Why This Architecture?

### The Holochain Membrane Concept

**hc-membrane** is NOT "Holochain Lite" - it provides no:
- Source chain (no local chain storage)
- Validation (no validation workflows)
- Full node capabilities (zero-arc, no DHT storage)

Instead, it's a **network edge** - like a cell membrane, it provides selective access between lightweight clients and the Holochain DHT network.

### Key Design Decisions

1. **Zero-Arc Participation**: Clients don't store DHT data, they fetch everything from the network. This is intentional for lightweight deployments.

2. **Op Construction on Gateway**: Clients send `Record` objects, gateway uses Holochain's `produce_ops_from_record()` to generate `DhtOp`s. This guarantees byte-identical serialization with Holochain conductors.

3. **TempOpStore for Publishing**: Since zero-arc nodes don't persist data, ops are held temporarily (60s) until DHT authorities fetch them.

4. **Remote Signing**: Keys stay with clients. Gateway requests signatures via WebSocket when Kitsune2 needs them.

5. **Dual API Design**:
   - `/hc/*` - Holochain semantic operations (get, get_links, publish)
   - `/k2/*` - Kitsune2 direct access (peers, status, network stats)

---

## Data Flow Diagrams

### Get Record Flow

```
Client                      hc-membrane                    DHT Authorities
  │                              │                              │
  │ GET /hc/{dna}/record/{hash}  │                              │
  │─────────────────────────────►│                              │
  │                              │                              │
  │                              │ HolochainP2pDna::get(hash)   │
  │                              │ ─────────────────────────────►
  │                              │                              │
  │                              │ Find peers near hash location│
  │                              │ Send WireMessage::GetReq     │
  │                              │◄─────────────────────────────│
  │                              │                              │
  │                              │ WireMessage::GetRes          │
  │◄─────────────────────────────│ (signed_action + entry)      │
  │                              │                              │
```

### Publish Flow

```
Client                      hc-membrane                    DHT Authorities
  │                              │                              │
  │ POST /hc/{dna}/publish       │                              │
  │ { record: Record }           │                              │
  │─────────────────────────────►│                              │
  │                              │                              │
  │                              │ produce_ops_from_record()    │
  │                              │ (Holochain's Rust code)      │
  │                              │                              │
  │                              │ Store in TempOpStore         │
  │                              │                              │
  │                              │ holochain_p2p.publish()      │
  │                              │ ─────────────────────────────►
  │                              │                              │
  │                              │ Peers near basis location    │
  │                              │ receive publish notification │
  │                              │                              │
  │                              │◄─────────────────────────────│
  │                              │ Peers fetch ops from         │
  │                              │ TempOpStore                  │
  │                              │                              │
  │◄─────────────────────────────│ Return success               │
```

### Signal Flow

```
Holochain Conductor         hc-membrane                     Client
  │                              │                              │
  │ send_remote_signal()         │                              │
  │ via Kitsune2                 │                              │
  │─────────────────────────────►│                              │
  │                              │                              │
  │                              │ ProxySpaceHandler receives   │
  │                              │ RemoteSignalEvt              │
  │                              │                              │
  │                              │ AgentProxyManager routes     │
  │                              │ to correct WebSocket         │
  │                              │                              │
  │                              │ WS: { type: "signal", ... }  │
  │                              │─────────────────────────────►│
  │                              │                              │
```

### Remote Signing Flow

```
Kitsune2 (needs sig)        hc-membrane                     Client
  │                              │                              │
  │ ProxyAgent.sign(message)     │                              │
  │─────────────────────────────►│                              │
  │                              │                              │
  │                              │ AgentProxyManager            │
  │                              │ request_signature()          │
  │                              │                              │
  │                              │ WS: sign_request             │
  │                              │─────────────────────────────►│
  │                              │                              │
  │                              │                              │ Sign with
  │                              │                              │ local key
  │                              │                              │
  │                              │◄─────────────────────────────│
  │                              │ WS: sign_response            │
  │                              │                              │
  │◄─────────────────────────────│ Return signature             │
  │                              │                              │
```

---

## Component Details

### TempOpStore

Temporary storage for ops during the publish flow:

```rust
pub struct TempOpStore {
    ops: Arc<RwLock<HashMap<OpId, StoredOpEntry>>>,
}

struct StoredOpEntry {
    op: StoredOp,
    expires_at: Instant,  // 60s TTL
}
```

- Implements `kitsune2_api::OpStore` trait
- Cleanup task runs every 10s
- Authorities fetch ops after receiving publish notification

### AgentProxyManager

Manages browser agent connections:

```rust
pub struct AgentProxyManager {
    // (dna_hash, agent_pubkey) → WebSocket sender
    connections: HashMap<(DnaHash, AgentPubKey), WsSender>,
    // request_id → oneshot channel for signature response
    pending_signatures: HashMap<String, oneshot::Sender<Signature>>,
}
```

### ProxyAgent

Represents a browser agent in Kitsune2:

```rust
pub struct ProxyAgent {
    agent_pubkey: AgentPubKey,
    dna_hash: DnaHash,
    agent_proxy_manager: Arc<AgentProxyManager>,
}

impl Signer for ProxyAgent {
    fn sign(&self, message: &[u8]) -> BoxFut<Result<Signature>> {
        // Request signature via WebSocket
        self.agent_proxy_manager.request_signature(
            self.agent_pubkey.clone(),
            message.to_vec()
        )
    }
}
```

---

## Related Documentation

- [STEPS/GATEWAY_ARCHITECTURE_ANALYSIS.md](./STEPS/GATEWAY_ARCHITECTURE_ANALYSIS.md) - Detailed analysis of protocol unification, RPC options, and migration plan
- [../fishy/ARCHITECTURE.md](../fishy/ARCHITECTURE.md) - Full browser extension architecture (client side)
- [../fishy/LESSONS_LEARNED.md](../fishy/LESSONS_LEARNED.md) - Serialization debugging lessons
