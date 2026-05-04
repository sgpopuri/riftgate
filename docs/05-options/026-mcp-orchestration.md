# 026. MCP Orchestration

> **Status:** `recommended` — gateway-as-broker: a first-class `CapabilityBroker` trait in the extension plane, per-tenant allowlist, WAL audit. Target milestone: `v0.5`. See ADR `0015` (reserved).
> **Source-systems chapters:** `systems/ch12 (system design patterns)` (ambassador pattern, capability-based security, interception), `systems/ch11 (WAL, journaling, recovery)` (reusing the WAL as an audit ledger)
> **Sibling-book chapters:** `advanced/ch08 (design of data structures)` (allowlist data structures — prefix trie, interval tree for time-bounded grants, bit-set allowlists), `graphs/ch03 (topological sort and DAGs)` (optional, if tool-dependency graphs become relevant)
> **Related options:** [`009 — request log`](009-request-log.md) (WAL we reuse for audit), [`016 — extension mechanism`](README.md) (where MCP plumbing lives), [`017 — multitenancy`](README.md) (the tenancy model MCP rides on)
> **Related ADR:** ADR `0015` (reserved)

## 1. The decision in one sentence

> Where, and at what depth, does Riftgate inspect and authorize [Model Context Protocol](https://modelcontextprotocol.io/) (MCP) requests — the agentic-era protocol by which models reach tools and resources — and how does that posture survive contact with hostile tenants and rapidly evolving upstream MCP servers?

## 2. Context — what forces this decision

MCP emerged from Anthropic in late 2024 as a JSON-RPC-shaped protocol for exposing tools, resources, and prompts to LLMs. By mid-2026 it is the de-facto agentic-era tool-use surface for Anthropic, several OpenAI integrations, and the open-source agent frameworks that ship against it. Riftgate's positioning — *the programmable AI data plane* — is incomplete without a considered answer to MCP.

Three forces make this load-bearing now:

1. **The gateway is becoming the capability broker.** When a model calls `tools/call` for `write_file` or `execute_sql`, somebody has to decide if this particular request in this particular tenant context is allowed to reach that tool. Historically this has lived inside the application or the MCP server; it is a natural fit for the gateway (one central policy point, one audit ledger, one authentication surface).
2. **Competitors are staking claims.** LangDB and LiteLLM have shipped basic MCP routing. None yet treats the gateway as a proper *capability broker* with a typed policy surface and a durable audit log — the gap Riftgate can honestly step into.
3. **The positioning risk of *not* doing this.** Riftgate cannot credibly describe itself as a programmable AI data plane for the agentic era if MCP requests are an opaque passthrough. Reviewers, prospective contributors, and essay readers would all notice.

Two requirements frame the choice:

- [`FR-501..504`](../01-requirements/functional.md) — the four functional requirements for MCP in `v0.5`.
- [`NFR-OBS07`](../01-requirements/non-functional.md) — the audit-log requirement.

The decision is complicated by the fact that MCP itself is young and evolving. We must pick a depth that is defensible today and extensible as the protocol matures — the same discipline that produced Options [`001`](001-io-model.md)'s epoll-first-then-io_uring stance for the IO model.

## 3. Candidates

We evaluate four candidates ordered by depth, from "don't touch it" to "actively rewrite it."

### 3.1. Gateway-as-passthrough (no MCP awareness)

**What it is.** Riftgate treats MCP traffic as opaque JSON-RPC payloads. The gateway parses HTTP, applies generic filters, and forwards to the backend MCP server. No MCP-aware logic; no allowlist; no audit beyond the generic request log.

**Why it's interesting.**
- Zero additional code to maintain as MCP evolves.
- No new trait; no new subsystem; no new testing surface.
- Consistent with how most proxies treat application-layer protocols today — they do not understand them, they route them.

**Where it falls short.**
- **Defeats the positioning.** We would be claiming to be a programmable AI data plane while declining to inspect the single most interesting agentic-era protocol.
- **No tenant-scoped policy.** An operator who wants to say "tenant X cannot reach `filesystem-write` tools" has nowhere to put that policy at the gateway layer.
- **No audit ledger at a useful granularity.** The request log captures bytes, not tool invocations. A post-incident query like "show me every `execute_sql` call made by service Y last week" has no answer.
- **Leaves the capability-broker opportunity on the table.** Exactly the kind of scope abdication that lets a competitor define the category.

**Real-world analog.** Most generic L7 proxies for opaque JSON-RPC protocols (raw HAProxy, Nginx reverse proxy on MCP).

### 3.2. Gateway-as-inspector (audit-only)

**What it is.** The gateway parses MCP requests enough to extract the tool name, resource URI, or prompt name. It logs every invocation for later audit. It does not enforce any policy — every request goes through — but the audit ledger is durable and queryable.

**Why it's interesting.**
- Minimal policy surface — no "should this be allowed" decisions, just "what happened."
- Makes post-incident forensics tractable.
- A cheap first step: audit only is a plausible `v0.5` if we want to land small and iterate.

**Where it falls short.**
- **Still leaves the tenant-policy gap open.** Operators asking "can I stop tenant X from reaching `filesystem-write`" get a shrug.
- **Two-phase rollout.** Ship audit-only in `v0.5`, then ship broker in `v0.6`. We spend most of the effort twice because the parser, the schema, and the trait surface all need to be built either way.
- **Understated differentiation.** "The gateway logs MCP invocations" is a less interesting story than "the gateway brokers MCP capabilities."

**Real-world analog.** OTel-only MCP integrations that trace but do not enforce; nascent Envoy WASM filters that log `tools/call` without authorizing.

### 3.3. Gateway-as-broker (enforce an allowlist)

**What it is.** The gateway parses MCP requests, looks up the caller's tenant identity, checks that identity against a configured allowlist of tools and resources, and either forwards with attestation headers or rejects with a structured denial. Every decision — allow or deny — is written to the WAL.

The allowlist is typed and expressive:

```toml
# riftgate.toml (sketch)
[mcp.tenants.acme]
allowed_tools = ["search-web", "read-file/ro"]
denied_tools  = ["filesystem-write", "execute-shell"]
allowed_resources_prefixes = ["s3://acme-datasets/*", "https://docs.acme.com/*"]
time_bounded_grants = [
  { tool = "send-email", until = "2026-06-01T00:00:00Z" },
]
```

**Why it's interesting.**
- **Matches the positioning.** The gateway is genuinely programmable and genuinely the capability broker.
- **Clean trait.** `CapabilityBroker` takes an MCP request and a tenant identity, returns `Allow { attestation_headers }` or `Deny { reason }`.
- **Reuses existing infra.** The audit ledger lives in the existing WAL; the tenant identity comes from the existing multi-tenancy config surface (Options [`017`](README.md)); policy loads from the existing config / CRDs.
- **Enables downstream policy engines.** Attestation headers (`riftgate-mcp-caller`, `riftgate-mcp-tool`, `riftgate-mcp-decision`) let an MCP server independently cross-check the gateway's decision, enabling defense in depth.
- **The pedagogical sweet spot.** Capability-based security is a rich teaching topic (a direct descendant of KeyKOS, EROS, seL4) that maps cleanly to this design — making it a high-value addition to the documented design corpus.

**Where it falls short.**
- **Policy expressiveness is a moving target.** As MCP evolves, what operators want to express about "who can reach what" will grow. We will be writing policy-language extensions for a while.
- **Every reject is a potential production incident.** A misconfigured allowlist can deny legitimate traffic. Operators need a dry-run mode (propose-but-allow) before enforcement — this is additional scope.
- **Attestation header design is forward-compatibility-sensitive.** If we name them badly, we are stuck with them.

**Real-world analog.** Cedar (AWS), Open Policy Agent (OPA) used as an external authorizer in service meshes, Envoy's RBAC filter. None of those is MCP-native; Riftgate would be.

### 3.4. Gateway-as-mediator (rewrite tool lists per tenant)

**What it is.** Beyond brokering, the gateway rewrites MCP traffic actively: on `tools/list`, it filters the list returned to the client based on tenant allowlist; on `tools/call`, it potentially rewrites arguments (e.g. scope `filesystem-read` paths to the tenant's sandbox prefix); on `resources/*`, it may substitute tenant-scoped URIs.

**Why it's interesting.**
- Ultimate programmability — the gateway is not just a broker, it is a mediator and a sandboxer.
- Handles the "hide capabilities from the model entirely" use case (a model that never sees `filesystem-write` is safer than one that sees it and is told "no").

**Where it falls short.**
- **Protocol-fidelity risk.** Rewriting MCP payloads means we have to keep up with every protocol revision, every vendor extension, every optional field. One missed case breaks a tenant's agent.
- **Upgrade tax.** Every upstream MCP server version bump is a potential gateway compatibility bug.
- **Increases the blast radius of gateway bugs.** A broker that wrongly denies is loud; a mediator that wrongly rewrites is silent and far worse.
- **Scope creep.** This is the kind of surface that starts as a small feature and becomes a multi-year project (see: Envoy's `ExtProc`).

**Real-world analog.** Envoy external-processor (ExtProc) in sophisticated mesh deployments; some corporate SaaS MCP proxies that advertise "fine-grained tool scoping."

## 4. Tradeoff matrix

| Property | Passthrough | Inspector | Broker | Mediator | Why it matters |
|---|---|---|---|---|---|
| Implementation cost | very low | medium | medium-high | high | Effort fits a milestone-scale project. |
| MCP protocol-fidelity risk | zero | low | medium | high | A gateway that breaks tenant agents is worse than no gateway. |
| Upgrade cost as MCP evolves | zero | low | medium | high | MCP is young; churn is expected. |
| Operator policy surface | none | read-only audit | rich | richer but dangerous | Matches what operators actually ask for. |
| Tenant trust model | "everything goes" | "everything goes, we watch" | "policy enforced at the gateway" | "policy enforced + traffic rewritten" | Enterprise deployments need the broker-or-better story. |
| Audit-forensics story | none | strong | strong | strong | A pillar of agentic-era operability. |
| Pedagogical story | thin | okay | strong (capability security) | strong but dense | Teaching value of the resulting docs. |
| Positioning fit | weak | weak | strong | strong | Vision [§3.1](../00-vision.md) requires programmability depth. |
| Blast radius of a gateway bug | passthrough — none worse than underlying | audit drift | false denies | silent rewrites (worst) | How bad is a wrong answer? |
| Composability with WAL / multitenancy / config | n/a | reuses WAL | reuses WAL + config + multitenancy | same + rewrite scope | Smaller new surface is better. |
| Can we start here and deepen later? | n/a | yes -> broker | yes -> mediator | terminal | Incrementality matters. |

## 5. What the source-systems chapters say

From `systems/ch12 (system design patterns)`, the **ambassador pattern** is the canonical framing for what we are doing: a trusted in-path component that speaks a protocol on behalf of an application and enforces policy. The chapter's warning that ambassadors should *understand* the protocol they speak but not *originate* it applies here — the gateway brokers, it does not invent tools.

The **capability-based security** material in the same chapter is directly applicable. A capability is "the right to invoke a named operation on a named resource." MCP's `tools/call` is exactly this shape; a per-tenant allowlist is exactly the capability set. This is the pedagogical core of the resulting LLD and ADR.

From `systems/ch11 (WAL, journaling, recovery)`, the insight is that an audit log is architecturally *exactly* a WAL: append-only, recoverable, replayable. Reusing `riftgate-replay` for MCP audit events costs near-zero new infrastructure. A forensic query ("replay every `execute_sql` call by tenant X on Tuesday") is a WAL replay with a filter predicate.

From `advanced/ch08 (design of data structures)`, the allowlist data-structure design space is richer than it first appears:

- A **prefix trie** for resource URIs (`s3://acme-datasets/*`) gives O(|path|) lookup and natural support for hierarchical grants.
- An **interval tree** for time-bounded grants (`until = 2026-06-01T00:00:00Z`) handles the "this permission expires" use case in O(log n).
- A **bit-set** over a fixed tool registry gives O(1) allow/deny on the common path — useful when the tool set is small and enumerable.

A production impl is likely a *composition* of these: bit-set for the common case, trie for resource prefixes, interval tree for time-bounded overrides.

From `graphs/ch03 (topological sort and DAGs)`, if (as anticipated) tool-dependency graphs become a feature — "tool B may only be called after tool A has been called in this session" — a DAG-based policy representation with topological-sort-checked invocations is the shape that fits. We do not need this in `v0.5`; we note the door.

## 6. Recommendation

**Gateway-as-broker (§3.3).** A new `CapabilityBroker` trait in `riftgate-core`, an MCP-aware request-parser in `crates/riftgate-mcp`, a per-tenant allowlist loaded from the existing config surface, and audit events written to the existing WAL.

Concretely:

1. **Trait in `riftgate-core`:**
   ```rust
   pub enum CapabilityDecision {
       Allow { attestation: AttestationHeaders },
       Deny { reason: DenialReason, retry: RetrySemantics },
   }

   pub trait CapabilityBroker: Send + Sync {
       fn authorize(&self, mcp_request: &McpRequest, identity: &TenantIdentity)
           -> CapabilityDecision;
   }
   ```
2. **First impl, `AllowlistBroker`,** composes the data structures of §5: bit-set for bounded tool registries, trie for resource-URI prefixes, interval tree for time-bounded grants.
3. **Parser in `crates/riftgate-mcp`** understands MCP `tools/list`, `tools/call`, `resources/*`, `prompts/*` — enough to make a decision. It does *not* rewrite payloads (see `v0.5` scope; mediator is `v1.0+`).
4. **Audit flow:** every call produces an `McpAuditEvent` that is appended to the WAL via the existing `riftgate-replay` infrastructure and emitted to OTel as a structured log.
5. **Attestation headers** on allowed requests: `riftgate-mcp-caller` (tenant ID), `riftgate-mcp-tool` (tool name), `riftgate-mcp-decision` (`allow`), plus a HMAC signature so downstream servers can verify the gateway made the call.
6. **Dry-run mode** for rollout: `enforce = false` logs decisions without denying, so operators can calibrate the allowlist before flipping to enforce.

### Conditions under which we'd revisit

- If MCP's protocol evolution outpaces our parser maintenance burden, we re-evaluate whether mediator-level rewriting is still a `v1.0+` goal or should be dropped entirely.
- If tool-dependency DAGs become a first-class need (Options [`026`](026-mcp-orchestration.md) extensions), we layer a DAG-policy representation on top of the `CapabilityBroker` — the base trait does not change.
- If OPA or Cedar emerges as the operator-preferred policy language, we add an `ExternalPolicyBroker` impl that delegates to one of them — without breaking the `AllowlistBroker` default.

## 7. What we explicitly reject

- **Gateway-as-passthrough.** Violates Vision [§3.1](../00-vision.md) programmability and defeats the agentic-era positioning. Reconsider only if MCP is superseded by a fundamentally different protocol.
- **Gateway-as-inspector only.** A two-phase rollout (audit-only, then enforce) doubles the integration cost without delivering the policy surface operators ask for. Reject as a terminal state; acceptable only as an intermediate rollout mode (which we already get via the broker's `enforce = false` dry-run).
- **Gateway-as-mediator in `v0.5`.** The protocol-fidelity and blast-radius risks are too high for the first-ship milestone. Revisit in `v1.0+` if protocol stability and operator demand both support it.
- **MCP as a fourth differentiation pillar.** Violates the three-pillar discipline. MCP support is a first-class extension-plane feature; the three pillars (programmable core + WASM, documentation-first build, eBPF observability) remain the brand frame. See Vision [§3](../00-vision.md).
- **Embedding a full policy engine (OPA / Cedar) as the default.** Too much new surface for `v0.5`. We ship a statically-configured allowlist first; we design the trait so an OPA-backed broker can land as a future impl without breaking callers.
- **Per-request dynamic capability introspection** (e.g. asking the model to declare its intent and matching against intent-level policy). Interesting research direction; not a `v0.5` shape.

## 8. References

1. Model Context Protocol specification — <https://modelcontextprotocol.io/>
2. Anthropic, *Introducing the Model Context Protocol* (announcement post) — original positioning.
3. Jonathan Shapiro et al., *EROS: a fast capability system* — the academic grounding for the capability-broker framing.
4. AWS, *Cedar language specification* — <https://www.cedarpolicy.com/>
5. Open Policy Agent — <https://www.openpolicyagent.org/>
6. Envoy Proxy, *External Authorization filter* documentation — the gateway-as-authorizer design pattern we are channeling.
7. Riftgate source-systems chapter `Ch12 (system design patterns)`
8. Riftgate source-systems chapter `Ch11 (WAL, journaling, recovery)`
9. Riftgate sibling-book chapter `advanced/ch08 (design of data structures)`
10. Riftgate sibling-book chapter `graphs/ch03 (topological sort and DAGs)`
