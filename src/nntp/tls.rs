//! TLS stream wrapper for NNTP connections
//!
//! Provides a unified stream type that can be either TLS-encrypted or plain TCP,
//! allowing opportunistic TLS with fallback for unauthenticated connections.

use std::cell::Cell;
use std::sync::Arc;

use async_trait::async_trait;
use nntp_rs::runtime::stream::AsyncStream;
use rustls::ClientConfig;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

// Thread-local to track whether TLS is required (set by worker before connecting)
thread_local! {
    static TLS_REQUIRED: Cell<bool> = const { Cell::new(false) };
    static LAST_CONNECTION_WAS_TLS: Cell<bool> = const { Cell::new(false) };
}

/// Set whether TLS is required for the next connection on this thread
pub fn set_tls_required(required: bool) {
    TLS_REQUIRED.set(required);
}

/// Check if the last connection on this thread used TLS
pub fn last_connection_was_tls() -> bool {
    LAST_CONNECTION_WAS_TLS.get()
}

/// A stream that can be either TLS-encrypted or plain TCP
pub enum NntpStream {
    /// Plain TCP connection
    Plain(TcpStream),
    /// TLS-encrypted connection
    Tls(TlsStream<TcpStream>),
}

#[async_trait]
impl AsyncStream for NntpStream {
    async fn connect(addr: &str) -> std::io::Result<Self>
    where
        Self: Sized,
    {
        let tls_required = TLS_REQUIRED.get();

        // Parse host from addr for TLS server name
        let host = addr
            .split(':')
            .next()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid address"))?;

        // Try TLS first
        match Self::connect_tls(addr, host).await {
            Ok(stream) => {
                LAST_CONNECTION_WAS_TLS.set(true);
                return Ok(stream);
            }
            Err(e) => {
                if tls_required {
                    // TLS is required, don't fall back
                    LAST_CONNECTION_WAS_TLS.set(false);
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        format!("TLS connection required but failed: {e}"),
                    ));
                }
                tracing::debug!(error = %e, "TLS connection failed, falling back to plain TCP");
            }
        }

        // Fall back to plain TCP
        let stream = Self::connect_plain(addr).await?;
        LAST_CONNECTION_WAS_TLS.set(false);
        Ok(stream)
    }

    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            NntpStream::Plain(stream) => stream.read(buf).await,
            NntpStream::Tls(stream) => stream.read(buf).await,
        }
    }

    async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            NntpStream::Plain(stream) => stream.write_all(buf).await,
            NntpStream::Tls(stream) => stream.write_all(buf).await,
        }
    }

    async fn shutdown(&mut self) -> std::io::Result<()> {
        match self {
            NntpStream::Plain(stream) => stream.shutdown().await,
            NntpStream::Tls(stream) => stream.shutdown().await,
        }
    }
}

impl NntpStream {
    /// Create a TLS connector using system root certificates
    fn create_tls_connector() -> TlsConnector {
        let root_store =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        TlsConnector::from(Arc::new(config))
    }

    /// Connect with TLS to the specified address
    async fn connect_tls(addr: &str, server_name: &str) -> std::io::Result<Self> {
        let tcp_stream = TcpStream::connect(addr).await?;

        let connector = Self::create_tls_connector();
        let server_name = ServerName::try_from(server_name.to_string())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let tls_stream = connector.connect(server_name, tcp_stream).await?;

        Ok(NntpStream::Tls(tls_stream))
    }

    /// Connect with plain TCP to the specified address
    async fn connect_plain(addr: &str) -> std::io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Ok(NntpStream::Plain(stream))
    }
}
