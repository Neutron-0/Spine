//! TLS 1.3 transport security, certificate management, and stream abstractions.

use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
pub use tokio_rustls;
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::server::TlsStream as ServerTlsStream;
pub use tokio_rustls::{TlsAcceptor, TlsConnector};

#[derive(Error, Debug)]
pub enum TlsError {
    #[error("TLS configuration error: {0}")]
    Config(String),
    #[error("Certificate error: {0}")]
    Certificate(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid server name: {0}")]
    InvalidServerName(String),
    #[error("Plaintext connections are strictly forbidden outside loopback (remote peer: {0})")]
    PlaintextForbiddenOutsideLoopback(SocketAddr),
}

/// Generates a self-signed certificate and private key in DER format for development and testing.
pub fn generate_self_signed_cert_der(
    subject_alt_names: &[&str],
) -> Result<(Vec<u8>, Vec<u8>), TlsError> {
    let sans: Vec<String> = subject_alt_names.iter().map(|s| s.to_string()).collect();
    let certified_key: CertifiedKey =
        generate_simple_self_signed(sans).map_err(|e| TlsError::Certificate(e.to_string()))?;

    let cert_der = certified_key.cert.der().to_vec();
    let key_der = certified_key.key_pair.serialize_der();
    Ok((cert_der, key_der))
}

/// Builds a `rustls::ServerConfig` from certificate chain and private key DER bytes.
pub fn build_server_tls_config(
    cert_chain_der: Vec<Vec<u8>>,
    key_der: Vec<u8>,
) -> Result<ServerConfig, TlsError> {
    let certs: Vec<CertificateDer<'static>> = cert_chain_der
        .into_iter()
        .map(CertificateDer::from)
        .collect();

    let key = PrivateKeyDer::try_from(key_der)
        .map_err(|e| TlsError::Config(format!("Invalid private key: {:?}", e)))?;

    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| TlsError::Config(e.to_string()))
}

/// Builds a `rustls::ClientConfig` with either system roots or custom root certificates.
pub fn build_client_tls_config(
    custom_root_certs_der: Option<Vec<Vec<u8>>>,
) -> Result<ClientConfig, TlsError> {
    let mut root_store = RootCertStore::empty();

    if let Some(roots) = custom_root_certs_der {
        for root_der in roots {
            root_store
                .add(CertificateDer::from(root_der))
                .map_err(|e| TlsError::Certificate(e.to_string()))?;
        }
    } else {
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(config)
}

/// Checks if a socket address is on the loopback interface (IPv4 127.0.0.0/8 or IPv6 ::1).
pub fn is_loopback_addr(addr: &SocketAddr) -> bool {
    match addr {
        SocketAddr::V4(v4) => v4.ip().is_loopback(),
        SocketAddr::V6(v6) => v6.ip().is_loopback(),
    }
}

/// Unified stream supporting plaintext TCP (loopback only) or TLS 1.3.
pub enum SpineStream {
    Plain(TcpStream),
    TlsServer(ServerTlsStream<TcpStream>),
    TlsClient(ClientTlsStream<TcpStream>),
}

impl std::fmt::Debug for SpineStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpineStream::Plain(_) => write!(f, "SpineStream::Plain"),
            SpineStream::TlsServer(_) => write!(f, "SpineStream::TlsServer"),
            SpineStream::TlsClient(_) => write!(f, "SpineStream::TlsClient"),
        }
    }
}

impl SpineStream {
    pub fn is_tls(&self) -> bool {
        matches!(self, SpineStream::TlsServer(_) | SpineStream::TlsClient(_))
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        match self {
            SpineStream::Plain(s) => s.peer_addr(),
            SpineStream::TlsServer(s) => s.get_ref().0.peer_addr(),
            SpineStream::TlsClient(s) => s.get_ref().0.peer_addr(),
        }
    }

    /// Upgrades a client TCP stream to TLS 1.3.
    pub async fn upgrade_client(
        stream: TcpStream,
        domain: &str,
        config: Arc<ClientConfig>,
    ) -> Result<Self, TlsError> {
        let connector = TlsConnector::from(config);
        let server_name = ServerName::try_from(domain.to_string())
            .map_err(|_| TlsError::InvalidServerName(domain.to_string()))?
            .to_owned();

        let tls_stream = connector
            .connect(server_name, stream)
            .await
            .map_err(TlsError::Io)?;

        Ok(SpineStream::TlsClient(tls_stream))
    }

    /// Upgrades a server TCP stream to TLS 1.3.
    pub async fn upgrade_server(
        stream: TcpStream,
        acceptor: TlsAcceptor,
    ) -> Result<Self, TlsError> {
        let tls_stream = acceptor.accept(stream).await.map_err(TlsError::Io)?;
        Ok(SpineStream::TlsServer(tls_stream))
    }
}

impl AsyncRead for SpineStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            SpineStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            SpineStream::TlsServer(s) => Pin::new(s).poll_read(cx, buf),
            SpineStream::TlsClient(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for SpineStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            SpineStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            SpineStream::TlsServer(s) => Pin::new(s).poll_write(cx, buf),
            SpineStream::TlsClient(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            SpineStream::Plain(s) => Pin::new(s).poll_flush(cx),
            SpineStream::TlsServer(s) => Pin::new(s).poll_flush(cx),
            SpineStream::TlsClient(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            SpineStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            SpineStream::TlsServer(s) => Pin::new(s).poll_shutdown(cx),
            SpineStream::TlsClient(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_self_signed_cert_generation() {
        let (cert, key) = generate_self_signed_cert_der(&["localhost", "127.0.0.1"]).unwrap();
        assert!(!cert.is_empty());
        assert!(!key.is_empty());

        let server_cfg = build_server_tls_config(vec![cert.clone()], key).unwrap();
        assert!(server_cfg.alpn_protocols.is_empty());

        let client_cfg = build_client_tls_config(Some(vec![cert])).unwrap();
        assert!(client_cfg.alpn_protocols.is_empty());
    }

    #[test]
    fn test_loopback_address_check() {
        let loopback_v4: SocketAddr = "127.0.0.1:4434".parse().unwrap();
        let loopback_v6: SocketAddr = "[::1]:4434".parse().unwrap();
        let external: SocketAddr = "192.168.1.50:4434".parse().unwrap();

        assert!(is_loopback_addr(&loopback_v4));
        assert!(is_loopback_addr(&loopback_v6));
        assert!(!is_loopback_addr(&external));
    }
}
