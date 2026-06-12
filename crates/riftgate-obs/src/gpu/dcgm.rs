//! DCGM exporter scrape-backed GPU-pressure source.
//!
//! This is the default `GpuPressureSource` implementation for `v0.4`.
//! It performs a blocking HTTP scrape against a Prometheus-format endpoint
//! and maps a bounded set of DCGM metrics into a single `GpuPressure` sample.

use riftgate_core::{
    BackendId, GpuPressure, GpuPressureError, GpuPressureSource, GpuThrottleState,
};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_PATH: &str = "/metrics";
const NVML_CLOCKS_THROTTLE_REASON_SW_POWER_CAP: u64 = 0x0000_0004;
const NVML_CLOCKS_THROTTLE_REASON_HW_SLOWDOWN: u64 = 0x0000_0008;
const NVML_CLOCKS_THROTTLE_REASON_SW_THERMAL_SLOWDOWN: u64 = 0x0000_0020;
const NVML_CLOCKS_THROTTLE_REASON_HW_THERMAL_SLOWDOWN: u64 = 0x0000_0040;
const NVML_CLOCKS_THROTTLE_REASON_HW_POWER_BRAKE_SLOWDOWN: u64 = 0x0000_0080;

/// Blocking scrape source for a single backend's DCGM exporter endpoint.
#[derive(Debug, Clone)]
pub struct DcgmScrapeSource {
    backend: BackendId,
    endpoint: String,
    gpu_index: u32,
    mig_uuid: Option<String>,
    timeout: Duration,
}

impl DcgmScrapeSource {
    /// Construct a source for one backend and one GPU index.
    #[must_use]
    pub fn new(backend: BackendId, endpoint: impl Into<String>, gpu_index: u32) -> Self {
        Self {
            backend,
            endpoint: endpoint.into(),
            gpu_index,
            mig_uuid: None,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Attach a MIG UUID selector.
    #[must_use]
    pub fn with_mig_uuid(mut self, mig_uuid: impl Into<String>) -> Self {
        self.mig_uuid = Some(mig_uuid.into());
        self
    }

    /// Override the network timeout used for connect, read, and write.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn scrape_metrics(&self) -> Result<String, GpuPressureError> {
        let endpoint = HttpEndpoint::parse(&self.endpoint)?;
        let address = resolve_socket_addr(&endpoint.host, endpoint.port)?;
        let mut stream = TcpStream::connect_timeout(&address, self.timeout)
            .map_err(|err| GpuPressureError::ScrapeFailed(err.to_string()))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|err| GpuPressureError::ScrapeFailed(err.to_string()))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|err| GpuPressureError::ScrapeFailed(err.to_string()))?;

        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: text/plain\r\n\r\n",
            endpoint.path, endpoint.host
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|err| GpuPressureError::ScrapeFailed(err.to_string()))?;

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .map_err(|err| GpuPressureError::ScrapeFailed(err.to_string()))?;

        parse_http_response_body(&response)
    }

    fn parse_metrics(&self, body: &str) -> Result<GpuPressure, GpuPressureError> {
        let mut snapshot = DcgmSnapshot::default();
        for raw_line in body.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let sample = MetricSample::parse(line)?;
            if !sample.matches(self.gpu_index, self.mig_uuid.as_deref()) {
                continue;
            }

            snapshot.observe(&sample)?;
        }

        snapshot.into_gpu_pressure(self.backend)
    }
}

impl GpuPressureSource for DcgmScrapeSource {
    fn poll_once(&self) -> Result<Vec<GpuPressure>, GpuPressureError> {
        let body = self.scrape_metrics()?;
        let pressure = self.parse_metrics(&body)?;
        Ok(vec![pressure])
    }

    fn name(&self) -> &'static str {
        "dcgm-exporter"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

impl HttpEndpoint {
    fn parse(input: &str) -> Result<Self, GpuPressureError> {
        let without_scheme = input.strip_prefix("http://").ok_or_else(|| {
            GpuPressureError::Parse(format!("unsupported endpoint scheme: {input}"))
        })?;

        let (authority, path) = match without_scheme.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (without_scheme, DEFAULT_PATH.to_string()),
        };
        if authority.is_empty() {
            return Err(GpuPressureError::Parse("missing scrape host".to_string()));
        }

        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let parsed_port = port.parse::<u16>().map_err(|_| {
                    GpuPressureError::Parse(format!("invalid scrape port in endpoint: {input}"))
                })?;
                (host.to_string(), parsed_port)
            }
            None => (authority.to_string(), 80),
        };

        if host.is_empty() {
            return Err(GpuPressureError::Parse("missing scrape host".to_string()));
        }

        Ok(Self { host, port, path })
    }
}

