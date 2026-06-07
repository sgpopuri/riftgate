# examples/

Sample configurations and deployment manifests. The first example ([`01-basic-openai-proxy`](01-basic-openai-proxy/)) ships with the v0.1 walking skeleton; the others land alongside the corresponding milestone in [`../docs/02-mvp-roadmap.md`](../docs/02-mvp-roadmap.md).

## Examples

| Example | Status |
|---------|--------|
| [`01-basic-openai-proxy/`](01-basic-openai-proxy/) — single-backend OpenAI-compatible proxy with OTel collector docker-compose | shipped (v0.1) |
| [`02-starter-filters/`](02-starter-filters/) — starter filter chain catalog and config shape | shipped (v0.3 planning surface) |
| `03-multi-backend-routing/` — round-robin and weighted-random across backends | v0.2 |
| `04-with-circuit-breaker/` — adaptive backpressure, 503 with `Retry-After` | v0.2 |
| `05-kv-aware-routing/` — KV-cache-aware routing against a vLLM cluster | v0.3 |
| `06-hedged-requests/` — hedged routing with cancellation | v0.3 |
| `07-with-ebpf-observability/` — Aya BPF sink enabled | v0.4 |
| `08-k8s-sidecar/` — Helm chart for sidecar deployment | v1.0 |
| `09-istio-mesh/` — Istio integration | v1.0 |

Each example will include:

- `riftgate.toml` — the config.
- `README.md` — what this example shows, how to run it, what to look for in logs / traces / metrics.
- `docker-compose.yml` or equivalent for reproducibility, where applicable.
- Curl scripts or a small Python script that exercises the example.
