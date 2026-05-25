//! Upstream HTTP client.
//!
//! One `hyper-util` legacy `Client` shared across the whole gateway.
//! TLS is provided by `hyper-rustls` over `webpki-roots` so we ship
//! without an OpenSSL dependency (NFR-SEC03).
//!
//! The client is built once at startup. Connection pooling and TLS
//! session caching live inside `hyper-util::client::legacy::Client`.

use bytes::Bytes;
use http_body_util::Full;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use std::sync::Arc;

/// The body type used for upstream requests. Bounded; v0.1 does not
/// stream request bodies (chat/completions JSON fits in memory).
pub type UpstreamReqBody = Full<Bytes>;

/// Concrete upstream client type.
///
/// `Arc`-wrapped so it is cheap to clone into per-connection tasks.
pub type UpstreamClient = Arc<Client<hyper_rustls::HttpsConnector<HttpConnector>, UpstreamReqBody>>;

/// Build a hyper-rustls upstream client backed by the bundled
/// `webpki-roots` trust store.
///
/// `tls_verify=false` is intentionally NOT supported by this builder;
/// dev / test harnesses can use HTTP (`http://...`) backends instead.
/// (We do not ship a "skip cert verification" path to make sure that
/// surface never appears in production by accident.)
pub fn build_client() -> UpstreamClient {
    install_default_crypto_provider_once();

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();

    let client = Client::builder(TokioExecutor::new()).build::<_, UpstreamReqBody>(https);

    Arc::new(client)
}

/// Install the default rustls crypto provider exactly once per
/// process. `rustls` 0.23 requires a process-wide selection; we pick
/// `aws_lc_rs` because it is the default rustls feature and ships
/// cleanly on every platform Riftgate supports.
///
/// Safe to call from multiple threads thanks to `OnceLock`.
fn install_default_crypto_provider_once() {
    use std::sync::OnceLock;
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        // Ignore the error: if some other code already installed a
        // provider, that is fine — rustls just needs *some* provider
        // installed before TLS is used.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}
