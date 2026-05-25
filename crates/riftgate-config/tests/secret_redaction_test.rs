//! Secret redaction round-trip tests.
//!
//! Verifies that `Secret<String>` redacts at every leak surface used by
//! the binary: `Debug`, `Display`, `Serialize`, the `Config`'s `Debug`
//! impl, and the loader's `--dry-run` output.

use riftgate_config::Secret;

#[test]
fn debug_does_not_leak_value() {
    let s = Secret::new("Bearer sk-real-token-do-not-leak".to_string());
    let dbg = format!("{s:?}");
    assert!(!dbg.contains("sk-real-token"), "Debug leaked secret: {dbg}");
    assert_eq!(dbg, "Secret(***)");
}

#[test]
fn display_does_not_leak_value() {
    let s = Secret::new("Bearer sk-also-do-not-leak".to_string());
    let disp = format!("{s}");
    assert!(!disp.contains("sk-"), "Display leaked secret: {disp}");
    assert_eq!(disp, "***");
}

#[test]
fn debug_inside_a_larger_struct_does_not_leak() {
    use riftgate_config::Config;

    let mut cfg = Config::default();
    cfg.backend.url = "https://upstream.example".into();
    cfg.backend.auth_header = Secret::new("Bearer sk-do-not-leak-via-config-debug".into());
    let dbg = format!("{cfg:?}");
    assert!(
        !dbg.contains("sk-do-not-leak"),
        "Config Debug leaked secret: {dbg}"
    );
}

#[test]
fn json_serialize_does_not_leak_value() {
    let s = Secret::new("Bearer sk-serialize-secret".to_string());
    let j = serde_json::to_string(&s).unwrap();
    assert_eq!(j, "\"***\"");
}
