# ADR 0029. Per-request tenant identity resolved via API key registry; `TenantResolver` trait in `riftgate-core`

> **Date:** 2026-07-04
> **Status:** accepted
> **Options doc:** [017-multitenancy](../05-options/017-multitenancy.md)
> **Deciders:** Sriram Popuri

## Context

`v0.5` shipped with a single-tenant shortcut: the proxy reads `x-riftgate-tenant` directly as a raw `u32` and falls back to `TenantId(0)`. This is correct for trusted-network deployments but breaks the threat model for multi-tenant operators: any caller can forge any tenant identity, defeating the capability broker (`AllowlistBroker`, ADR 0015) and the rate limiter (`TokenBucketLimiter`, ADR 0009). `FR-502` requires authentic per-tenant identity. Full exploration of the four candidate approaches lives in [Options `017`](../05-options/017-multitenancy.md).

## Decision

**`v1.0` resolves per-request tenant identity via an API key registry; a new `TenantResolver` trait in `riftgate-core` abstracts the resolution so future impls (JWT, external delegation) can land without touching callers.**

The specifics:

- `TenantResolver` trait defined in `crates/riftgate-core/src/tenant.rs`. Single method: `resolve(&self, headers: &http::HeaderMap) -> Option<TenantIdentity>`. `Send + Sync`. Trait-object safe.
- `ApiKeyTenantResolver` is the production default impl. It reads `Authorization: Bearer <key>`, computes SHA-256 of the raw key, performs an O(1) `HashMap` lookup against hashed keys stored in config, and returns the mapped `TenantIdentity`. Unknown or absent keys return `None`; the proxy returns `401`.
- `HeaderTenantResolver` retains the v0.5 behavior. Enabled via `[multitenancy] mode = "trusted-header"` in config. Not the default.
- API keys are stored as `SHA-256(<raw-key>)` hex in config under `[multitenancy.api_keys]`, never as plaintext. The gateway computes `SHA-256(incoming_bearer)` at request time and compares. This means the config file does not contain usable keys even if read by an attacker.
- The resolver is constructed at startup from config and stored in `HandlerState` as `Arc<dyn TenantResolver>`. The MCP broker path (`proxy.rs`) and the rate limiter path both call `resolver.resolve(headers)` to obtain the identity before dispatching.
- The config key for tenant names maps to `TenantId` via the same FNV-1a hash already used by `mcp.rs`, so the name-to-id mapping is consistent across the MCP config and the tenant resolver config.

## Consequences

- **Positive:**
  - Real credential check on every request with ~100 ns overhead (one SHA-256 + one HashMap lookup). No network round-trip.
  - The API key model is what developers already expect from LLM API providers; no learning curve.
  - Config file safety: stored SHA-256 hashes mean raw keys are not exposed even if the config is leaked.
  - `HeaderTenantResolver` retains backward compatibility for internal/mesh deployments.
  - `JwtTenantResolver` and `ExternalTenantResolver` can land as new impls behind the same trait without changing any calling code.
- **Negative / accepted tradeoffs:**
  - Key revocation requires a config change and restart (v1.0). Hot-reload from CRDs is the v1.1+ path.
  - No built-in support for OAuth or short-lived tokens in v1.0. Operators who need short-lived credentials should front Riftgate with a JWT-issuing proxy or wait for `JwtTenantResolver`.
  - SHA-256 per request adds ~20–50 ns (software SHA-256 on a modern ARM64 core for a 20-50 byte key). Acceptable; well within the gateway's P95 budget.
- **Future work this enables:**
  - `JwtTenantResolver` (additive, new ADR): verify JWT signature, extract claim, map to `TenantId`.
  - `ExternalTenantResolver` (additive, new ADR): forward-auth sidecar for arbitrary policy.
  - CRD-driven key registry in v1.1+: `ApiKeyTenantResolver` loads from the `Riftgate` CRD's `spec.apiKeys` field instead of TOML.
- **Future work this forecloses (until superseded):**
  - The gateway will not use raw unauthenticated tenant headers as the production default. `HeaderTenantResolver` remains available as an explicit opt-in.

## Compliance

- `crates/riftgate-core::tenant::TenantResolver` is the single trait. `HeaderTenantResolver` lives in `riftgate-core`; `ApiKeyTenantResolver` lives in `riftgate-config` or a new `crates/riftgate-auth`.
- `HandlerState.tenant_resolver: Arc<dyn TenantResolver>` replaces the inline header read in `proxy.rs`.
- The resolver is constructed from `config.multitenancy` in `crates/riftgate/src/main.rs`.
- A unit test asserts that `ApiKeyTenantResolver` rejects unknown keys, accepts known hashed keys, and maps them to the correct `TenantId`.
- A unit test asserts that `HeaderTenantResolver` maps numeric and named (FNV-1a hashed) header values correctly and returns `None` on a missing header.
- Adding a new `TenantResolver` impl that delegates externally requires a new ADR superseding this one.

## Notes

- SHA-256 is chosen over faster non-cryptographic hashes for key storage because: (1) the stored hash is security-sensitive (prevents reconstruction of raw keys), and (2) the computational cost on modern hardware is negligible for this use case.
- The `Authorization: Bearer <key>` header is the standard credential-bearing header per RFC 6750. Custom headers like `x-api-key` are also common; Options `017` evaluated both. RFC 6750 is preferred for interoperability.
- The FNV-1a tenant ID derivation from string keys (introduced in v0.5) is preserved here so MCP config and tenant resolver config share a consistent name-to-id mapping without a separate lookup table.
