# AGENTS.md — Riftgate's Context Harness

> If you are an agent (or a human running an agent) about to edit anything in this repository, **read this file first**, every session, before generating output. It is short on purpose.

This file applies the context-harness pattern — durable component context, temporary project context, and environment guardrails — to Riftgate itself. It is intentionally concise; it points to the surfaces that hold the actual content. **AGENTS.md at the repo root is the single agent surface** for every assistant that edits this tree.

---

## Tools: Cursor and VS Code + GitHub Copilot

The **same** load order (§1), invariants (§5, §9), and verification checklist (§7) apply regardless of editor.

| Environment | How to use this file |
|---------------|----------------------|
| **Cursor** | Open this folder as the workspace root. If Cursor exposes project rules, keep them aligned with this file; do not contradict §5 or §9 without a new ADR. |
| **Visual Studio Code + GitHub Copilot** | Open this folder as the workspace. Copilot Chat and Copilot-powered edits should treat this file as the project harness. If a session does not auto-include it, reference `AGENTS.md` explicitly (for example `@AGENTS.md` in Chat, or paste the §1 load order) before asking for code or doc changes. |

There is **no** separate Cursor-only or Copilot-only harness in this repository. If a tool-specific file under `.cursor/` or `.vscode/` exists, it is optional sugar; **this file remains authoritative** when there is a conflict.

---

## 1. Load order (every session, in this order)

```
1. THIS FILE                                        — session entry point
2. README.md                                        — project posture and status
3. docs/00-vision.md                                — north star and non-goals
4. The component context for the area you'll touch  — see §2
5. The project context for the active work          — see §3
6. The specific Options doc and ADR that govern     — see §4
```

You do not need to load everything every time. You **do** need to load the right things. If the task touches a subsystem, load that subsystem's component context. If the task touches a current decision, load the relevant Options doc and ADR. Loading order matters: broader before narrower.

---

## 2. Component context (durable)

The durable, theory-of-the-system knowledge for each subsystem lives next to the design docs.

| Subsystem | Component context | Implementation |
|-----------|-------------------|----------------|
| IO runtime | [`docs/04-design/lld-io-runtime.md`](docs/04-design/lld-io-runtime.md) | `crates/riftgate-io-epoll` (mio: epoll on Linux, kqueue on macOS) — shipped v0.1 |
| Scheduling | [`docs/04-design/lld-scheduling.md`](docs/04-design/lld-scheduling.md) | trait surface in `crates/riftgate-core`; v0.1 binary uses tokio multi-thread runtime; custom `PerCoreScheduler` in v0.2 |
| Parser | [`docs/04-design/lld-parsing.md`](docs/04-design/lld-parsing.md) | `crates/riftgate-parser` (`Http1Parser` + `SseFramer`) — shipped v0.1 |
| Storage / WAL | [`docs/04-design/lld-storage.md`](docs/04-design/lld-storage.md) | trait in `crates/riftgate-core`; `crates/riftgate-replay` impl in v0.2 |
| Allocator | [`docs/04-design/lld-allocator.md`](docs/04-design/lld-allocator.md) | `crates/riftgate-core` (`BumpArena` + `SystemAllocator`) — shipped v0.1 |
| Timers | [`docs/04-design/lld-timers.md`](docs/04-design/lld-timers.md) | `crates/riftgate-core` (`BinaryHeapTimers`) — shipped v0.1 |
| Config | [`docs/05-options/015-config-model.md`](docs/05-options/015-config-model.md) | `crates/riftgate-config` (TOML + env override + fail-loudly validator) — shipped v0.1 |
| Routing | [`docs/04-design/lld-routing.md`](docs/04-design/lld-routing.md) | `crates/riftgate-router` (`RoundRobinRouter` + `ConstantRouter`) — shipped v0.1; `WeightedRandomRouter` + `CircuitBreakerArbiter` — shipped v0.2; `KvAwareRouter` + `HedgedRouter` in v0.3 per [ADR 0022](docs/06-adrs/0022-kv-aware-routing-prefix-trie.md) and [ADR 0023](docs/06-adrs/0023-hedged-requests-p99-triggered.md) |
| Filter chain | [`docs/04-design/lld-filter-chain.md`](docs/04-design/lld-filter-chain.md) | trait in `crates/riftgate-core` (`IdentityFilter` + `LoggingFilter`) — shipped v0.1; `FilterChain` executor + `WasmFilter` in new crate `crates/riftgate-filter` in v0.3 per [Options `016`](docs/05-options/016-extension-mechanism.md) and [ADR 0019](docs/06-adrs/0019-wasm-extension-mechanism.md) |
| Observability | [`docs/04-design/lld-observability.md`](docs/04-design/lld-observability.md) | `crates/riftgate-obs` (bounded MPSC bus + `OtelSink` + `JsonStdoutSink` + `MultiSink`) — shipped v0.1; `TokenLevelAggregator`, `DcgmScrapeSource`, feature-gated `NvmlSource`, and `BpfSink` scaffold — landed in v0.4; Aya programs and verifier harnesses remain in flight |
| Binary | n/a | `crates/riftgate` (tokio runtime, accept loop, hyper-rustls upstream client, SSE forwarding, `/health` + `/ready`, SIGTERM drain) — shipped v0.1 |
| Rate limiting | [`docs/04-design/lld-rate-limiter.md`](docs/04-design/lld-rate-limiter.md) | trait in `crates/riftgate-core`; `TokenBucketLimiter` in `crates/riftgate-core` v0.2; a separate `crates/riftgate-rate-limit-*` crate emerges only if a distributed impl lands later, per [Options `021`](docs/05-options/021-rate-limiting.md) and [ADR 0009](docs/06-adrs/0009-rate-limiter-trait-in-proc-only.md) |
| MCP capability broker | [`docs/04-design/lld-mcp-capability.md`](docs/04-design/lld-mcp-capability.md) | trait in `crates/riftgate-core`; `crates/riftgate-mcp` (`AllowlistBroker`, `DryRunBroker`, parser, HMAC attestation, WAL audit) — shipped v0.5 per [Options `026`](docs/05-options/026-mcp-orchestration.md) and [ADR 0015](docs/06-adrs/0015-mcp-extension-plane-broker.md) |
| Kubernetes operator | [`docs/04-design/lld-mcp-capability.md`](docs/04-design/lld-mcp-capability.md) | `crates/riftgate-operator` (`Riftgate`, `RiftgateBackend`, `RiftgateRoute` CRDs, ConfigMap+Deployment reconciler, Helm chart) — shipped v1.0 per [Options `018`](docs/05-options/018-deployment.md) and [ADR 0030](docs/06-adrs/0030-k8s-operator-crds.md) |
| Tenant identity resolver | n/a | `crates/riftgate-core::tenant` (`TenantResolver`, `HeaderTenantResolver`) + `crates/riftgate-config` (`ApiKeyTenantResolver`, `MultitenancyConfig`) — shipped v1.0 per [Options `017`](docs/05-options/017-multitenancy.md) and [ADR 0029](docs/06-adrs/0029-api-key-tenant-resolver.md) |

