//! Reverse proxy, routing table, ACME HTTP-01, DNS check, and certificates (Phase 4).

pub mod acme;
pub mod dns;
pub mod domain;
pub mod proto;
pub mod proxy;
pub mod server;

pub use acme::{AcmeDirectory, AcmeError, AcmeManager, needs_renewal, parse_cert_metadata};
pub use dns::{
    DnsCheckResult, DnsRecordHint, DnsStatus, check_dns, check_dns_with_nameservers,
    detect_public_ips, evaluate_dns,
};
pub use domain::{DomainError, normalize_domain, validate_domain};
pub use proto::{
    CertInfo, CertState, DEFAULT_HTTP_ADDR, DEFAULT_HTTPS_ADDR, DEFAULT_SOCKET_PATH,
    DEFAULT_STATE_DIR, ProxyClient, ProxyClientError, ProxyError, ProxyErrorCode, ProxyOp,
    ProxyReply, ProxyRequest, ProxyResponse, ProxyResult, ProxyStatus, Route, UpstreamTls,
};
pub use proxy::sha256_hex;
pub use server::{ProxyConfig, ProxyServer, ProxyServerError};

/// Version reported by `Ping` over `proxy.sock`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the reverse proxy until `shutdown` completes.
pub async fn run(
    config: ProxyConfig,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), ProxyServerError> {
    let server = ProxyServer::bind(config).await?;
    server.run(shutdown).await
}
