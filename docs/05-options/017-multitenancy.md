# 017. Multitenancy — per-request tenant identity resolution

> **Status:** `recommended` — API key registry as the production default; header passthrough retained for trusted-network mode. Target milestone: `v1.0`. See [ADR 0029](../06-adrs/0029-api-key-tenant-resolver.md).
> **Foundational topics:** capability-based security, zero-trust networking (BeyondCorp), API key authentication patterns, JWT (RFC 7519), object-capability security
> **Related options:** [`026-mcp-orchestration`](026-mcp-orchestration.md) (the capability broker that consumes the resolved identity), [`021-rate-limiting`](021-rate-limiting.md) (the rate limiter that scopes buckets per tenant)
> **Related ADR:** [ADR 0029](../06-adrs/0029-api-key-tenant-resolver.md)

## 1. The decision in one sentence

> How does Riftgate map an incoming request's wire-level credential or header to a `TenantId` that the rest of the data plane — capability broker, rate limiter, observability — uses to apply per-tenant policy?

## 2. Context — what forces this decision

Riftgate shipped `v0.5` with a single-tenant shortcut: the binary reads a raw `x-riftgate-tenant` header, parses it as a `u32`, and falls back to `TenantId(0)`. This works for development and for internal mesh deployments where callers are trusted to self-identify, but it breaks the threat model for any multi-tenant operator:

- **A compromised or misconfigured client can claim any tenant identity.** There is no credential check behind the header.
- **The rate limiter (`TokenBucketLimiter`, ADR 0009) and capability broker (`AllowlistBroker`, ADR 0015) both scope policy to `TenantId`.** If tenants can forge their identity, both policy surfaces are defeated.
- **Observability dashboards partition by tenant.** Forged identities pollute the audit trail.

The requirement is `FR-502` (per-tenant capability allowlist), which depends on the identity being authentic. And [NFR-SEC03](../01-requirements/non-functional.md) requires that security-sensitive paths pass through cryptographic or structural validation, not raw header trust.

This decision is also the prerequisite for per-request tenant resolution, the v0.5 open question carried forward at ADR 0015's close.

Three constraints frame the options:

1. **Low overhead.** Tenant resolution happens on every request, on the hot path. Cryptographic operations add latency; the resolver must be fast enough that the impact is below the gateway's P95 budget.
2. **No new network dependency on the fast path.** An external auth service call per request adds tail-latency risk and a new failure mode. The base case must be in-process.
3. **Config-static in v1.0; CRD-driven in v1.1+.** The resolver config is loaded from the TOML config at startup (and, at v1.1+, from Kubernetes CRDs). Hot-reload is a future concern; restarts are acceptable for v1.0.

## 3. Candidates

### 3.1. Header passthrough (current v0.5 default)

**What it is.** The gateway reads `x-riftgate-tenant: <id>` directly and uses the value as the `TenantId`. No credential validation. Falls back to `TenantId(0)` when absent.

**Why it's interesting.**
- Zero latency: a single header lookup.
- Zero configuration: no credential registry to maintain.
- Correct for trusted internal deployments where callers are authenticated by a mesh or sidecar before reaching Riftgate.

**Where it falls short.**
- Any unauthenticated caller can claim any tenant. This breaks the threat model for internet-facing deployments.
- Operators must enforce identity outside Riftgate (service mesh mTLS, load-balancer auth). That external enforcement is invisible to Riftgate's audit trail.
- Cannot be the production default for any deployment that cares about per-tenant billing or capability enforcement.

**Real-world analog.** Internal microservice trust headers in a mesh (e.g. Istio's `x-forwarded-client-cert`, or GCP's internal identity headers on fully-authenticated meshes). Appropriate when every caller has already been authenticated at the edge.

### 3.2. API key registry (config-defined)

**What it is.** The gateway maintains a config-defined map of API key → tenant name. The client presents its API key in `Authorization: Bearer <key>` (or a custom header). The gateway performs an O(1) lookup (HashMap) to resolve the key to a `TenantId`. Unknown keys are rejected with `401`.

**Why it's interesting.**
- Provides a real credential check with near-zero overhead: one HashMap lookup per request.
- Config-driven revocation: removing a key from config (and reloading/restarting) invalidates it immediately.
- No new network dependencies on the fast path.
- Matches how most LLM API providers (OpenAI, Anthropic, Mistral) actually authenticate developers: a pre-shared Bearer token.
- Keys can be stored hashed (SHA-256) in config so the config file itself does not leak raw keys.