Each LLD is the operating theory of one subsystem: architecture, dependencies, patterns, pitfalls, quality contract, agent guidance. Load the one(s) you'll touch. Do not infer them from nearby code.

---

## 3. Project context (temporary)

The active project context — work in flight, current spec, session logs, open questions — lives at the top of [`docs/02-mvp-roadmap.md`](docs/02-mvp-roadmap.md) under the **"Currently shipping"** section. When you start a session, read it.

If you discover a fact that should have been in the project context but was not — a missing pitfall, an outdated dependency description, an undocumented invariant — propose a context correction in the same PR. The harness only works if it is alive.

---

## 4. The decision-bearing surfaces

Every load-bearing decision in Riftgate lives in two places:

- **[`docs/05-options/`](docs/05-options/)** — the Options docs. Numbered. Each one explores 3-5 candidates exhaustively before recommending one.
- **[`docs/06-adrs/`](docs/06-adrs/)** — the ADRs. Numbered. Each one accepts a decision from an Options doc, with tradeoffs explicit.

If you are about to make a load-bearing change, find the corresponding ADR and read it. If there is no ADR, **stop and write one first** (or open an issue requesting that one be written before the change is made). The "no ADR yet" path is exactly the path that produces plausible-wrong code.

---

## 5. Load-bearing invariants

These are the project's **non-negotiable** properties. An agent that violates one of these has produced incorrect work, even if the code compiles and tests pass.

- **Source of truth is the repo.** Conference talks and external citations all derive from documents in this repo. If an external source claims something the repo does not, the source is wrong, not the repo.
- **No code without a corresponding Options doc and ADR.** Every load-bearing change in `crates/` must trace back to a documented decision. PRs that add behavior without a justifying ADR will be closed.
- **No silent change to public traits.** `riftgate-core` defines the trait surface (`AsyncIO`, `Scheduler`, `Queue`, `Allocator`, `TimerSubsystem`, `WAL`, `Filter`, `Router`). Changes to these traits require a new ADR superseding the one that established them.
- **Pluggability over performance.** When in doubt, choose the design that is easier to swap out. Riftgate is a framework, not a benchmark champion. We have explicitly declined to compete with TensorZero on raw P99.
- **Honest numbers only.** Benchmarks must be reproducible, must include the harness, must compare against a real baseline (LiteLLM, an existing Rust gateway, or a vendor-published claim with citation). No vendor-style number-fishing.
- **Anonymized war stories.** Documentation may draw on production experience but never names customers, employers, or proprietary systems.
- **No local-environment leakage.** Hostnames, user IDs, internal mirror URLs, internal paths, credentials, tokens, and other machine-local or security-sensitive values must stay in ignored local config and must never be copied into tracked code, docs, harness files, examples, commits, or PR text.

