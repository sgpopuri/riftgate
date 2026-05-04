# examples/

Sample configurations and deployment manifests. **Empty during the `v0.0` public design phase.** Examples land alongside the corresponding milestone in [`../docs/02-mvp-roadmap.md`](../docs/02-mvp-roadmap.md).

## Planned examples

| Example | Lands at |
|---------|----------|
| `01-basic-openai-proxy/` — single-backend OpenAI-compatible proxy | v0.1 |
| `02-multi-backend-routing/` — round-robin and weighted-random across backends | v0.2 |
| `03-with-circuit-breaker/` — adaptive backpressure, 503 with `Retry-After` | v0.2 |
| `04-wasm-pii-filter/` — minimal WASM filter that redacts emails from prompts | v0.3 |
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