**Where it falls short.**
- Keys are static until config reload: short-lived credentials (e.g. OAuth tokens) require a different approach.
- The config file is the credential store; it must be protected with appropriate filesystem permissions.
- No support for key rotation without a restart (v1.0 constraint; hot-reload from CRDs is a v1.1+ concern).

**Code sketch.**
```toml
[multitenancy.api_keys]
# Keys are SHA-256 hex of the actual raw key so the config file is safe to store.
# 'acme-prod' maps to tenant id 1.
"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" = "acme"
"sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08" = "bigcorp"

[multitenancy.tenants.acme]
id = 1  # explicit numeric ID; auto-assigned via FNV-1a hash if absent
```

**Real-world systems.** OpenAI API keys, Anthropic API keys, GitHub personal access tokens, AWS access key ID + secret — all are pre-shared bearer tokens in a registry.

### 3.3. JWT claim extraction

**What it is.** The client presents `Authorization: Bearer <jwt>`. The gateway verifies the JWT signature against a configured public key (or a JWKS endpoint, fetched at startup and cached), then extracts a configured claim (`tenant`, `org`, `sub`) from the payload as the tenant identifier.

**Why it's interesting.**
- Short-lived tokens: JWTs typically expire in minutes to hours, reducing the blast radius of a leaked token.
- No credential registry to maintain at the gateway: token validation is stateless (verify signature + check expiry).
- Supports existing SSO / IdP infrastructure (Okta, Auth0, Keycloak) without changes to the IdP.

**Where it falls short.**
- JWT signature verification requires one RSA/ECDSA operation per request, which adds ~0.5–2 µs of cryptographic overhead per request (acceptable, but visible on the hot path).
- Clock skew: JWT expiry validation requires time synchronization. The gateway must have NTP-synchronized time.
- JWKS endpoint: fetching the public key from a JWKS URL at startup adds an external network dependency; caching the key in config is simpler but requires manual rotation.
- Operator complexity: JWTs are not self-explanatory for developers setting up a local dev loop. API keys are simpler to issue and debug.

**Real-world systems.** Kubernetes service account tokens, Auth0/Okta JWT-based API access, GCP service account JWTs.

### 3.4. Upstream auth delegation (sidecar / forward-auth)

**What it is.** The gateway forwards an auth sub-request to an external authorization service (e.g. an OPA instance, an Envoy ext-authz sidecar, or a custom auth microservice) before dispatching the upstream request. The auth service returns identity headers that the gateway reads.

**Why it's interesting.**
- Maximum policy flexibility: the auth service can implement any resolution logic (LDAP lookup, custom business rules, dynamic revocation).
- Decouples policy logic from the gateway binary entirely.
- Matches the Envoy external authorization model ([envoyproxy.io/docs/envoy/latest/intro/arch_overview/security/ext_authz_filter](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/security/ext_authz_filter)).

**Where it falls short.**
- Every request incurs an additional network round-trip. Even a local sidecar adds 0.1–0.5 ms of tail latency per request — an order of magnitude above the API-key lookup path.
- Introduces a new synchronous failure mode: if the auth service is slow or unavailable, every request is blocked or fails.
- Contradicts the Riftgate constraint "no new network dependency on the fast path."
- Correct as a future `ExternalTenantResolver` impl behind the same trait; not the default.

**Real-world systems.** Envoy + OPA / Styra, Istio + Ext-authz, nginx + auth_request.

## 4. Tradeoff matrix

| Property | Header passthrough | API key registry | JWT claim | External delegation |
|---|---|---|---|---|
| Hot-path overhead | ~0 ns (header read) | ~100 ns (HashMap lookup) | ~1–2 µs (RSA/ECDSA verify) | ~0.1–0.5 ms (network round-trip) |
| Credential strength | None (header trust) | Pre-shared key (HMAC-safe) | Signed token (RSA/ECDSA) | Delegated to external policy |
| Revocation latency | Instant (no credential) | On config reload / restart | JWT TTL (minutes) | Immediate (auth service controls) |
| Config complexity | Zero | Low (key-to-tenant map) | Medium (JWKS URL or PEM key) | High (auth service deployment) |
| External dependency on fast path | None | None | None (key cached at startup) | Yes (hard dependency) |
| Supports SSO / IdP integration | No | No | Yes | Yes |
| In-process verifiability | Yes | Yes (hash check) | Yes (local signature verify) | No (delegated) |
| Safe as internet-facing default | No | Yes | Yes | Yes |
| v1.0 complexity | Negligible | Low | Medium | High |

