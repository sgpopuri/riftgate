# ADR 0015. MCP as a first-class citizen of the extension plane (gateway-as-broker)

> **Date:** TBD (target acceptance: at the open of `v0.5`)
> **Status:** proposed
> **Options doc:** [026-mcp-orchestration](../05-options/026-mcp-orchestration.md)
> **Deciders:** Sriram Popuri

## Context

[Model Context Protocol](https://modelcontextprotocol.io/) emerged from Anthropic in late 2024 as a JSON-RPC-shaped protocol for exposing tools, resources, and prompts to LLMs. By mid-2026 it is the de-facto agentic-era tool-use surface. Riftgate's positioning — *the programmable AI data plane* — is incomplete without a considered answer to MCP. Full exploration of the four candidate postures (passthrough, inspector, broker, mediator) and the tradeoff matrix lives in [Options `026`](../05-options/026-mcp-orchestration.md).

The forces summarized: declining to address MCP before `v1.0` would leave an obvious hole in Riftgate's agentic-era positioning; inventing a fourth differentiation pillar around MCP would violate the three-pillar discipline established in [Vision §3](../00-vision.md). The compromise is to absorb MCP into the existing extension plane as a first-class feature: a new trait alongside `Filter` and `Router`, with the same pluggability discipline.

## Decision

**Model Context Protocol orchestration lives inside the existing extension plane via a new `CapabilityBroker` trait with a per-tenant allowlist and a WAL-backed audit trail; it is not a fourth plane.**

The discipline:

- The `CapabilityBroker` trait is defined in `riftgate-core` per the sketch in [Options `026` §6](../05-options/026-mcp-orchestration.md). `authorize(mcp_request, identity) -> CapabilityDecision` is the single method.
- The MCP parser ships in `crates/riftgate-mcp` and understands `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, `prompts/get`, plus the protocol-lifecycle methods (`initialize`, `ping`, `shutdown`). The parser does not rewrite payloads — that is the mediator posture, deferred to `v1.0+`.
- The default impl, `AllowlistBroker`, composes a bit-set over the tool registry, a prefix trie over resource URIs, and an interval tree for time-bounded grants per [`docs/04-design/lld-mcp-capability.md`](../04-design/lld-mcp-capability.md).
- Every authorization decision (allow or deny) produces an `McpAuditEvent` written to the existing WAL via `riftgate-replay` and emitted to OTel as a structured log.
- Allowed requests carry attestation headers (`riftgate-mcp-caller`, `riftgate-mcp-tool`, `riftgate-mcp-decision`, plus an HMAC signature) so downstream MCP servers can independently cross-check the gateway's decision.
- A `DryRunBroker` wrapper logs decisions but always returns `Allow`; operators enable it via `enforce = false` in config to calibrate the allowlist before flipping to enforce.

## Consequences

- **Positive:**
  - The agentic-era posture is filled in without inventing a new plane: capability brokering is just another extension-plane feature alongside filters and routing strategies.
  - Operators get a typed policy surface ("tenant X cannot reach `filesystem-write`") that lives at the gateway, with a durable audit ledger that reuses the existing WAL.
  - Capability-based security (KeyKOS / EROS / seL4 lineage; Mark Miller, *Robust Composition*) is the pedagogical core of the resulting LLD and ADR — a high-value addition to the documented design corpus.
  - Attestation headers enable defense-in-depth: a downstream MCP server can independently verify the gateway's decision via the HMAC signature.
  - Future external policy delegation (OPA, Cedar) lands as a new `ExternalPolicyBroker` impl without breaking callers.
- **Negative / accepted tradeoffs:**
  - As MCP evolves, the parser carries an upgrade tax — every protocol revision is a parser update. This is a known cost of "understand the protocol" versus passthrough.
  - The HMAC signing key needs a rotation story; we start with a static key per gateway process and document the upgrade path.
  - A misconfigured allowlist can deny legitimate traffic, which is why dry-run mode (`enforce = false`) is part of the first ship.
  - We accept that the in-proc allowlist is per-instance; cross-replica policy coherence is the operator's responsibility (same posture as the rate limiter; see [ADR `0009`](0009-rate-limiter-trait-in-proc-only.md)).
- **Future work this enables:**
  - `ExternalPolicyBroker` (OPA / Cedar over gRPC) as a future impl behind a feature flag.
  - Tool-dependency DAG evaluation (topological-sort-checked invocations) layered on top of `CapabilityBroker` if and when it becomes a first-class need.
  - Mediator-level rewriting (filter `tools/list` per tenant; substitute resource URIs) in `v1.0+` if protocol stability and operator demand both support it.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship MCP as a separate, fourth differentiation pillar; it stays inside the extension plane.
  - Riftgate will not rewrite MCP payloads in `v0.5`; the mediator posture is explicitly out of scope for the first ship.
  - Riftgate will not embed a full policy engine (OPA, Cedar) as the default in `v0.5`; the static-allowlist `AllowlistBroker` is the first ship.

## Compliance

- `crates/riftgate-core::capability::CapabilityBroker` is the single trait; `AllowlistBroker` and `DryRunBroker` are the `v0.5` impls.
- `crates/riftgate-mcp` contains the MCP parser; the parser does not rewrite payloads.
- A protocol-fidelity test corpus replays recorded MCP traffic against a reference server and the Riftgate parser, with diffs caught at CI time.
- An audit round-trip test (decide N invocations, replay the WAL, verify every event) lives in `crates/riftgate-mcp/tests/wal_audit.rs`.
- An attestation-verification test in a downstream-server harness validates the HMAC signature on allowed requests.
- Adding a new `CapabilityBroker` impl that delegates externally (e.g. `ExternalPolicyBroker`) requires a new ADR superseding this one and a demonstration that the audit-event emission is the same as `AllowlistBroker`.

## Notes

- The decision to absorb MCP into the existing extension plane (rather than inventing a fourth pillar) is captured in [Options `026` §6](../05-options/026-mcp-orchestration.md) and is the central trade-off of this ADR. The three-pillar discipline (programmable Rust core + WASM extensions, documentation-first build, integrated eBPF observability) is the brand frame; MCP support is a load-bearing feature that lives inside one of those pillars.
- The naming `CapabilityBroker` (rather than `McpAuthorizer` or similar) is deliberate: the trait is a generalization of capability-based access control, and a future non-MCP capability surface (some other agentic-era protocol) can implement the same trait. The MCP-specific parsing is in `riftgate-mcp`; the trait is protocol-agnostic.
- The decision to ship `AllowlistBroker` first (rather than starting with `ExternalPolicyBroker`) is in line with [Vision §3.2](../00-vision.md): documentation-first means we ship the substrate that an operator can read and understand end-to-end, then add the external-engine option for users who already run OPA or Cedar.
