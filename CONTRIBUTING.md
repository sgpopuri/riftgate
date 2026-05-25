# Contributing to Riftgate

Riftgate is built in public as a teaching artifact. Contributions of every shape are welcome — including, importantly, contributions to the *design discourse* before any code lands.

## What kind of contribution should I make?

With the `v0.1` walking skeleton in the tree, valuable contributions include:

- **Critique of an existing Options doc.** Did we miss a candidate? Misread a tradeoff? Open an issue or a pull request that proposes the change.
- **A new Options doc proposal.** If we have an unaddressed design decision that should be in `docs/05-options/`, propose one. Use the `[_template.md](docs/05-options/_template.md)`. Number it consecutively and add it to the [index](docs/05-options/README.md).
- **An ADR critique.** If a chosen direction in `docs/06-adrs/` no longer holds, open an ADR-supersession PR that links to a new ADR and explains the reasoning.
- **A clarifying question that becomes a glossary entry.** If something in the docs is unclear, that's a documentation bug. Help us fix it.

For code changes:

- Bugs and small features through normal pull requests, gated by the relevant ADR.
- New routing strategies, filters, observability sinks via the trait extension points.
- Microbenchmarks against real workloads (we want honest numbers, not vendor numbers).

**Getting a binary:** clone this repo and `cargo build --release -p riftgate`. We do not publish to crates.io until the v1.0 distribution decision ([roadmap § Distribution](docs/02-mvp-roadmap.md#distribution-cratesio)).

## The contribution workflow

1. **Open an issue first.** Describe the problem or proposal. Link to existing Options docs or ADRs that are relevant.
2. **Discuss the shape.** A maintainer will respond within a week (this is a small project — please be patient).
3. **Submit a PR** that updates the appropriate doc and any code or tests.
4. **The PR description references the Options doc and ADR** that justify the change. If it doesn't yet exist, write it as part of the PR.

## Style and conventions

- Prose voice: precise, unhurried, honest about tradeoffs. American English spelling throughout.
- ADRs follow the Michael Nygard format (see `[docs/06-adrs/_template.md](docs/06-adrs/_template.md)`).
- Options docs follow the `[_template.md](docs/05-options/_template.md)` — exhaustive, citation-rich, decision-recommending but not yet decision-final (the decision lives in the corresponding ADR).
- Mermaid diagrams: no spaces in node IDs, quote labels with parentheses or other special characters, avoid `end` / `subgraph` / `graph` / `flowchart` as node IDs. The full convention list is in `[AGENTS.md](AGENTS.md)` §11.
- No emojis in docs or code.

## Code commenting discipline

Riftgate is a teaching artifact. Source comments are part of the contract: a reader landing on a file should learn the *theory* of that file, not just its mechanics. To keep that bar concrete we require **module-level ASCII diagrams** whenever the module owns any of:

1. A non-trivial **data structure with internal layout** (packed atomics, sharded maps, ring buffers, alias tables, segment files).
2. **More than one thread, task, or actor** sharing state — show who locks what, who notifies whom, where parking happens.
3. A **state machine with more than two states** (parsers, breakers, drain coordinators).
4. A **packed bit layout in an atomic** (always document the bit ranges).
5. A **flow control or fast path** worth distinguishing from the slow path.

Conventions:

- Place the diagram in the module doc (`//!`) before any `use` statement. Type-specific layouts go in the type's `///` doc.
- Fence diagrams with ```` ```text ```` inside the doc comment so rustdoc renders them in monospace without trying to compile them as Rust.
- Cite the governing ADR and LLD next to the diagram so design rationale and visualization stay co-located.
- ASCII only. No Unicode box-drawing characters (rustdoc renders them, but `grep` and many terminals do not). No emojis.
- Keep diagrams under ~30 lines. If the picture wants to be larger, link out to the LLD's Mermaid diagram and keep only the load-bearing skeleton in source.

Examples already in the tree, worth reading before adding your own:

- [`crates/riftgate/src/scheduler.rs`](crates/riftgate/src/scheduler.rs) — `ShardedMpmcQueue` data layout + worker lifecycle.
- [`crates/riftgate-core/src/rate_limit.rs`](crates/riftgate-core/src/rate_limit.rs) — packed `AtomicU64` bit layout + CAS retry fast path.
- [`crates/riftgate-router/src/circuit.rs`](crates/riftgate-router/src/circuit.rs) — 3-state breaker + decorator data flow.
- [`crates/riftgate-replay/src/file_wal.rs`](crates/riftgate-replay/src/file_wal.rs) — on-disk frame format + per-shard flusher topology.
- [`crates/riftgate-parser/src/http1.rs`](crates/riftgate-parser/src/http1.rs) — parser FSM.

This convention applies retroactively to existing modules and prospectively to every future phase. PRs that introduce a load-bearing data structure or state machine without the corresponding diagram will be sent back for one.

## Working with AI agents on this project

Before letting any agent edit files in this repo, read `[AGENTS.md](AGENTS.md)`. It defines the component context, project context, loading protocol, invariants, and verification checklist. PRs that violate these expectations will be closed with a request to re-do the work under the harness.

This is not bureaucracy. Riftgate treats context as runtime engineering input; the harness is the mechanism that makes that real.

## Code of conduct

Be substantive, be specific, be kind. Disagreements are welcome and expected — they are how a documentation-first project arrives at correct answers. Personal attacks, harassment, or bad-faith engagement get one warning, then a permanent ban. The bar is professional, not friendly.

## License

By contributing, you agree your contributions are licensed under [Apache-2.0](LICENSE).