## 5. Foundational principles

**API key authentication** is the dominant model for developer-facing API authentication (REST APIs, LLM providers). A pre-shared bearer token that the gateway validates against a config-defined registry is both the simplest and most widely understood pattern. OpenAI, Anthropic, GitHub, Stripe, Twilio — every major developer API uses this model.

**Zero-trust networking** (BeyondCorp, Google's internal network paper; John Kindervag's zero-trust model) argues that network position cannot substitute for identity verification. Header passthrough is only safe when the network enforces identity externally; API key registry works at any network boundary.

**Pluggable policy behind a trait** mirrors the project's core discipline. The `CapabilityBroker` trait absorbed MCP authorization; a `TenantResolver` trait absorbs credential-to-identity translation. Future impls (`JwtTenantResolver`, `ExternalTenantResolver`) can land without changing callers, in the same pattern as `AllowlistBroker` vs `DryRunBroker`.

**Stored key hashing.** Storing SHA-256 hashes of API keys in config (rather than plaintext keys) follows the same principle as password hashing: even if the config file is read by an attacker, raw keys cannot be reconstructed. The gateway computes `SHA-256(incoming_key)` and compares against the stored hash. This is the same model used by GitHub for personal access tokens.

## 6. Recommendation

**API key registry** (`3.2`) is the production default for v1.0. **Header passthrough** (`3.1`) is retained as an explicit `mode = "trusted-header"` opt-in for trusted internal deployments (service mesh, local dev).

The implementation introduces a `TenantResolver` trait in `riftgate-core`:

```rust
pub trait TenantResolver: Send + Sync {
    /// Resolve a request's tenant identity from its HTTP headers.
    /// Returns `None` if the credential is absent or invalid.
    fn resolve(&self, headers: &http::HeaderMap) -> Option<TenantIdentity>;
}
```

Two impls ship in v1.0:

- `ApiKeyTenantResolver` in `crates/riftgate-config` (or a new `crates/riftgate-auth`): reads `Authorization: Bearer <key>`, computes SHA-256, looks up in a HashMap, returns the mapped `TenantIdentity`.
- `HeaderTenantResolver` in `riftgate-core`: reads `x-riftgate-tenant` directly (current behavior, mode = "trusted-header").

JWT claim extraction (`3.3`) lands as `JwtTenantResolver` in a future milestone when operator demand materializes. External delegation (`3.4`) lands as `ExternalTenantResolver` at v1.1+ under a new ADR.

Conditions for revisiting: if a significant fraction of Riftgate operators use existing JWT-issuing IdPs and request native JWT support.

## 7. What we explicitly reject

**Header passthrough as the default.** It is safe only on trusted networks; it cannot be the production default for any internet-facing deployment. Retained only as an explicit opt-in.

**External delegation as the default.** The latency cost (a network round-trip per request) violates the "no network dependency on the fast path" constraint. Available as a future `ExternalTenantResolver` impl.

**JWT as the v1.0 default.** JWT verification adds RSA/ECDSA overhead per request and requires configuring and managing public keys or JWKS endpoints. The API key model is simpler, has equivalent security for single-operator deployments, and is what developers already expect from LLM APIs. JWT extraction ships later as an additive impl.

## 8. References

1. Ward, B., Kindervag, J. (2010). *No More Chewy Centers: Introducing the Zero Trust Model of Information Security.* Forrester Research.
2. Ward, B. (2014). *BeyondCorp: A New Approach to Enterprise Security.* Google. [research.google/pubs/pub43231](https://research.google/pubs/pub43231/)
3. Jones, M. et al. (2015). *JSON Web Token (JWT).* RFC 7519. [datatracker.ietf.org/doc/html/rfc7519](https://datatracker.ietf.org/doc/html/rfc7519)
4. Jones, M., Bradley, J. (2015). *JSON Web Key (JWK).* RFC 7517. [datatracker.ietf.org/doc/html/rfc7517](https://datatracker.ietf.org/doc/html/rfc7517)
5. Envoy Proxy. *External Authorization.* [envoyproxy.io/docs/envoy/latest/intro/arch_overview/security/ext_authz_filter](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/security/ext_authz_filter)
6. GitHub. *API Authentication — Personal Access Tokens.* [docs.github.com/en/rest/overview/other-authentication-methods](https://docs.github.com/en/rest/overview/other-authentication-methods)
7. OWASP. *Authentication Cheat Sheet.* [cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)
