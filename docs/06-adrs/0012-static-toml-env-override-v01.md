# ADR 0012. Static TOML configuration with environment-variable overrides; safe-subset hot reload deferred to v0.2 / v0.3; CRDs in v1.0

> **Date:** 2026-05-10
> **Status:** accepted
> **Options doc:** [015-config-model](../05-options/015-config-model.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate needs a configuration model for `v0.1` that satisfies [`FR-005`](../01-requirements/functional.md) (TOML config with restart-on-change semantics, fail-loudly on invalid configs), [`NFR-O02`](../01-requirements/non-functional.md) (TOML or env vars; env wins on conflict), [`NFR-SEC02`](../01-requirements/non-functional.md) (credentials never logged), and [`NFR-O01`](../01-requirements/non-functional.md) (single static binary). The `v0.1` shape must also accommodate the future commitments — [`NFR-O03`](../01-requirements/non-functional.md) safe-subset hot reload (`v0.2`/`v0.3`) and CRD-driven configuration via the operator (`v1.0`) — without a breaking refactor. Full exploration of candidates (static TOML only, TOML + env, TOML + env + hot reload, K8s CRD-driven, xDS-style remote) lives in [Options `015`](../05-options/015-config-model.md).

The forces summarized: TOML is the operator-readable canonical source; env is the runtime injection point for secrets and per-environment overrides; validation runs against the *effective* config after layered merge; the loader is re-runnable so hot reload is a small `v0.2`/`v0.3` add; secrets must be redacted at every leak point.

## Decision

**Riftgate `v0.1` ships a static TOML configuration loaded once at startup, with environment-variable overrides on a per-key basis using the `RIFTGATE_<SECTION>_<KEY>` convention, fail-loudly validation against a versioned typed schema (`serde` + a `Validate` derive), a re-runnable loader that future hot reload will consume unchanged, and explicit redaction of `Secret<T>`-marked fields at every logging surface.**

The discipline:

- **Schema.** `crates/riftgate-config::schema` defines the typed `Config` with sections `[server]`, `[backend]`, `[timer]`, `[obs]`, `[log]`. Each field carries `serde` deserialization, `Validate` derive metadata, and a `#[reload = "safe" | "restart"]` annotation. `v0.1` parses the annotation but does not act on it.
- **Loader.** `crates/riftgate-config::loader::load(path: &Path, env: &Env) -> Result<Config>` is a pure function: defaults are baked in; the file is parsed via `serde` + `toml`; env is overlaid via `RIFTGATE_<SECTION>_<KEY>` (e.g. `RIFTGATE_BACKEND_TIMEOUT_MS` → `backend.timeout_ms`). Any `RIFTGATE_*` env var that does not map to a known key is logged at `warn` level as a probable typo.
- **Validation.** `crates/riftgate-config::validate::validate(&Config) -> Result<(), Vec<ConfigError>>` runs after the merge. `ConfigError { path, expected, got, source_layer }` carries enough information for the operator to jump directly to the offending key. The binary exits with status 78 (`EX_CONFIG`) on any validation failure, after printing every violation.
- **Bootstrap.** `crates/riftgate::bootstrap` calls `load(...)` then `validate(...)` then constructs the runtime. There is no fall-back, no "best-effort" mode, no "warn and continue" path.
- **CLI.** `--config <path>`, `--version`, `--help`, `--dry-run` (load + validate + print effective config + exit), `--dump-default` (print schema's default TOML to stdout). Nothing else. CLI flags do not override config keys in `v0.1`; env is the override layer.
- **Secret-handling.** Auth headers and any field marked `Secret<String>` are redacted in all log lines, in the `Debug` impl on `Config`, and in `ConfigError` messages. The `Secret<T>` type wraps an inner value with a non-derived `Debug` that prints `"***"`.
- **In-memory snapshot shape.** The validated `Config` is wrapped in `ArcSwap<Config>` from the start, even though `v0.1` never swaps it. This makes the `v0.2`/`v0.3` hot-reload path a small, additive change.
- **`v0.2`/`v0.3` adds hot reload.** A file watcher (via `notify`) triggers `load(...)` + `validate(...)` + `diff(current, new)`. If the diff touches only `#[reload = "safe"]` fields, the new `Config` is swapped into the `ArcSwap<Config>`. If the diff touches a `#[reload = "restart"]` field, the reload is rejected with a structured log message naming the offending paths.
- **`v1.0` adds the operator.** The K8s operator writes a TOML file inside the pod and triggers the file watcher. The data plane's input is still TOML; the operator's input is the CRD. This keeps the standalone deployment shape unchanged.

## Consequences

- **Positive:**
  - Satisfies [`FR-005`](../01-requirements/functional.md) and [`NFR-O02`](../01-requirements/non-functional.md) in a single mechanism. Operators get the readable file as the canonical source and the env-injection ergonomics they expect from containerized deployments.
  - Validation runs against the *effective* (merged) config, catching the class of bug where each layer is individually valid but the merge is not.
  - Loader is re-runnable from day one; hot reload (`v0.2`/`v0.3`) is a small additive change, not a rewrite.
  - `ArcSwap<Config>` from `v0.1` means call sites already use `cfg.load()` to get a snapshot; readers get a consistent snapshot during a future swap with no synchronization cost.
  - `Secret<T>` redaction is enforced at the type level; a stray `format!("{:?}", config)` cannot leak a credential.
  - CLI surface is minimal and conventional (`--config`, `--version`, `--help`, `--dry-run`, `--dump-default`); operators are not surprised.
- **Negative / accepted tradeoffs:**
  - Env-var paths are stringly-typed (`RIFTGATE_BACKEND_AUTH_HEADER`); a typo silently does nothing. Mitigation: log every consumed env var at startup at `info` level; log every unrecognized `RIFTGATE_*` env var at `warn` level.
  - No hot reload in `v0.1`; operators must restart for any change. This is intentional and matches [`FR-005`](../01-requirements/functional.md) acceptance ("changes take effect on restart"). [`NFR-O03`](../01-requirements/non-functional.md) reload lands in `v0.2`/`v0.3`.
  - Env-var secret material is visible in `/proc/<pid>/environ` and to `ps` on misconfigured systems; this is a deployment concern documented in `examples/minimal-proxy/README.md`.
  - TOML's expressiveness is bounded; complex nested arrays-of-tables work but are unwieldy. We use them sparingly.
- **Future work this enables:**
  - File-watch-driven safe-subset hot reload in `v0.2`/`v0.3` via the existing re-runnable loader and `ArcSwap<Config>` shape.
  - Operator-driven CRD configuration in `v1.0` (operator writes a TOML file inside the pod; data plane consumes via the same loader).
  - Per-environment config layering (e.g. a `defaults.toml` shipped in the binary, an operator-supplied `overrides.toml`).
  - A future YAML alternative file format behind `--config-format` if a class of configuration emerges that does not fit cleanly in TOML.
- **Future work this forecloses (until superseded):**
  - We will not ship an env-only configuration (no file).
  - We will not ship a JSON or YAML primary format in `v0.x`.
  - We will not ship hot reload of trait-shaping config (IO model, allocator, port bindings); the schema enforces "restart required" for these.
  - We will not ship xDS-style remote configuration; out of scope per [Vision §8](../00-vision.md).
  - We will not ship a CRD-only `v0.1` config path; that would force a K8s dependency on standalone users.
  - We will not ship a "warn and continue" validation mode; failure is loud and immediate.

## Compliance

- `crates/riftgate-config::schema::Config` is the single typed config struct.
- `crates/riftgate-config::loader::load(path, env) -> Result<Config>` is the single loader entry point.
- `crates/riftgate-config::validate::validate(&Config) -> Result<(), Vec<ConfigError>>` is the single validation entry point.
- `crates/riftgate-config/tests/loader_test.rs` covers: file-only, env-only-overrides, file + env merge, missing-file, invalid-TOML, unrecognized-env-var-warning, validation-failure-exit-code.
- `crates/riftgate-config/tests/secret_redaction_test.rs` covers: `Secret<String>` redaction in `Debug`, in log lines, in `ConfigError` messages.
- The `--dry-run` CLI flag is a release-grade tool: operators run it before deploying a config change.
- Adding a new section or key requires updating `crates/riftgate-config::schema` and the corresponding fixture in `crates/riftgate-config/tests/fixtures/`.
- Promoting a `#[reload = "restart"]` key to `#[reload = "safe"]` requires a new ADR superseding the relevant clause of this one (because hot-reload safety is a load-bearing claim about each individual key).

## Notes

- The choice of TOML over YAML is deliberate. YAML's implicit-typing surprises (`country: NO` parsed as a boolean) and indentation-sensitivity are well-documented hazards; TOML is explicit and unambiguous. We may add YAML as an alternative if there is operator demand.
- The `RIFTGATE_<SECTION>_<KEY>` convention is the [twelve-factor](https://12factor.net/config) discipline applied directly. Nested keys use `_` as the separator: `RIFTGATE_BACKEND_TLS_VERIFY` → `backend.tls.verify`. This is a documented limitation: a config key whose name contains `_` could be ambiguous to the env-mapper, so the schema must avoid such names (we do).
- The `ArcSwap<Config>` shape from `v0.1` (even without hot reload) is the most important forward-compatibility decision in this ADR. It is the seam that lets `v0.2`/`v0.3` add reload without rewriting any caller.
- The operator (`v1.0`) is intentionally a *consumer* of this config model, not a *replacement* for it. The CRD shape and the TOML shape will be kept in correspondence, with the operator translating CRDs into the TOML the data plane already understands. This is the same pattern Cilium uses (CRDs → cilium-config ConfigMap → agent reads file).
- The `Secret<T>` type is the most important security primitive in this ADR. Every leak surface (log macros, `Debug`, error messages, the `--dry-run` output) consults it. A reviewer who sees a credential in any output should treat it as a `Secret<T>` regression and open a security issue.
