//! Verify that every canonical span name is unique and matches the
//! FR-006 fragment list. (Same as the in-module test, exposed as a
//! public conformance test for completeness.)

use riftgate_obs::spans;

#[test]
fn span_names_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for &name in spans::ALL {
        assert!(seen.insert(name), "duplicate span name: {name}");
    }
}

#[test]
fn span_names_match_fr006_fragments() {
    for fragment in [
        "received",
        "queued",
        "dispatched",
        "first_token",
        "completed",
    ] {
        assert!(
            spans::ALL.iter().any(|n| n.contains(fragment)),
            "no canonical span contains the FR-006 fragment `{fragment}`"
        );
    }
}
