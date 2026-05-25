//! Canonical span-name registry.
//!
//! Per [`FR-006`](../../../docs/01-requirements/functional.md): every
//! request emits a span sequence with the exact names listed here. Span
//! names are part of Riftgate's public API; renaming requires a
//! deprecation cycle (operators have dashboards keyed on these names).
//!
//! Emission sites use these constants exclusively (no string literals).
//! CI lints the data plane for raw `&'static str` span names that are
//! not in this set.

/// Span emitted when a request is first received from a client.
pub const REQUEST_RECEIVED: &str = "request.received";

/// Span emitted when a request is enqueued onto a worker shard.
pub const REQUEST_QUEUED: &str = "request.queued";

/// Span emitted when a request is dispatched to an upstream backend.
pub const REQUEST_DISPATCHED: &str = "request.dispatched";

/// Span emitted on the first token / response byte from the upstream.
pub const REQUEST_FIRST_TOKEN: &str = "request.first_token";

/// Span emitted when the request completes (success or failure).
pub const REQUEST_COMPLETED: &str = "request.completed";

/// Span emitted when a request is rejected before dispatch (e.g. by the
/// rate limiter in `v0.2`+).
pub const REQUEST_REJECTED: &str = "request.rejected";

/// All canonical span names, in the order they appear during a typical
/// successful request. Useful for tests and registry checks.
pub const ALL: &[&str] = &[
    REQUEST_RECEIVED,
    REQUEST_QUEUED,
    REQUEST_DISPATCHED,
    REQUEST_FIRST_TOKEN,
    REQUEST_COMPLETED,
    REQUEST_REJECTED,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for &name in ALL {
            assert!(seen.insert(name), "duplicate span name: {name}");
        }
    }

    #[test]
    fn span_names_match_fr006_canonical_words() {
        // FR-006 enumerates: received, queued, dispatched, first_token,
        // completed. Verify each canonical fragment appears in ALL.
        for fragment in [
            "received",
            "queued",
            "dispatched",
            "first_token",
            "completed",
        ] {
            assert!(
                ALL.iter().any(|n| n.contains(fragment)),
                "no span contains the FR-006 fragment `{fragment}`"
            );
        }
    }
}