---

## 6. Freedoms

These are the places where you (agent or human) can exercise reasonable judgment without seeking explicit guidance:

- Naming inside a module, as long as it is consistent with the module's existing conventions.
- The choice of a local data structure when the surrounding code's traits and contracts are unaffected.
- Test scaffolding and fixture data.
- Refactors that preserve behavior and trait shape and that ship with their own ADR if they touch ≥3 files.

When in doubt: assume it is not a freedom; consult an ADR or open an issue.

---

## 7. Verifications you must run before finishing a turn

If you are an agent producing a change in this repo, before declaring "done":

- [ ] Have I loaded the component context for the subsystem I touched?
- [ ] Have I located the Options doc and ADR that govern this change? If neither exists, have I written one?
- [ ] Does my change preserve the public trait surface in `riftgate-core` (or supersede it via a new ADR)?
- [ ] Have I updated the relevant documentation (LLD, Options doc, README) to reflect the change?
- [ ] If I added a benchmark or perf claim, is it reproducible from the repo with `cargo bench`?
- [ ] If I learned something that contradicts existing documentation, have I proposed a context correction?
- [ ] Have I avoided introducing duplicate helpers (search before write — accretion is a real cost)?
- [ ] Have I kept local-config values out of tracked files, examples, and PR text, using placeholders or ignored config instead?
- [ ] If the change touches rate limiting, is every enforcement decision routed through the `RateLimiter` trait (no direct in-proc globals, no ad-hoc counters)?
- [ ] If the change touches MCP, does it respect the per-tenant allowlist and emit a `McpAuditEvent` to the WAL via the `CapabilityBroker` trait?

If any answer is "no" or "I'm not sure," do not finish the turn. Surface the gap in the PR description and ask the human owner.

---

## 8. The horse and the harness

The model you are using is fast, fluent, and dangerous without direction. This file, the LLDs, the Options docs, and the ADRs are the harness. Your judgment — what to keep, what to refuse, what to escalate — is the rider. Do not surrender either.

If anything in this file does not match how the project actually works, **the file is the bug, not the project**. Open an issue or fix it directly.

---

## 9. Identity and posture (non-negotiable)

These are the project-identity invariants every agent and contributor must internalize before editing anything. They overlap deliberately with §5; the repetition is intentional.

- Riftgate is a **framework, not a product**. Pluggability over raw performance.
- We do **not** compete with TensorZero on P99. We compete on documentation depth, pluggability, and integrated eBPF observability.
- Every load-bearing change traces back to an Options doc in `docs/05-options/` and an ADR in `docs/06-adrs/`. **No code without a justifying ADR.**
- The repo is the source of truth.
- War stories and examples are always anonymized.
- **No emojis anywhere in docs or code.**

### Before making changes

1. Read this file (AGENTS.md) for the full context harness.
2. Check [`docs/02-mvp-roadmap.md`](docs/02-mvp-roadmap.md) — the "Currently shipping" block at the top tells you what milestone is active and what's in flight.
3. Load the relevant LLD from `docs/04-design/lld-*.md` for whichever subsystem you are touching.
4. Find the Options doc and ADR that govern the change. If none exists, surface the gap — do not infer.

### The three differentiation pillars

1. **Programmable Rust core + WASM extensions** — every subsystem is a trait with multiple impls.
2. **Documentation-first build** — every decision is captured as an Options doc + ADR pair.
3. **Integrated eBPF observability** — gateway-internal, not bolted-on.

---

## 10. Status pointer

There is exactly one live status surface in this repo: the **"Currently shipping"** block at the top of [`docs/02-mvp-roadmap.md`](docs/02-mvp-roadmap.md). It carries:

- Active milestone (`v0.0`, `v0.1`, …).
- What is in flight.
- Open questions.
- Recent learnings.

**Maintenance contract:** when a milestone transitions, or work moves into / out of flight, update that block in the same change. Contributors who pull learn the new state from the same diff. Do not maintain status in any other file.

---

## 11. Conventions

## 11.5 Local environment profile (current)

