//! Loader round-trip tests.
//!
//! Each test exercises a specific path through `load`:
//!
//! - file-only with all sections
//! - file-only with partial sections (defaults fill in)
//! - env-only-overrides on a defaults base
//! - file + env merge with env winning
//! - missing file → loader returns `FileRead` error
//! - invalid TOML → loader returns `TomlParse` error
//! - validation failure → loader returns `Validation` errors

use riftgate_config::{Config, ConfigError, Env, load};
use std::io::Write;
use tempfile::NamedTempFile;

fn write_file(s: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(s.as_bytes()).unwrap();
    f
}

fn valid_baseline_toml() -> &'static str {
    r#"
[server]
listen_addr = "localhost:9090"

[backend]
url = "https://api.openai.com"
auth_header = "Bearer test"
timeout_ms = 5000

[timer]
tick_resolution_ms = 10

[obs]
otel_endpoint = "http://localhost:4317"
sample_rate = 0.05
bus_capacity = 8192

[log]
level = "info"
format = "json"
"#
}

#[test]
fn file_only_round_trip() {
    let f = write_file(valid_baseline_toml());
    let env = Env::new();
    let cfg = load(Some(f.path()), &env).expect("valid baseline should load");
    assert_eq!(cfg.server.listen_addr.port(), 9090);
    assert_eq!(cfg.backend.url, "https://api.openai.com");
    assert_eq!(cfg.backend.timeout_ms, 5000);
    assert_eq!(cfg.obs.bus_capacity, 8192);
}

#[test]
fn file_with_partial_sections_uses_defaults() {
    let f = write_file(
        r#"
[backend]
url = "https://upstream.example"
"#,
    );
    let env = Env::new();
    let cfg = load(Some(f.path()), &env).expect("partial config should still load");
    assert_eq!(cfg.backend.url, "https://upstream.example");
    // defaults survived
    assert_eq!(cfg.backend.timeout_ms, 30_000);
    assert_eq!(cfg.timer.tick_resolution_ms, 10);
}

#[test]
fn env_overrides_file() {
    let f = write_file(valid_baseline_toml());
    let env = Env::new()
        .with("RIFTGATE_BACKEND_TIMEOUT_MS", "9999")
        .with("RIFTGATE_OBS_BUS_CAPACITY", "1024");
    let cfg = load(Some(f.path()), &env).expect("file + env should merge");
    assert_eq!(cfg.backend.timeout_ms, 9999, "env should win over file");
    assert_eq!(cfg.obs.bus_capacity, 1024);
}

#[test]
fn defaults_plus_env_only() {
    let env = Env::new()
        .with("RIFTGATE_BACKEND_URL", "https://upstream.example")
        .with("RIFTGATE_BACKEND_AUTH_HEADER", "Bearer secret-from-env");
    let cfg = load(None, &env).expect("defaults + env should be sufficient");
    assert_eq!(cfg.backend.url, "https://upstream.example");
    assert_eq!(cfg.backend.auth_header.expose(), "Bearer secret-from-env");
}

#[test]
fn missing_file_is_an_error() {
    let env = Env::new();
    let path = std::path::Path::new("/nonexistent/path/that/should/not/exist.toml");
    let err = load(Some(path), &env).unwrap_err();
    assert!(matches!(err.first(), Some(ConfigError::FileRead { .. })));
}

#[test]
fn invalid_toml_is_an_error() {
    let f = write_file("not valid toml = = = oops");
    let env = Env::new();
    let err = load(Some(f.path()), &env).unwrap_err();
    assert!(matches!(err.first(), Some(ConfigError::TomlParse { .. })));
}

#[test]
fn validation_failure_lists_every_violation() {
    let f = write_file(
        r#"
[backend]
url = "ftp://wrong-scheme"
timeout_ms = 0

[obs]
sample_rate = 2.0
bus_capacity = 0

[log]
level = "garbage"
"#,
    );
    let env = Env::new();
    let errors = load(Some(f.path()), &env).unwrap_err();
    assert!(
        errors.len() >= 5,
        "expected several violations, got {errors:?}"
    );
    let paths: Vec<String> = errors
        .iter()
        .filter_map(|e| match e {
            ConfigError::Validation { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert!(paths.contains(&"backend.url".to_string()));
    assert!(paths.contains(&"backend.timeout_ms".to_string()));
    assert!(paths.contains(&"obs.sample_rate".to_string()));
    assert!(paths.contains(&"obs.bus_capacity".to_string()));
    assert!(paths.contains(&"log.level".to_string()));
}

#[test]
fn unrecognised_env_var_is_warning_not_error() {
    let f = write_file(valid_baseline_toml());
    let env = Env::new().with("RIFTGATE_BACKEND_TYPOO", "value");
    let cfg: Config = load(Some(f.path()), &env)
        .expect("an unrecognised RIFTGATE_* env var should not block loading");
    // Sanity: the recognised key still loaded.
    assert_eq!(cfg.backend.url, "https://api.openai.com");
}
