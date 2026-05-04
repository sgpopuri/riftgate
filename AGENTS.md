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

| Subsystem | Component context | Implementation (later) |
|-----------|-------------------|------------------------|
| IO runtime | [`docs/04-design/lld-io-runtime.md`](docs/04-design/lld-io-runtime.md) | `crates/riftgate-io-*` |
| Scheduling | [`docs/04-design/lld-scheduling.md`](docs/04-design/lld-scheduling.md) | `crates/riftgate-core` |
| Parser | [`docs/04-design/lld-parsing.md`](docs/04-design/lld-parsing.md) | `crates/riftgate-parser` |
| Storage / WAL | [`docs/04-design/lld-storage.md`](docs/04-design/lld-storage.md) | `crates/riftgate-replay` |
| Allocator | [`docs/04-design/lld-allocator.md`](docs/04-design/lld-allocator.md) | `crates/riftgate-core` |
| Timers | [`docs/04-design/lld-timers.md`](docs/04-design/lld-timers.md) | `crates/riftgate-core` |
| Routing | [`docs/04-design/lld-routing.md`](docs/04-design/lld-routing.md) | `crates/riftgate-router` |
| Observability | [`docs/04-design/lld-observability.md`](docs/04-design/lld-observability.md) | `crates/riftgate-obs` |

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

### Code (when Rust lands)

- Rust, stable toolchain, MSRV pinned in `rust-toolchain.toml`.
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

### The source-systems references

Options docs and LLDs cite chapter-level references to a private source-systems curriculum (chapters on IO models, io_uring, ring buffers and zero-copy, work-stealing, FSM-based parsing, allocators, timer wheels, eBPF, etc.). Citations appear as plain-text chapter titles only — there is no public sibling repo to link to. If you propose to deepen the rationale of an Options doc, attribute the chapter by title and number; do not invent a hyperlink.
