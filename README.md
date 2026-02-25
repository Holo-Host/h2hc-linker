# h2hc-linker

Network edge gateway providing DHT access for lightweight browser clients.

## Overview

h2hc-linker is designed to serve zero-arc Holochain nodes that don't participate in gossip. Instead, these lightweight clients (like browser extensions) fetch and publish data through the gateway.

## API Design

### Serialization

- **HTTP API**: JSON for all request/response wrappers
- **Binary data**: Msgpack-encoded, base64-wrapped within JSON fields (e.g., DhtOp in publish requests)

This matches the pattern established in hc-http-gw for browser compatibility while preserving Holochain's binary data formats.

### Endpoints

#### Kitsune Direct API (`/k2/*`)

Low-level network introspection for debugging and liveness UIs:

| Endpoint | Description |
|----------|-------------|
| `GET /k2/status` | Network connection status |
| `GET /k2/peers` | All known peers across spaces |
| `GET /k2/space/{id}/status` | Status for a specific DNA space |
| `GET /k2/space/{id}/peers` | Peers for a specific space |
| `GET /k2/space/{id}/local-agents` | Local agents in a space |
| `GET /k2/transport/stats` | Transport layer statistics |

#### Holochain Semantic API (`/hc/*`)

*Coming in M2* - Higher-level API matching Holochain concepts (cells, zome calls, etc.).

## Building

```bash
cargo build --release
```

## Running

```bash
# Default: localhost:8090
./target/release/h2hc-linker

# Custom address/port
./target/release/h2hc-linker --address 0.0.0.0 --port 8080

# Environment variables
H2HC_LINKER_ADDRESS=0.0.0.0 H2HC_LINKER_PORT=8080 ./target/release/h2hc-linker
```

## Configuration

Environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `H2HC_LINKER_ADDRESS` | Bind address | `127.0.0.1` |
| `H2HC_LINKER_PORT` | Bind port | `8090` |
| `BOOTSTRAP_URL` | Kitsune bootstrap server | None |
| `SIGNAL_URL` | Kitsune signal server | None |

## Releasing

Releases are triggered by pushing a git tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

This runs the GitHub Actions release workflow which builds binaries for:
- Linux x86_64
- Linux aarch64
- macOS aarch64 (Apple Silicon)

All binaries are uploaded to a GitHub Release. Tags containing `-` (e.g., `v0.1.0-rc.1`) are marked as prereleases.

See [COMPATIBILITY.md](./COMPATIBILITY.md) for version compatibility with the [Holo Web Conductor](https://github.com/Holo-Host/holo-web-conductor) extension.

## License

CAL-1.0 (Cryptographic Autonomy License)