fn resolve_socket_addr(host: &str, port: u16) -> Result<SocketAddr, GpuPressureError> {
    (host, port)
        .to_socket_addrs()
        .map_err(|err| GpuPressureError::ScrapeFailed(err.to_string()))?
        .next()
        .ok_or_else(|| {
            GpuPressureError::ScrapeFailed(format!("no address resolved for {host}:{port}"))
        })
}

fn parse_http_response_body(response: &str) -> Result<String, GpuPressureError> {
    let (head, body) = response
        .split_once("\r\n\r\n")
        .or_else(|| response.split_once("\n\n"))
        .ok_or_else(|| GpuPressureError::ScrapeFailed("malformed HTTP response".to_string()))?;

    let status_line = head.lines().next().unwrap_or_default();
    if !(status_line.starts_with("HTTP/1.1 200") || status_line.starts_with("HTTP/1.0 200")) {
        return Err(GpuPressureError::ScrapeFailed(format!(
            "unexpected scrape status: {status_line}"
        )));
    }

    Ok(body.to_string())
}

#[derive(Debug, Default)]
struct DcgmSnapshot {
    utilization_pct: Option<f32>,
    memory_used_mib: Option<f64>,
    memory_free_mib: Option<f64>,
    memory_total_mib: Option<f64>,
    throttle_reasons: Option<u64>,
    ecc_errors_total: u64,
}

impl DcgmSnapshot {
    fn observe(&mut self, sample: &MetricSample) -> Result<(), GpuPressureError> {
        match sample.name.as_str() {
            "DCGM_FI_DEV_GPU_UTIL" => {
                self.utilization_pct = Some(parse_percent(sample.value, &sample.name)?);
            }
            "DCGM_FI_DEV_FB_USED" => {
                self.memory_used_mib = Some(sample.value);
            }
            "DCGM_FI_DEV_FB_FREE" => {
                self.memory_free_mib = Some(sample.value);
            }
            "DCGM_FI_DEV_FB_TOTAL" => {
                self.memory_total_mib = Some(sample.value);
            }
            "DCGM_FI_DEV_CLOCK_THROTTLE_REASONS" => {
                if sample.value < 0.0 {
                    return Err(GpuPressureError::Parse(format!(
                        "{} must be non-negative",
                        sample.name
                    )));
                }
                self.throttle_reasons = Some(sample.value as u64);
            }
            name if name.starts_with("DCGM_FI_DEV_ECC_") => {
                if sample.value < 0.0 {
                    return Err(GpuPressureError::Parse(format!(
                        "{} must be non-negative",
                        sample.name
                    )));
                }
                self.ecc_errors_total = self.ecc_errors_total.saturating_add(sample.value as u64);
            }
            _ => {}
        }
        Ok(())
    }

    fn into_gpu_pressure(self, backend: BackendId) -> Result<GpuPressure, GpuPressureError> {
        let utilization_pct = self.utilization_pct.ok_or_else(|| {
            GpuPressureError::Parse("missing DCGM_FI_DEV_GPU_UTIL metric".to_string())
        })?;
        let memory_used_pct = self.memory_used_pct()?;

        Ok(GpuPressure {
            backend,
            utilization_pct,
            memory_used_pct,
            throttle_state: throttle_state_from_bits(self.throttle_reasons.unwrap_or(0)),
            ecc_errors_total: self.ecc_errors_total,
            observed_at: Instant::now(),
        })
    }

    fn memory_used_pct(&self) -> Result<f32, GpuPressureError> {
        let used = self.memory_used_mib.ok_or_else(|| {
            GpuPressureError::Parse("missing DCGM_FI_DEV_FB_USED metric".to_string())
        })?;
        let total = match self.memory_total_mib {
            Some(total) => total,
            None => {
                let free = self.memory_free_mib.ok_or_else(|| {
                    GpuPressureError::Parse(
                        "missing DCGM_FI_DEV_FB_TOTAL or DCGM_FI_DEV_FB_FREE metric".to_string(),
                    )
                })?;
                used + free
            }
        };

        if total <= 0.0 {
            return Err(GpuPressureError::Parse(
                "framebuffer total must be positive".to_string(),
            ));
        }

        Ok(((used / total) * 100.0).clamp(0.0, 100.0) as f32)
    }
}

