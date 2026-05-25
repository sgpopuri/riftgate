# 015. Configuration model

> **Status:** `recommended` — static TOML at startup, with environment-variable overrides on a per-key path basis; fail-loudly validation; restart-only changes for trait-shaping config in `v0.1`. Hot reload of the *safe subset* (backend additions/removals, route table) lands in `v0.2` or `v0.3`. CRD-driven configuration via the Kubernetes operator lands in `v1.0`. See [ADR `0012`](../06-adrs/0012-static-toml-env-override-v01.md).
> **Foundational topics:** layered configuration patterns (defaults → file → env → CLI), structured-config schemas (`serde` + `toml`), file-watch hot reload (`inotify(7)`, `kqueue` `EVFILT_VNODE`), Kubernetes Custom Resource Definitions (CRDs), the twelve-factor configuration discipline
> **Related options:** [`013`](013-observability-sink.md) (observability sink — its endpoint URL is a config key), [`021`](021-rate-limiting.md) (rate limiter — its tuning parameters are config keys), [`010`](README.md) (routing strategy — its parameters are config keys), [`018`](README.md) (deployment model — informs CRD shape)
> **Related ADR:** [ADR `0012`](../06-adrs/0012-static-toml-env-override-v01.md)

## 1. The decision in one sentence

> What is the shape of Riftgate's configuration surface in `v0.1` — file format, override mechanism, validation discipline, and reload semantics — and how does that shape stay compatible with hot reload in `v0.2`/`v0.3` and CRD-driven config via the operator in `v1.0`?

## 2. Context — what forces this decision

Configuration is the most under-designed surface in most network gateways and the source of an outsized share of production outages. The forcing functions for Riftgate:

- [`FR-005`](../01-requirements/functional.md) — configurable upstream URL, auth header, and timeout via TOML config; config changes take effect on restart; invalid configs fail loudly at startup.
- [`NFR-O02`](../01-requirements/non-functional.md) — configuration via TOML or env vars; both supported; env wins in conflicts.
- [`NFR-O03`](../01-requirements/non-functional.md) — hot config reload of the safe subset (backend additions/removals); trait-changing config (e.g. swap IO model) requires restart by design.
- [`NFR-O01`](../01-requirements/non-functional.md) — single static binary; no external config services in the data path by default.
- [`NFR-SEC02`](../01-requirements/non-functional.md) — backend credentials never logged.
- [Vision §8](../00-vision.md) — distributed control plane (xDS-style) is explicitly out of scope; CRDs via the operator are the v1.0 mesh-native shape.

Three forces drive the design:

- **Config is part of the security surface.** Backend credentials, mTLS keys, OAuth client secrets, allowlists. These cannot be logged ([NFR-SEC02](../01-requirements/non-functional.md)), cannot leak through error messages, cannot be diff'd against an old version that contained them. The shape of the loader determines whether secret handling is a discipline or a constant footgun.
- **Validation is part of the contract.** A config that parses but is semantically invalid (a `timeout_ms = 0`, a `backend.url` that is not a URL, a `route` that points to an undefined backend) is the most common operational failure mode in any structured-config system. Failing loudly at startup is the only sane policy; the alternative is a process that runs and silently misbehaves.
- **Hot reload is a deferred capability that must not break the static surface.** [NFR-O03](../01-requirements/non-functional.md) commits us to a *safe-subset* hot reload eventually. The `v0.1` shape must accommodate it: the loader must be re-runnable, the schema must distinguish trait-shaping fields from operationally-tunable fields, and the in-memory config must be reference-counted so callers see a consistent snapshot during a swap.

A fourth implicit force: **operator legibility.** A config that requires reading the source to understand is a config that produces 3am surprises. The schema must be self-describing, the defaults must be the right defaults, and the failure messages must point at the offending key by path.

## 3. Candidates

We evaluate five candidates spanning "literally one TOML file" to "K8s CRD with a controller."

### 3.1. Static TOML only (no env override, no reload)

**What it is.** One TOML file, parsed once at startup via `serde` + `toml`. The path is the first CLI argument or `RIFTGATE_CONFIG`. Configuration changes require a process restart. Environment variables are not consulted.

