//! Integration test for successful DCGM exporter scraping.

use riftgate_core::{BackendId, GpuPressureSource, GpuThrottleState};
use riftgate_obs::DcgmScrapeSource;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

#[test]
fn dcgm_scrape_reads_loopback_exporter_response() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let address = listener.local_addr().expect("listener addr");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).expect("read request");
        let body = concat!(
            "DCGM_FI_DEV_GPU_UTIL{gpu=\"0\"} 81\n",
            "DCGM_FI_DEV_FB_USED{gpu=\"0\"} 12288\n",
            "DCGM_FI_DEV_FB_TOTAL{gpu=\"0\"} 16384\n",
            "DCGM_FI_DEV_CLOCK_THROTTLE_REASONS{gpu=\"0\"} 4\n",
            "DCGM_FI_DEV_ECC_DBE_VOL_TOTAL{gpu=\"0\"} 2\n"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let source = DcgmScrapeSource::new(BackendId(9), format!("http://{address}/metrics"), 0)
        .with_timeout(Duration::from_secs(1));
    let snapshot = source.poll_once().expect("scrape succeeds");

    server.join().expect("server thread joins");

    assert_eq!(snapshot.len(), 1);
    let pressure = &snapshot[0];
    assert_eq!(pressure.backend, BackendId(9));
    assert_eq!(pressure.utilization_pct, 81.0);
    assert_eq!(pressure.memory_used_pct, 75.0);
    assert_eq!(pressure.throttle_state, GpuThrottleState::Power);
    assert_eq!(pressure.ecc_errors_total, 2);
}
