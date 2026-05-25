# riftgate-router

Routing impls behind the `riftgate-core::router::Router` trait.

| Impl | Status | Lands at |
|------|--------|----------|
| `RoundRobinRouter` | shipped | `v0.1` |
| `ConstantRouter` | shipped (test impl) | `v0.1` |
| `WeightedRandomRouter` | planned | `v0.2` |
| `KvAwareRouter` | planned | `v0.3` |
| `HedgedRouter` | planned | `v0.3` |

`RoundRobinRouter` uses a single `AtomicUsize` cursor over the `BackendPool`. The cursor is incremented with `Ordering::Relaxed`; perfect monotonicity is not required, only fair distribution over time. A statistical fairness test in `tests/fairness.rs` asserts that 3000 requests across 3 backends produce a per-backend count within ±10 of 1000.

`ConstantRouter` always returns the same `BackendId`. It is the FR-X02 second impl and is also useful in unit tests for any caller that needs a `Router` without testing routing logic itself.

See [`docs/04-design/lld-routing.md`](../../docs/04-design/lld-routing.md) for the design rationale.
