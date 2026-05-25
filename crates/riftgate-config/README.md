# riftgate-config

Riftgate's v0.1 configuration surface, per [Options 015](../../docs/05-options/015-config-model.md) and [ADR 0012](../../docs/06-adrs/0012-static-toml-env-override-v01.md).

- **Schema** — typed `Config` struct with `[server]`, `[backend]`, `[timer]`, `[obs]`, `[log]` sections.
- **Loader** — pure function `load(path, env) -> Result<Config>`. Layered merge: defaults → file → env. Re-runnable; the v0.2/v0.3 hot-reload path consumes it unchanged.
- **Validation** — `validate(&Config) -> Result<(), Vec<ConfigError>>` runs against the *effective* (merged) config. The binary exits with status 78 (`EX_CONFIG`) on any violation.
- **Secrets** — the `Secret<T>` newtype redacts every field marked with it in `Debug`, `Display`, and the loader's diagnostic output.
- **Env override convention** — every key is addressable as `RIFTGATE_<SECTION>_<KEY>` (e.g. `RIFTGATE_BACKEND_AUTH_HEADER` overrides `[backend] auth_header`). Env wins on conflict per [NFR-O02](../../docs/01-requirements/non-functional.md). Unrecognised `RIFTGATE_*` env vars are logged at `warn` level as probable typos.

## Tests

- `tests/loader_test.rs` — round-trip on file-only, env-only-overrides, file + env merge, missing-file, invalid-TOML, unrecognised-env-var-warning, validation-failure cases.
- `tests/secret_redaction_test.rs` — verifies `Secret<String>` redaction in `Debug`, `Display`, and `ConfigError` messages.
