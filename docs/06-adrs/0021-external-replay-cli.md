# ADR 0021. External `riftgate-replay` CLI binary in `crates/riftgate-replay` with `dump`, `replay`, `eval` subcommands

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [019-replay-eval](../05-options/019-replay-eval.md)
> **Deciders:** Sriram Popuri

## Context

`v0.2` shipped `crates/riftgate-replay` as a library-only crate implementing the `WAL` trait ([ADR `0013`](0013-append-only-file-wal.md)). v0.3 requires a tool that lets operators (a) inspect WAL segments from a shell, (b) replay recorded traffic against an alternative configuration, and (c) evaluate recorded or replayed responses against an eval-set. Five shapes were evaluated in [Options `019`](../05-options/019-replay-eval.md): no CLI, embedded HTTP endpoint, external CLI binary, CLI + HTTP, and Python bindings + Jupyter. The CLI shape satisfies `NFR-OPS03` (single binary, no runtime), composes with shell pipelines, decouples replay from the live gateway, and lets the eval surface compose with the existing WAL as the corpus.

## Decision

**`v0.3` ships `riftgate-replay` as a new binary target in the existing `crates/riftgate-replay` crate with three subcommands — `dump`, `replay`, `eval` — each emitting structured JSON to stdout by default and optionally exporting to OTel with `riftgate.run.kind = "replay"`. The library API remains. Embedded HTTP endpoint and Python bindings are explicitly deferred / rejected.**

- New binary target: `[[bin]] name = "riftgate-replay" path = "src/bin/riftgate-replay.rs"`. The library and binary share the existing workspace dependencies; no new crate is created.
- `dump --segments ... [--from] [--to] [--filter] [--format json|jsonl|csv]` — decode WAL entries to stdout.
- `replay --segments ... --config <PATH> [--compare-against-recorded] [--rate-multiplier] [--otel-export]` — construct an in-process Riftgate driver with the supplied config, re-run recorded requests, emit per-request and summary deltas.
- `eval --segments ... --eval-set <TOML> [--config] [--fail-fast] [--exit-code-on-fail N]` — evaluate recorded or replayed responses against a TOML eval-set (schema graders, regex-absence graders, aggregate-metric graders).
- Default sink is `JsonStdoutSink`; opt-in OTel export uses a separate OTLP destination from the live gateway.
- All emitted telemetry carries `riftgate.run.kind = "replay" | "eval" | "dump"` and `riftgate.run.id = <ULID>`.
- Exit codes: zero on success; non-zero on any task fail, any decoding error, or any divergence beyond a configurable threshold.

## Consequences

- **Positive:**
  - Single binary satisfies `NFR-OPS03` (no runtime, no container required for the tool itself).
  - Decoupled from the live gateway; replay is a workshop activity, not a production activity.
  - Composes with `jq`, `grep`, and shell pipelines via stdout JSON.
  - Eval composition (`FR-X05`) lands free; the WAL remains the single corpus source-of-truth.
  - CI can wrap `riftgate-replay eval` and gate deployments on exit code.
  - Library API stays; future embedded HTTP endpoint is trivial to add (calls the same library functions) without breaking the CLI.
- **Negative / accepted tradeoffs:**
  - One additional binary in the workspace; build pipeline now produces `riftgate` and `riftgate-replay`. Trivial.
  - CLI argument shape is now versioned; we commit to backward-compatible arguments within a minor cycle.
  - Replay against real upstream backends (`upstream = "real"`) costs real upstream-token spend; default is `upstream = "recorded"` to avoid surprise.
  - Eval-set graders in v0.3 are declarative only (schema / regex-absence / aggregate); custom imperative grading is not yet expressible.
- **Future work this enables:**
  - WASM-grader hook ([Options `016`](../05-options/016-extension-mechanism.md)) lands as a v0.4+ extension to the eval surface using the same WIT shape and different host functions.
  - Embedded HTTP endpoint can land later (v0.4+) by reusing the library functions the CLI already calls — both interfaces coexist.
  - Web-UI on replay reports composes on top of the JSON output format.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship Python bindings (`pyo3`) in v0.3.
  - Riftgate will not invent a new eval-corpus format; the WAL is the corpus.
  - Riftgate will not couple replay execution to the live gateway process in v0.3.

## Compliance

- `crates/riftgate-replay/Cargo.toml` declares the `[[bin]]` target.
- `crates/riftgate-replay/tests/cli_smoke.rs` exercises each subcommand against a recorded WAL fixture and asserts the documented exit-code behaviour.
- `crates/riftgate-replay/tests/eval_schema.rs` asserts the eval-set TOML parser accepts the documented schema and rejects malformed inputs with a clear error.
- The CLI's `--help` output is snapshotted in `crates/riftgate-replay/tests/snapshots/`; argument-shape changes require an explicit snapshot update reviewed in the PR.
- All emitted telemetry includes `riftgate.run.kind` (lint: `crates/riftgate-replay/tests/telemetry_tagging.rs`).
- Adding a new declarative grader (schema variant, regex variant, aggregate metric) does not require a new ADR; adding an imperative grader or changing the eval-set TOML schema does.

## Notes

- The CLI shape follows the etcd / Kafka / Tempo precedent: an operational tool that lives next to its server binary and composes with shell pipelines. Operators recognise the shape instantly.
- Python bindings + Jupyter (Option `019` §3.5) was rejected at the workspace-membership level: the cost of adding `pyo3` to the dependency closure is wrong for v0.3, even though replay is not the data path.
- The `--compare-against-recorded` flag is the load-bearing feature for testing a new config against historical traffic. The comparison granularity (per-token, per-response, per-tenant) is configurable; default is "per-response with byte-for-byte diff for the response body."
- Choosing `upstream = "recorded"` as the replay default protects operators from surprise upstream-token charges; `upstream = "real"` is opt-in and must be explicit on the command line.