fn parse_percent(value: f64, metric_name: &str) -> Result<f32, GpuPressureError> {
    if !(0.0..=100.0).contains(&value) {
        return Err(GpuPressureError::Parse(format!(
            "{metric_name} out of range: {value}"
        )));
    }
    Ok(value as f32)
}

fn throttle_state_from_bits(bits: u64) -> GpuThrottleState {
    if bits & NVML_CLOCKS_THROTTLE_REASON_HW_SLOWDOWN != 0 {
        return GpuThrottleState::HwSlowdown;
    }
    if bits
        & (NVML_CLOCKS_THROTTLE_REASON_SW_THERMAL_SLOWDOWN
            | NVML_CLOCKS_THROTTLE_REASON_HW_THERMAL_SLOWDOWN)
        != 0
    {
        return GpuThrottleState::Thermal;
    }
    if bits
        & (NVML_CLOCKS_THROTTLE_REASON_SW_POWER_CAP
            | NVML_CLOCKS_THROTTLE_REASON_HW_POWER_BRAKE_SLOWDOWN)
        != 0
    {
        return GpuThrottleState::Power;
    }
    if bits == 0 {
        GpuThrottleState::None
    } else {
        GpuThrottleState::SwLimit
    }
}

#[derive(Debug, Clone)]
struct MetricSample {
    name: String,
    labels: BTreeMap<String, String>,
    value: f64,
}

impl MetricSample {
    fn parse(line: &str) -> Result<Self, GpuPressureError> {
        let (metric, value_str) = split_metric_and_value(line)?;
        let value = value_str.parse::<f64>().map_err(|_| {
            GpuPressureError::Parse(format!("invalid metric value in line: {line}"))
        })?;

        let (name, labels) = match metric.split_once('{') {
            Some((name, labels_blob)) => {
                let labels_str = labels_blob.strip_suffix('}').ok_or_else(|| {
                    GpuPressureError::Parse(format!("invalid labels in line: {line}"))
                })?;
                (name.to_string(), parse_labels(labels_str)?)
            }
            None => (metric.to_string(), BTreeMap::new()),
        };

        Ok(Self {
            name,
            labels,
            value,
        })
    }

    fn matches(&self, gpu_index: u32, mig_uuid: Option<&str>) -> bool {
        if let Some(expected_mig_uuid) = mig_uuid {
            let actual = self
                .label_value(&["UUID", "uuid", "mig_uuid"])
                .or_else(|| self.label_value(&["gpu_uuid"]));
            if actual != Some(expected_mig_uuid) {
                return false;
            }
        }

        match self.label_value(&["gpu", "gpu_index", "minor_number", "device"]) {
            Some(raw) => raw.parse::<u32>().ok() == Some(gpu_index),
            None => mig_uuid.is_some(),
        }
    }

    fn label_value<'a>(&'a self, keys: &[&str]) -> Option<&'a str> {
        keys.iter()
            .find_map(|key| self.labels.get(*key).map(std::string::String::as_str))
    }
}

fn split_metric_and_value(line: &str) -> Result<(&str, &str), GpuPressureError> {
    let mut brace_depth = 0usize;
    for (idx, ch) in line.char_indices() {
        match ch {
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            ' ' | '\t' if brace_depth == 0 => {
                let metric = &line[..idx];
                let value = line[idx..].trim();
                let value = value.split_whitespace().next().ok_or_else(|| {
                    GpuPressureError::Parse(format!("missing metric value in line: {line}"))
                })?;
                return Ok((metric, value));
            }
            _ => {}
        }
    }

    Err(GpuPressureError::Parse(format!(
        "missing metric separator in line: {line}"
    )))
}

