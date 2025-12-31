# September

A Rust web application that provides a modern web interface to NNTP (Usenet/newsgroup) servers. Built with Axum and Tera templates.

## Features

- Federated multi-server architecture with automatic failover
- Worker pool for concurrent NNTP connections
- Request coalescing to prevent duplicate requests
- Multi-tier caching for articles, threads, and groups
- TLS support with ACME (Let's Encrypt) or manual certificates
- Hierarchical newsgroup browsing
- Threaded article view with pagination
- Post and reply support (requires authentication)
- OpenID Connect (OIDC) authentication with multiple providers
- CDN-friendly Cache-Control headers
- Health check endpoint for container orchestration
- Graceful shutdown with connection draining

## Quickstart

### Prerequisites

Install the Rust toolchain via [rustup.rs](https://rustup.rs/).

### Build and Run

```bash
# Clone the repository
git clone https://github.com/your-org/september.git
cd september

# Build
cargo build --release

# Run (uses dist/config/default.toml by default)
cargo run --release

# Or run the binary directly
./target/release/september
```

Access the web interface at http://127.0.0.1:3000

## Command Line Options

```
Usage: september [OPTIONS]

Options:
  -c, --config <CONFIG>        Path to configuration file [default: dist/config/default.toml]
  -l, --log-level <LOG_LEVEL>  Log level filter (e.g., "september=debug,tower_http=info")
  --log-format <FORMAT>        Log format: "text" (default) or "json"
  -h, --help                   Print help
  -V, --version                Print version
```

Examples:

```bash
# Use a custom config file
./target/release/september -c /etc/september/config.toml

# Set log level to info
./target/release/september -l "september=info,tower_http=warn"

# Combine options
./target/release/september --config prod.toml --log-level september=warn
```

Log level priority: CLI (`-l`) > `RUST_LOG` environment variable > default (`september=debug,tower_http=debug`)

## Configuration

Configuration uses TOML format. See `dist/config/default.toml` for available options.

The default configuration connects to `nntp.lore.kernel.org` (Linux kernel mailing list archives).

## Operations

### Health Check

September exposes a health check endpoint for container orchestration:

```
GET /health → 200 OK, body: "ok"
```

Use this for Kubernetes liveness probes, ECS health checks, or load balancer health checks.

### Signals

| Signal | Behavior |
|--------|----------|
| `SIGTERM` / `SIGINT` | Graceful shutdown with 30-second connection drain |
| `SIGHUP` | Reload TLS certificates (manual TLS mode only) |

### Logging

September supports two log formats:

- `text` (default): Human-readable format for development
- `json`: Structured JSON format for production log aggregation

Set via `--log-format json` or `[logging] format = "json"` in config.

## Documentation

- [docs/architecture.md](docs/architecture.md) - System architecture
- [docs/nntp-service.md](docs/nntp-service.md) - NNTP service design
- [docs/routing.md](docs/routing.md) - HTTP routing and caching
- [docs/oidc.md](docs/oidc.md) - OpenID Connect authentication
- [docs/background-refresh.md](docs/background-refresh.md) - Activity-based cache refresh

## License

Apache-2.0
