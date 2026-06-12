//! Integration test for malformed DCGM exporter responses.

use riftgate_core::{BackendId, GpuPressureError, GpuPressureSource};
use riftgate_obs::DcgmScrapeSource;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

#[test]
fn dcgm_scrape_reports_parse_error_for_malformed_metrics() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let address = listener.local_addr().expect("listener addr");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).expect("read request");
        let body = "DCGM_FI_DEV_GPU_UTIL{gpu=\"0\"} not-a-number\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let source = DcgmScrapeSource::new(BackendId(4), format!("http://{address}/metrics"), 0)
        .with_timeout(Duration::from_secs(1));
    let error = source.poll_once().expect_err("malformed scrape fails");

    server.join().expect("server thread joins");

    assert!(matches!(error, GpuPressureError::Parse(_)));
}
