# September

A Rust web application that provides a modern web interface to NNTP (Usenet/newsgroup) servers. Built with Axum and Tera templates.

## Features

- Federated multi-server architecture with automatic failover
- Worker pool for concurrent NNTP connections
- Request coalescing to prevent duplicate requests
- Multi-tier caching for articles, threads, and groups
- TLS support with opportunistic fallback
- Hierarchical newsgroup browsing
- Threaded article view with pagination
- CDN-friendly Cache-Control headers

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

# Run (uses config/default.toml by default)
cargo run --release

# Or run the binary directly
./target/release/september
```

Access the web interface at http://127.0.0.1:3000

## Command Line Options

```
Usage: september [OPTIONS]

Options:
  -c, --config <CONFIG>        Path to configuration file [default: config/default.toml]
  -l, --log-level <LOG_LEVEL>  Log level filter (e.g., "september=debug,tower_http=info")
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

Configuration uses TOML format. See `config/default.toml` for available options.

The default configuration connects to `nntp.lore.kernel.org` (Linux kernel mailing list archives).

## Documentation

- [docs/architecture.md](docs/architecture.md) - System architecture
- [docs/nntp-service.md](docs/nntp-service.md) - NNTP service design
- [docs/routing.md](docs/routing.md) - HTTP routing and caching

## License

Apache-2.0