fn parse_labels(labels: &str) -> Result<BTreeMap<String, String>, GpuPressureError> {
    let mut parsed = BTreeMap::new();
    if labels.trim().is_empty() {
        return Ok(parsed);
    }

    for pair in labels.split(',') {
        let (key, raw_value) = pair
            .split_once('=')
            .ok_or_else(|| GpuPressureError::Parse(format!("invalid label pair: {pair}")))?;
        let value = raw_value
            .trim()
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .ok_or_else(|| GpuPressureError::Parse(format!("invalid label value: {pair}")))?;
        parsed.insert(key.trim().to_string(), value.to_string());
    }

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_metrics_maps_snapshot_to_gpu_pressure() {
        let source = DcgmScrapeSource::new(BackendId(7), "http://127.0.0.1:9400/metrics", 0);
        let body = concat!(
            "# HELP DCGM_FI_DEV_GPU_UTIL GPU utilization\n",
            "DCGM_FI_DEV_GPU_UTIL{gpu=\"0\"} 42\n",
            "DCGM_FI_DEV_FB_USED{gpu=\"0\"} 8192\n",
            "DCGM_FI_DEV_FB_FREE{gpu=\"0\"} 8192\n",
            "DCGM_FI_DEV_CLOCK_THROTTLE_REASONS{gpu=\"0\"} 32\n",
            "DCGM_FI_DEV_ECC_DBE_VOL_TOTAL{gpu=\"0\"} 3\n",
            "DCGM_FI_DEV_ECC_SBE_VOL_TOTAL{gpu=\"0\"} 4\n"
        );

        let pressure = source.parse_metrics(body).expect("dcgm snapshot parses");
        assert_eq!(pressure.backend, BackendId(7));
        assert_eq!(pressure.utilization_pct, 42.0);
        assert_eq!(pressure.memory_used_pct, 50.0);
        assert_eq!(pressure.throttle_state, GpuThrottleState::Thermal);
        assert_eq!(pressure.ecc_errors_total, 7);
    }

    #[test]
    fn parse_metrics_filters_to_requested_gpu_index() {
        let source = DcgmScrapeSource::new(BackendId(1), "http://127.0.0.1:9400/metrics", 1);
        let body = concat!(
            "DCGM_FI_DEV_GPU_UTIL{gpu=\"0\"} 10\n",
            "DCGM_FI_DEV_FB_USED{gpu=\"0\"} 1024\n",
            "DCGM_FI_DEV_FB_FREE{gpu=\"0\"} 1024\n",
            "DCGM_FI_DEV_GPU_UTIL{gpu=\"1\"} 77\n",
            "DCGM_FI_DEV_FB_USED{gpu=\"1\"} 3072\n",
            "DCGM_FI_DEV_FB_FREE{gpu=\"1\"} 1024\n"
        );

        let pressure = source.parse_metrics(body).expect("gpu 1 snapshot parses");
        assert_eq!(pressure.utilization_pct, 77.0);
        assert_eq!(pressure.memory_used_pct, 75.0);
    }

    #[test]
    fn parse_metrics_honors_mig_uuid() {
        let source = DcgmScrapeSource::new(BackendId(2), "http://127.0.0.1:9400/metrics", 0)
            .with_mig_uuid("MIG-1234");
        let body = concat!(
            "DCGM_FI_DEV_GPU_UTIL{gpu=\"0\",UUID=\"MIG-other\"} 12\n",
            "DCGM_FI_DEV_FB_USED{gpu=\"0\",UUID=\"MIG-other\"} 2048\n",
            "DCGM_FI_DEV_FB_FREE{gpu=\"0\",UUID=\"MIG-other\"} 2048\n",
            "DCGM_FI_DEV_GPU_UTIL{gpu=\"0\",UUID=\"MIG-1234\"} 64\n",
            "DCGM_FI_DEV_FB_USED{gpu=\"0\",UUID=\"MIG-1234\"} 6144\n",
            "DCGM_FI_DEV_FB_FREE{gpu=\"0\",UUID=\"MIG-1234\"} 2048\n"
        );

        let pressure = source.parse_metrics(body).expect("mig snapshot parses");
        assert_eq!(pressure.utilization_pct, 64.0);
        assert_eq!(pressure.memory_used_pct, 75.0);
    }

    #[test]
    fn parse_metrics_rejects_missing_required_values() {
        let source = DcgmScrapeSource::new(BackendId(3), "http://127.0.0.1:9400/metrics", 0);
        let body = "DCGM_FI_DEV_FB_USED{gpu=\"0\"} 1024\n";

        let error = source
            .parse_metrics(body)
            .expect_err("missing utilization fails");
        assert!(matches!(error, GpuPressureError::Parse(_)));
    }

    #[test]
    fn parse_http_endpoint_defaults_path() {
        let endpoint = HttpEndpoint::parse("http://localhost:9400").expect("endpoint parses");
        assert_eq!(endpoint.path, "/metrics");
        assert_eq!(endpoint.port, 9400);
    }
}
