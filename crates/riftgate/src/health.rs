//! `/health` and `/ready` endpoints.
//!
//! These are deliberately tiny so that any operator running Riftgate
//! behind Kubernetes / Nomad / a TCP load balancer can wire them to
//! the platform's liveness and readiness probes without configuration.
//!
//! Semantics:
//!
//! - `GET /health` — process is alive and the accept loop is healthy.
//!   Returns `200 OK` always (until the process actually exits).
//! - `GET /ready` — the data plane is ready to take traffic. Returns
//!   `200 OK` while serving normally; `503 Service Unavailable` while
//!   draining.
//!
//! Per [Persona P5](../../../docs/01-requirements/personas.md) the
//! response bodies are short and human-readable; an SRE who does
//! `curl localhost:8080/ready` should see `OK` or `DRAINING`.

use crate::shutdown::DrainReceiver;
use crate::shutdown::is_draining;
use bytes::Bytes;
use http::{Response, StatusCode};
use http_body_util::Full;

/// Response type returned by every health handler. Wraps an in-memory
/// `Bytes` body for the common short-string case.
pub type HealthResponse = Response<Full<Bytes>>;

/// Handle `GET /health`.
///
/// Returns `200 OK` always. The body is `OK\n` for human-friendly
/// `curl` output.
pub fn health() -> HealthResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain; charset=utf-8")
        .header("x-riftgate-endpoint", "health")
        .body(Full::new(Bytes::from_static(b"OK\n")))
        .expect("static response should always build")
}

/// Handle `GET /ready`.
///
/// Returns `200 OK` while the gateway is accepting traffic, `503
/// Service Unavailable` while draining.
pub fn ready(drain: &DrainReceiver) -> HealthResponse {
    if is_draining(drain) {
        Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header("content-type", "text/plain; charset=utf-8")
            .header("x-riftgate-endpoint", "ready")
            .body(Full::new(Bytes::from_static(b"DRAINING\n")))
            .expect("static response should always build")
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/plain; charset=utf-8")
            .header("x-riftgate-endpoint", "ready")
            .body(Full::new(Bytes::from_static(b"OK\n")))
            .expect("static response should always build")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shutdown;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn health_is_always_200() {
        let resp = health();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"OK\n");
    }

    #[tokio::test]
    async fn ready_is_200_when_not_draining() {
        let (_tx, rx) = shutdown::channel();
        let resp = ready(&rx);
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"OK\n");
    }

    #[tokio::test]
    async fn ready_is_503_when_draining() {
        let (tx, rx) = shutdown::channel();
        shutdown::begin_drain(&tx);
        let resp = ready(&rx);
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"DRAINING\n");
    }
}