The current implementation environment is a Lima VM (Ubuntu 24.04 LTS) running on macOS, defined in [`lima/riftgate.yaml`](lima/riftgate.yaml). Lima routes guest network through the macOS host so outbound internet access is available — `rustup`, crates.io, and `apt` all work directly with no proxy or internal tarball.

- Machine-specific identifiers and local filesystem paths must live only in ignored local config: `config/workspace.local.env` and `.cargo/config.toml`.
- Never quote or copy the concrete values from ignored local config into tracked docs, code comments, examples, commit messages, or PR descriptions; tracked surfaces must use placeholders or generic descriptions.
- Do not rely on any other machine for day-to-day build, test, or benchmark work unless the owner explicitly overrides this policy.
- `sudo` + `apt` are allowed inside the VM and should be the default path for system packages.
- Rust toolchain: installed inside the VM by the Lima provisioning script via `rustup`. The `rust-toolchain.toml` channel (stable) is picked up automatically; `scripts/cargow` injects `RUSTUP_TOOLCHAIN` when the VM hostname matches `RIFTGATE_ENV_HOST_SHORT` in `config/workspace.local.env`.
- Cargo registry: crates.io is directly accessible from the Lima VM. Do not run `scripts/render-cargo-config` or set `RIFTGATE_CARGO_REGISTRY_*` variables for the Lima profile.
- Docker Engine is available inside the VM via `sudo apt install docker.io`. Docker-based example and smoke-test flows work; image pulls succeed because the VM has internet access.

Bootstrap:

```bash
# macOS — one-time
brew install lima
limactl start lima/riftgate.yaml          # provisions Rust stable + apt packages (~5-10 min first boot)

# Inside the VM — once after first start
cp config/workspace.local.env.example config/workspace.local.env
# Set RIFTGATE_ENV_HOST_SHORT=lima-riftgate  RIFTGATE_ENV_TOOLCHAIN=stable
./scripts/cargow check --workspace --all-targets
```

VS Code Remote-SSH (run once on macOS after the VM is started):

```bash
limactl show-ssh --format config riftgate >> ~/.ssh/config
# VS Code -> Remote-SSH -> Connect to Host -> lima-riftgate
```

BPF source builds require a nightly toolchain with `rust-src`. Install inside the VM when needed:

```bash
rustup toolchain install nightly --component rust-src
```

If this profile changes (host class, network policy, Docker availability), update this section and the "Currently shipping" block in `docs/02-mvp-roadmap.md` in the same change. Never commit concrete local values.

### Code (when Rust lands)

- Rust, stable toolchain, MSRV pinned in `rust-toolchain.toml`.
- **Distribution:** through v0.4, crates are **not** published to crates.io; consumers build from the GitHub repo (`cargo build -p riftgate`). Registry publish is a **v1.0** decision only — see [`docs/02-mvp-roadmap.md`](docs/02-mvp-roadmap.md) § Distribution (crates.io). Do not add `cargo publish` or `cargo install` as a milestone requirement before then.
- `cargo clippy --deny warnings` and `cargo fmt --check` must pass.
- Public items have rustdoc. No obvious / narrating comments.
- Per-request arena allocator for hot-path memory.
- **No Python in the data path, ever.** Python is fine for tests, benchmarks, tooling.

### Documentation

- Options docs follow [`docs/05-options/_template.md`](docs/05-options/_template.md) exactly.
- ADRs follow [`docs/06-adrs/_template.md`](docs/06-adrs/_template.md) (Michael Nygard format). Short and decisive.
- LLDs in `docs/04-design/` are the component context for each subsystem.
- Voice: precise, unhurried, honest about tradeoffs.
- American English spelling throughout.
- Mermaid diagrams: no spaces in node IDs; quote labels with special characters; avoid `end` / `subgraph` / `graph` / `flowchart` as node IDs.

### The decision discipline

- No code lands without a corresponding ADR.
- Options docs recommend; ADRs decide.
- When an ADR is superseded, a new ADR is written — the old one is never edited in place.
- Tagged commits anchor decisions: `v0.N-decision-NNN-<slug>`.

### Citations and external references

Options docs and LLDs cite the underlying technology, pattern, paper, or named system that informs each decision — `io_uring`'s submission/completion ring model, work-stealing schedulers, write-ahead logging à la ARIES, and so on. Every Options doc carries a `## References` section that lists concrete external sources: RFCs, kernel documentation, papers, source repositories, well-known projects, books. The standard is that [Persona P3 (Maya, the systems-engineering learner)](docs/01-requirements/personas.md) can follow every citation without access to any private material. If you propose to deepen the rationale of an Options doc, name the technique and add the corresponding paper, RFC, or kernel-docs link to the References section.