**Why it's interesting.**
- Simplest possible surface. One source of truth, one parse, one validation pass.
- Self-documenting: a TOML file is human-readable and operators can keep it in version control.
- Zero runtime mystery: the in-memory config matches the file at startup, and never changes.
- TOML's strict syntax catches typos at parse time (no implicit truthiness, no shell-substitution surprises).

**Where it falls short.**
- **No environment override violates [NFR-O02](../01-requirements/non-functional.md).** Many production deployments inject secrets via env vars (Kubernetes secrets, Docker secrets, AWS Secrets Manager + IAM); requiring those secrets to live in a file on disk is a posture regression.
- **No hot reload violates [NFR-O03](../01-requirements/non-functional.md) (eventually).** [NFR-O03](../01-requirements/non-functional.md) is a `v0.2`+ commitment, not a `v0.1` blocker, but the `v0.1` shape must not preclude it. A loader hard-coded to "parse once, never again" requires a refactor when reload lands.
- **No CLI arguments.** Operators expect at minimum `--config <path>` and `--version`; a config-file-only design has to bolt those on later anyway.

**Real-world systems that use it.** Many small CLI tools and embedded utilities. Not the common shape for a network gateway with operational expectations.

### 3.2. Static TOML + environment-variable overrides (recommended)

**What it is.** Same TOML primary as 3.1, but every key is also addressable via an environment variable using a deterministic naming convention: `RIFTGATE_<SECTION>_<KEY>` (e.g. `RIFTGATE_BACKEND_AUTH_HEADER` overrides `[backend] auth_header`). Env wins on conflict per [NFR-O02](../01-requirements/non-functional.md). A small CLI surface (`--config <path>`, `--version`, `--help`) handles the bootstrap. The loader is structured as a layered merge: `defaults → file → env`, each layer producing a typed `Partial<Config>` that is folded into the final `Config`. Validation runs after the merge.

The same loader is *re-runnable* (it is a pure function of `(file_path, env_snapshot)`), so when [NFR-O03](../01-requirements/non-functional.md) hot reload lands in `v0.2`/`v0.3`, the rebuild path is the same code.

