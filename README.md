# September - HTTP to NNTP Bridge

[![CI](https://github.com/forever-august/september/actions/workflows/ci.yml/badge.svg)](https://github.com/forever-august/september/actions/workflows/ci.yml)

September is a modern HTTP to NNTP bridge server built with Rust and Leptos. It provides a web interface for browsing and interacting with NNTP newsgroups through a standard web browser.

## Features

- **Web Interface**: Modern, responsive web UI built with Leptos
- **NNTP Client**: Full-featured NNTP client for connecting to newsgroups
- **HTTP API**: RESTful API for programmatic access
- **Bridge Architecture**: Seamless translation between HTTP and NNTP protocols
- **Cross-Platform**: Runs on Linux, macOS, and Windows

## Architecture

The application consists of several key components:

- **Web Frontend**: Leptos-based single-page application
- **HTTP Server**: Axum-based web server with RESTful API
- **NNTP Client**: Async NNTP client for newsgroup connectivity
- **Bridge Service**: Core service that translates between HTTP and NNTP

## Building

### Prerequisites

- Rust 1.70+ (2021 edition)
- Cargo

### Build from Source

```bash
git clone https://github.com/forever-august/september.git
cd september
cargo build --release
```

### Running Tests

```bash
cargo test
```

### Development

For development with hot reloading:

```bash
cargo run
```

The server will start on `http://localhost:3000` by default.

## Configuration

The application can be configured via environment variables or a configuration file:

### Environment Variables

- `SEPTEMBER_BIND_ADDRESS`: Server bind address (default: `127.0.0.1:3000`)
- `SEPTEMBER_NNTP_SERVER`: NNTP server hostname
- `SEPTEMBER_NNTP_PORT`: NNTP server port (default: `119`)

### Configuration File

Create a `Leptos.toml` file in the project root:

```toml
[leptos]
env = "DEV"
bin-features = ["ssr"]
lib-features = ["hydrate"]
site-root = "target/site"
site-pkg-dir = "pkg"
site-addr = "127.0.0.1:3000"
reload-port = 3001
```

## Usage

### Starting the Server

```bash
./target/release/september
```

### Web Interface

Navigate to `http://localhost:3000` in your browser to access the web interface.

### API Endpoints

- `GET /api/health` - Health check
- `GET /api/groups` - List available newsgroups
- `GET /api/groups/{group}` - Get articles from a specific newsgroup
- `GET /api/articles/{id}` - Get a specific article

## Development Status

This project is in early development. Current features include:

- [x] Basic project structure
- [x] Leptos web framework integration
- [x] Axum HTTP server
- [x] NNTP client foundation
- [x] CI/CD pipeline
- [ ] Full NNTP protocol implementation
- [ ] Web UI for browsing newsgroups
- [ ] Article viewing and posting
- [ ] Search functionality
- [ ] User authentication
- [ ] Caching layer

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

### Development Setup

1. Clone the repository
2. Install Rust and Cargo
3. Run `cargo test` to ensure everything works
4. Make your changes
5. Run tests and formatting: `cargo test && cargo fmt`
6. Submit a pull request

## License

This project is licensed under the Apache License 2.0. See the [LICENSE](LICENSE) file for details.

## Acknowledgments

- [Leptos](https://leptos.dev/) - Web framework
- [Axum](https://github.com/tokio-rs/axum) - HTTP server framework
- [Tokio](https://tokio.rs/) - Async runtime
- NNTP crate contributors