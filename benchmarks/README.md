# benchmarks/

Reproducible benchmark harness. **Empty during the `v0.0` public design phase.** Benchmarks land alongside the corresponding milestone in [`../docs/02-mvp-roadmap.md`](../docs/02-mvp-roadmap.md).

## Discipline

The Riftgate brand explicitly opposes vendor-style benchmark claims. Every published number must be:

- **Reproducible from this directory** with one command (e.g. `cargo bench --bench accept_throughput`).
- **Documented** with hardware, kernel version, OS, Riftgate version, configuration, and competitor versions.
- **Comparable** to a real baseline (LiteLLM, an existing Rust gateway, or a published vendor claim with citation).
- **Honest about scope.** A benchmark on a 100-token prompt is not a claim about a 100k-token prompt; the docs say so.
- **Honest about losses.** When Riftgate loses a benchmark — and it will, against TensorZero on raw P99 — we say so plainly and link to the `v0.0` Vision doc that explains why we are not chasing that axis.

## Planned benchmarks

| Benchmark | Lands at | Purpose |
|-----------|----------|---------|
| `accept_throughput` | v0.1 | Bare accept-loop throughput; baselines our IO subsystem. |
| `parser_throughput` | v0.1 | HTTP/1.1 + SSE parser throughput on captured traffic. |
| `arena_alloc_cost` | v0.1 | Per-request arena vs system allocator microbench. |
| `mpmc_queue_scaling` | v0.2 | Lock-free MPMC queue scaling vs core count. |
| `epoll_vs_io_uring` | v0.2 | Honest A/B between IO backends on the same workload. |
| `circuit_breaker_overhead` | v0.2 | Circuit breaker per-request cost. |
| `wasm_filter_overhead` | v0.3 | Per-filter overhead for WASM filters. |
| `kv_aware_routing_hit_rate` | v0.3 | Cache-hit rate for KV-aware routing on a realistic prefix-distribution mix. |
| `ebpf_profiling_overhead` | v0.4 | Cost of always-on BPF profiling. |
| `vs_litellm` | v0.2 | Riftgate v0.2 against LiteLLM. (Expected win.) |
| `vs_tensorzero_published_claim` | v0.2 | Riftgate v0.2 against TensorZero's published <1ms P99 at 10k QPS claim. (Expected loss; documented.) |

## Hardware reference profile

We standardize on **AWS `c7i.xlarge` (4 vCPU, 8 GB RAM, Linux 6.1+)** for the headline benchmarks because that is TensorZero's reference profile and the comparisons should be apples-to-apples.

Per-benchmark `README.md` will document the exact hardware, kernel, and config used.

## What goes in this directory

- `harness/` — the shared benchmark harness (request generator, latency histogram, etc.).
- One subdirectory per benchmark, with a `README.md`, the harness invocation, and any required fixtures.
- `results/` — captured results from named runs, with the metadata above. Source-of-truth results are tagged in git so external citations can pin to a stable artifact.