**Why it's interesting.**
- Satisfies [`FR-005`](../01-requirements/functional.md) and [NFR-O02](../01-requirements/non-functional.md) in a single mechanism. The TOML is the operator-readable canonical source; env is the runtime injection point for secrets and per-environment overrides.
- The `RIFTGATE_<SECTION>_<KEY>` convention is the [twelve-factor configuration](https://12factor.net/config) discipline applied directly: env is the lingua franca of containerized deployments.
- Layered merge as a structured pattern (defaults → file → env, optional CLI later) matches how every mature config library in the Rust ecosystem (`figment`, `config-rs`, `clap` + custom layers) wants the shape to be.
- Re-runnable loader is the seam for [NFR-O03](../01-requirements/non-functional.md) hot reload without rewriting the loader.
- Validation pass after the merge means the *effective* config is what we validate, not whatever any single layer happened to set.

**Where it falls short.**
- **Env-var paths are stringly-typed.** `RIFTGATE_BACKEND_AUTH_HEADER` is a string, and a typo (`RIFTGATE_BACKND_AUTH_HEADER`) silently does nothing. Mitigation: log every env var consumed at startup, log every `RIFTGATE_*` env var ignored as a warning ("not a recognized config key — typo?").
- **No hot reload yet.** Per [NFR-O03](../01-requirements/non-functional.md), reload is a `v0.2`+ feature; the `v0.1` shape gets us to "ready for it" rather than "shipping it."
- **TOML's expressiveness is bounded.** Complex nested arrays-of-tables can be read but are unwieldy; we use them sparingly and document the patterns.
- **Secret material in env vars is visible in `/proc/<pid>/environ` and to `ps` on misconfigured systems.** This is a deployment concern; mitigation is documented (use container env-injection, not shell exports) but the tradeoff exists for any env-based secret model.

**Real-world systems that use it.** Envoy (`envoy-bootstrap.yaml` + env overrides for some fields), Vault (`config.hcl` + env overrides), most twelve-factor apps. The most common shape for a production-ready single-binary service.

### 3.3. Static TOML + env override + safe-subset hot reload (file-watch driven)

**What it is.** Same as 3.2, plus a file watcher (`notify` crate, backed by `inotify(7)` on Linux and `kqueue` `EVFILT_VNODE` on macOS). On a debounced file change, the loader re-runs, the validator re-runs, and a *diff* is computed against the current in-memory config. If the diff touches only the safe subset (backend pool, route table, observability endpoint URLs, rate-limit values), the new config is swapped atomically into an `ArcSwap<Config>` that all readers consult. If the diff touches the unsafe subset (IO model, async runtime, scheduler choice, allocator strategy, port bindings), the reload is *rejected* with a log line; the operator must restart for those.

The safe-subset / unsafe-subset split is encoded as a derive macro on the schema (`#[reload = "safe"]` vs `#[reload = "restart"]`), so the validator can compute the diff and reject accurately.

**Why it's interesting.**
- Satisfies [NFR-O03](../01-requirements/non-functional.md) directly.
- The `ArcSwap<Config>` (or equivalent RCU pattern) gives readers a consistent snapshot during the swap with no synchronization on the read path.
- The safe/unsafe split is *enforced by the schema*, not by review. Operators cannot accidentally hot-reload a port binding because the schema rejects it.

**Where it falls short.**
- **Real complexity.** File watcher debouncing, atomic swap discipline, partial-config rejection paths, "what does the diff *mean*" semantics for nested arrays — each is a small bug source on its own.
- **`v0.2`+ work, not `v0.1`.** `v0.1` ships static-only; this is the target for `v0.2` or `v0.3` per the roadmap.
- **File-watch on macOS via `kqueue` `EVFILT_VNODE` is finicky** (file replacement vs in-place edit semantics; some editors trigger spurious events).

**Real-world systems that use it.** Envoy ([dynamic-resource-discovery configurations]), HAProxy (runtime API + reload), Caddy (config push API), Cilium (CRD-driven reload).

### 3.4. K8s CRD-driven configuration

**What it is.** No file. No env vars. The Riftgate process watches a set of Kubernetes Custom Resource Definitions (`Riftgate`, `RiftgateBackend`, `RiftgateRoute`) via the controller-runtime client; every CRD change triggers a re-validation and reload.

**Why it's interesting.**
- The native shape for mesh-native deployments. Operators express intent through `kubectl apply -f`; the operator reconciles desired vs actual state and pushes config to the data-plane pods.
- Bounded blast radius via K8s namespaces, RBAC, admission controllers.
- Audit trail comes for free via K8s audit logs.

**Where it falls short.**
- **Requires a Kubernetes operator.** Per [`docs/02-mvp-roadmap.md`](../02-mvp-roadmap.md) the operator is `v1.0`. Demanding it as the only config path means Riftgate has no config story for `v0.1`–`v0.5`.
- **Couples Riftgate to a specific control plane.** Standalone deployments (a single binary on a VM) need a fallback; the trait is the same as "static TOML."
- **Operational complexity.** Operators who run Riftgate without K8s should not have to learn CRD authoring to configure a backend URL.

**Real-world systems that use it.** Envoy via the Envoy AI Gateway operator; Istio (the mesh, not the data plane); Linkerd; Cilium. All ship a non-K8s configuration path as well.

### 3.5. xDS-style remote config (gRPC streaming from a control plane)

**What it is.** No file, no env, no CRD watch. The data plane connects to a remote control plane via the xDS gRPC streaming protocol (the same protocol Envoy uses with Istio's Pilot or the AWS App Mesh control plane). Config arrives as a stream of typed protobuf messages; updates are atomic; the data plane never restarts.

**Why it's interesting.**
- Battle-tested at hyperscaler-mesh scale. Envoy + xDS is the production reference.
- The most "mesh-native" shape — the control plane is a separate process / cluster.
- Atomic update semantics are well-defined.

**Where it falls short.**
- **Riftgate has explicitly chosen not to ship a distributed control plane.** [Vision §8](../00-vision.md) and [`docs/03-architecture/hld.md` §8](../03-architecture/hld.md) both name "distributed control plane (xDS-style)" as out of scope; we leave that to Envoy AI Gateway.
- **A remote control plane is a hard runtime dependency.** Standalone deployments would need a stub control plane. The complexity-to-value ratio for our personas is wrong.
- **gRPC is a non-trivial dependency to drag onto the config path.** TLS, retries, backoff, schema versioning — every operational concern from a network protocol now applies to config bootstrap.

**Real-world systems that use it.** Envoy (xDS), Linkerd (Destination service), AWS App Mesh, GCP Traffic Director. All in the mesh control-plane category.

## 4. Tradeoff matrix

| Property | Static TOML (3.1) | TOML + env (3.2) | TOML + env + hot reload (3.3) | CRD-driven (3.4) | xDS-style (3.5) | Why it matters |
|---|---|---|---|---|---|---|
| Operational complexity | very low | low | medium | high | very high | Must work for a single-binary VM deployment. |
| Twelve-factor env-injection | no | yes | yes | n/a (CRD is the env) | n/a | Containerized deployments inject secrets via env. |
| Hot reload of safe subset | no | no | yes | yes | yes | [NFR-O03](../01-requirements/non-functional.md). |
| Restart for trait-shaping config | required | required | enforced by schema | enforced | enforced | The unsafe subset must not be hot-reloadable. |
| Validation runs against effective config | yes | yes | yes | yes | yes | A merged config can fail in ways no individual layer does. |
| Runtime dependency on a control plane | no | no | no | yes (K8s) | yes (xDS server) | We commit to standalone-first. |
| Compatibility with `v1.0` operator + CRDs | yes (operator can write the file) | yes | yes (operator writes the file; data plane hot-reloads) | yes (this is that case) | no (different shape) | The roadmap commits us to operator + CRDs in `v1.0`. |
| Engineering cost in `v0.1` | very low | low | medium-high | very high | very high | Walking-skeleton scope. |
| Re-runnable loader (seam for hot reload) | yes (trivially) | yes | required | n/a | n/a | The shape that lets `v0.2` add reload without rewrites. |
| Secret-handling discipline | manual (file perms) | env-injection (mature) | both | K8s Secrets | mTLS to control plane | Production-grade. |
| Supports CLI args (`--version`, `--config`) | bolted on | first-class | first-class | bolted on | bolted on | Operator expectation. |

## 5. Foundational principles

**Twelve-factor configuration.** The twelve-factor manifesto's third factor — *store config in the environment* — is not dogma but is the most-tested operational discipline for containerized deployments. The principle: anything that varies between deployment environments (URLs, secrets, feature flags) lives in the environment; the binary is identical across environments. We honor this with the `RIFTGATE_<SECTION>_<KEY>` env-override convention while keeping the operator-readable TOML as the canonical source.

**Layered configuration with a final validation pass.** The standard pattern in mature config libraries (Spring Boot's `application.yml` + profile + env, Envoy's bootstrap + xDS, Kubernetes' kubeconfig + flags) is *defaults → file → env → CLI*, with each layer producing a partial config that merges into the next, and validation running on the *effective* config after all merges. This catches the class of bug where each layer is individually valid but the merge is not. The pattern is documented in detail in the `figment` Rust crate's design notes and in the Kubernetes Configuration Schema documentation; we follow it.

**Validation as a contract — fail loudly.** The directive is direct: *fail at startup with a structured error message*, do not run with degraded behavior. The Postgres `postgresql.conf` validation (which refuses to start on most invalid configs) and Envoy's bootstrap validation (which exits with a structured `ConfigError` listing every path-level violation) are the right shapes. A structured `ConfigError { path, expected, got, source_layer }` lets operators jump directly to the offending key.

**RCU / `ArcSwap` for hot reload of immutable snapshots.** The standard lock-free pattern for "many readers consult a config; occasionally a writer swaps in a new config" is read-copy-update (RCU) — the readers always see a consistent snapshot, and the writer atomically swaps a pointer. The Rust ecosystem's `arc-swap` crate is the canonical implementation; the same pattern appears in Linux kernel data structures, in Java's `AtomicReference`, and in Erlang's process dictionaries. We adopt it as the in-memory shape so the `v0.2`/`v0.3` hot-reload path is small and well-understood.

**Schema-encoded reload safety.** The split between *reloadable* and *restart-required* config is a property of each individual key, not of the whole file. Encoding it as schema metadata (`#[reload = "safe"]` vs `#[reload = "restart"]`) means the validator can compute the diff and reject unsafe partial reloads without an operator having to remember which keys are which. This is the same shape as Envoy's `dynamic_resources` vs `static_resources` distinction.

## 6. Recommendation

**`v0.1` ships static TOML at startup with environment-variable overrides on every key, fail-loudly validation against a versioned schema, and a re-runnable loader that the `v0.2`/`v0.3` safe-subset hot reload will consume unchanged. Trait-shaping config requires restart by design. CRDs via the operator land in `v1.0`.**

Concretely:

1. **Schema.** `crates/riftgate-config::schema` defines the typed config:
   ```rust
   #[derive(Deserialize, Validate)]
   pub struct Config {
       pub server: ServerConfig,    // listen_addr, worker_threads
       pub backend: BackendConfig,  // url, auth_header, tls, timeout_ms
       pub timer: TimerConfig,      // tick_resolution_ms
       pub obs: ObsConfig,          // otel_endpoint, sample_rate
       pub log: LogConfig,          // level, format
   }
   ```
   Each field carries derive metadata for validation (`Validate`) and a future `#[reload = "safe" | "restart"]` annotation that the `v0.2`/`v0.3` hot-reload code will consume. `v0.1` parses the annotation but does not act on it.
2. **Loader.** `crates/riftgate-config::loader::load(path: &Path, env: &Env) -> Result<Config>` is a pure function: defaults are baked in, file is parsed via `serde` + `toml`, env is overlaid via the `RIFTGATE_<SECTION>_<KEY>` convention with explicit recognition of nested keys (`RIFTGATE_BACKEND_TIMEOUT_MS` → `backend.timeout_ms`). Any `RIFTGATE_*` env var that does not map to a known key is logged at `warn` level as a probable typo.
3. **Validation.** `crates/riftgate-config::validate::validate(&Config) -> Result<(), Vec<ConfigError>>` runs after the merge. `ConfigError` carries the path (`backend.timeout_ms`), the expected shape (`positive integer`), the observed value (`0`), and the source layer (`env: RIFTGATE_BACKEND_TIMEOUT_MS`). The binary exits with status 78 (`EX_CONFIG`) on any validation failure, after printing every violation.
4. **Bootstrap.** The `riftgate` binary's `bootstrap.rs` calls `load(...)` then `validate(...)` then constructs the runtime. There is no fall-back, no "best-effort" mode, no "warn and continue."
5. **CLI.** `--config <path>`, `--version`, `--help`. Nothing else. CLI flags do not override config keys in `v0.1` (env is the override layer).
6. **Secret-handling.** Auth headers and any field marked `Secret<String>` are redacted in all log lines, in the `Debug` impl on `Config`, and in the validation error messages. The `Secret<T>` type wraps an inner value with a non-derived `Debug` that prints `"***"`.
7. **`v0.2`/`v0.3` adds hot reload.** A file watcher (via `notify`) triggers `load(...)` + `validate(...)` + `diff(current, new)`. If the diff touches only `#[reload = "safe"]` fields, the new `Config` is swapped into an `ArcSwap<Config>` that all data-plane callers consult. If the diff touches a `#[reload = "restart"]` field, the reload is rejected with a structured log message naming the offending paths.
8. **`v1.0` adds the operator.** The K8s operator writes a TOML file inside the pod and SIGHUPs the data plane (or, equivalently, triggers the file watcher). The operator's input is the CRD; the data plane's input is still TOML. This keeps the standalone deployment shape unchanged.

### Conditions under which we'd revisit

- We discover a class of configuration that does not fit cleanly into TOML (e.g. very nested route-matching DSLs, regex-heavy filter chains). We add YAML as an alternative file format behind a `--config-format` flag without changing the loader's structure.
- A persona emerges that needs a remote control plane (e.g. multi-tenant cloud-gateway-as-a-service). We add an xDS-style gRPC config source as a *peer* of the file source, not a replacement.
- Operator practice with the CRDs (post-`v1.0`) shows that the CRD-as-source-of-truth model is preferable to the "operator writes a TOML file" indirection. We add a direct CRD-watch mode in `v1.x`.

### What stays available behind feature flags

- A `dryrun` mode (`--dry-run`) that loads, validates, prints the effective config (with secrets redacted), and exits. Ships in `v0.1`.
- A `dump-default` mode that prints the schema's default TOML to stdout. Ships in `v0.1`.
- The hot reload (`v0.2`/`v0.3`) ships behind a `--no-hot-reload` opt-out for operators who explicitly want restart-only.
- The CRD watcher (`v1.0`) ships behind the operator's deployment manifest.

## 7. What we explicitly reject

- **Static TOML only with no env override.** Violates [NFR-O02](../01-requirements/non-functional.md) and is a posture regression for containerized deployments.
- **Env-only with no file.** Multi-line, multi-section configs become unreadable as env vars. The TOML file is the canonical operator-readable source.
- **YAML as the primary format.** YAML's implicit-typing surprises (`country: NO` becoming a boolean) and indentation-sensitivity are well-documented hazards. TOML's explicitness is the better default; we may add YAML as an alternative if there is demand.
- **JSON as the primary format.** No comments, awkward for humans to author. Keep JSON for outputs (logs, OTel events), not inputs.
- **Hot reload of trait-shaping config.** Swapping the IO model or the allocator at runtime is not safely implementable; the schema enforces "restart required" for these.
- **xDS-style remote config in `v0.x`.** Out of scope per [Vision §8](../00-vision.md). Reconsider only if a multi-tenant cloud deployment persona emerges with a real funded need.
- **Configuration via SIGHUP-only reload.** SIGHUP can be a *trigger* for the file watcher (it is a reasonable additional reload signal), but it cannot be the *only* trigger; the file watcher is the primary path because operators expect "edit and save" to take effect.
- **A CRD-only `v0.1` config path.** Forces a K8s dependency on standalone users. We ship the file path first; CRDs are an additional source in `v1.0`.

## 8. References

1. Adam Wiggins, *The Twelve-Factor App* — <https://12factor.net/>
2. TOML specification — <https://toml.io/en/>
3. The `serde` Rust crate — <https://serde.rs/>
4. The `toml` Rust crate — <https://docs.rs/toml>
5. The `figment` Rust crate (layered configuration with provenance tracking) — <https://docs.rs/figment>
6. The `notify` Rust crate (cross-platform file watching) — <https://docs.rs/notify>
7. The `arc-swap` Rust crate (RCU-style atomic pointer swap) — <https://docs.rs/arc-swap>
8. Linux `inotify(7)` man page — <https://man7.org/linux/man-pages/man7/inotify.7.html>
9. FreeBSD `kevent(2)` and `EVFILT_VNODE` — <https://man.freebsd.org/cgi/man.cgi?query=kevent>
10. Kubernetes Custom Resource Definitions documentation — <https://kubernetes.io/docs/concepts/extend-kubernetes/api-extension/custom-resources/>
11. Envoy bootstrap configuration reference — <https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/bootstrap/v3/bootstrap.proto>
12. Envoy xDS REST and gRPC protocol — <https://www.envoyproxy.io/docs/envoy/latest/api-docs/xds_protocol>
13. PostgreSQL `postgresql.conf` validation behavior — <https://www.postgresql.org/docs/current/runtime-config.html>
14. Paul E. McKenney, *What is RCU, Fundamentally?* (LWN.net) — <https://lwn.net/Articles/262464/>
15. The Rust `clap` crate (CLI parsing) — <https://docs.rs/clap>
