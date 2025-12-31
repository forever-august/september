# Changelog

All notable changes to September will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Health check endpoint at `/health` for container orchestration

## [0.1.0] - YYYY-MM-DD

### Added

- Initial release
- Federated multi-server NNTP architecture with automatic failover
- Worker pool for concurrent NNTP connections with priority queues
- Request coalescing to prevent duplicate NNTP requests
- Multi-tier caching for articles, threads, and groups
- TLS support with ACME (Let's Encrypt) and manual certificate modes
- Hierarchical newsgroup browsing
- Threaded article view with pagination
- Post and reply support (requires authentication)
- OpenID Connect (OIDC) authentication with multiple providers
- CDN-friendly Cache-Control headers with stale-while-revalidate
- Background refresh for active groups based on request activity
- JSON structured logging option for production deployments
- Theme support with template and static file layering
- Graceful shutdown with connection draining
- Certificate hot-reload via SIGHUP (manual TLS mode)
