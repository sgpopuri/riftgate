# Contributing to Riftgate

Riftgate is built in public as a teaching artifact. Contributions of every shape are welcome — including, importantly, contributions to the *design discourse* before any code lands.

## What kind of contribution should I make?

Right now, in the `v0.0` public design phase, the most valuable contributions are:

- **Critique of an existing Options doc.** Did we miss a candidate? Misread a tradeoff? Open an issue or a pull request that proposes the change.
- **A new Options doc proposal.** If we have an unaddressed design decision that should be in `docs/05-options/`, propose one. Use the [`_template.md`](docs/05-options/_template.md). Number it consecutively and add it to the [index](docs/05-options/README.md).
- **An ADR critique.** If a chosen direction in `docs/06-adrs/` no longer holds, open an ADR-supersession PR that links to a new ADR and explains the reasoning.
- **A clarifying question that becomes a glossary entry.** If something in the docs is unclear, that's a documentation bug. Help us fix it.

When code lands (`v0.1` and beyond):

- Bugs and small features through normal pull requests, gated by the relevant ADR.
- New routing strategies, filters, observability sinks via the trait extension points.
- Microbenchmarks against real workloads (we want honest numbers, not vendor numbers).

## The contribution workflow

1. **Open an issue first.** Describe the problem or proposal. Link to existing Options docs or ADRs that are relevant.
2. **Discuss the shape.** A maintainer will respond within a week (this is a small project — please be patient).
3. **Submit a PR** that updates the appropriate doc and any code or tests.
4. **The PR description references the Options doc and ADR** that justify the change. If it doesn't yet exist, write it as part of the PR.

## Style and conventions

- Prose voice: precise, unhurried, honest about tradeoffs. American English spelling throughout.
- ADRs follow the Michael Nygard format (see [`docs/06-adrs/_template.md`](docs/06-adrs/_template.md)).
- Options docs follow the [`_template.md`](docs/05-options/_template.md) — exhaustive, citation-rich, decision-recommending but not yet decision-final (the decision lives in the corresponding ADR).
- Mermaid diagrams: no spaces in node IDs, quote labels with parentheses or other special characters, avoid `end` / `subgraph` / `graph` / `flowchart` as node IDs. The full convention list is in [`AGENTS.md`](AGENTS.md) §11.
- No emojis in docs or code.

## Working with AI agents on this project

Before letting any agent edit files in this repo, read [`AGENTS.md`](AGENTS.md). It defines the component context, project context, loading protocol, invariants, and verification checklist. PRs that violate these expectations will be closed with a request to re-do the work under the harness.

This is not bureaucracy. Riftgate treats context as runtime engineering input; the harness is the mechanism that makes that real.

## Code of conduct

Be substantive, be specific, be kind. Disagreements are welcome and expected — they are how a documentation-first project arrives at correct answers. Personal attacks, harassment, or bad-faith engagement get one warning, then a permanent ban. The bar is professional, not friendly.

## License

By contributing, you agree your contributions are licensed under [Apache-2.0](LICENSE).
