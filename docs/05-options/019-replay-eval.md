# 019. Replay-eval

> **Status:** `recommended` — `v0.3` ships an external CLI binary `riftgate-replay` (a new binary target in the existing `crates/riftgate-replay` crate) with three subcommands: `dump`, `replay`, and `eval`. An embedded HTTP-served replay endpoint and a "no CLI, library only" stance are catalogued and rejected. See [ADR `0021`](../06-adrs/0021-external-replay-cli.md).
> **Foundational topics:** deterministic replay against a write-ahead log (ARIES lineage; CockroachDB / Datomic replay semantics), eval-set evaluation patterns from LLM-eval tooling (Anthropic's evals, OpenAI's evals, LangSmith), streaming sketches for replay-time aggregation (HyperLogLog, Count-Min Sketch, reservoir sampling), command-line argument parsing (`clap` derive), the principle that operational tools should be debuggable from a shell.
> **Related options:** [`009 — request log`](009-request-log.md) (the WAL we replay), [`013 — observability sink`](013-observability-sink.md) (replay results are emitted to the same sink surface), [`015 — config model`](015-config-model.md) (replay accepts the same TOML as the live binary), [`021 — rate-limiting`](021-rate-limiting.md) (replay can validate that limits are honoured under a recorded load).
> **Related ADR:** [ADR `0021`](../06-adrs/0021-external-replay-cli.md)

## 1. The decision in one sentence

> What shape — none / library API only / embedded HTTP endpoint / external CLI binary — does the v0.3 replay-and-evaluation tool take, given that the WAL ([Options `009`](009-request-log.md)) is already a load-bearing v0.2 deliverable and operators have asked for "rerun yesterday's traffic against a new config" since v0.1?

## 2. Context — what forces this decision

[`crates/riftgate-replay`](../../crates/riftgate-replay/) shipped in v0.2 as a library-only crate: it implements the `WAL` trait with per-shard append-only segment files (per [ADR `0013`](../06-adrs/0013-append-only-file-wal.md)) and exposes no CLI. The library shape was correct for v0.2 — the binary needed the WAL for durable audit; nothing else did. v0.3 changes the calculus on three axes:

1. **Replay is the natural follow-up to the WAL.** A request log that nothing can replay is a write-only artifact. Operators consistently ask "can I rerun yesterday's traffic against the new config?" and "can I run my evals against recorded production traffic?" The answer in v0.2 is "yes, but you write the program." In v0.3 we ship the program.
2. **Evals are agentic-era infrastructure.** Anthropic's eval framework, OpenAI's evals, LangSmith, Inspect AI — the ecosystem has settled on a shape: take a corpus of inputs, run them through a system-under-test, compare against a target. The WAL is already a corpus. The system-under-test is already a Riftgate instance (or a recorded fragment of one). The composition is natural; we either ship the tool or operators reach for an external one that does not know the WAL format.
3. **`v0.3`'s programmability story is incomplete without replay-driven validation.** A new WASM filter or a new routing strategy ([Options `016`](016-extension-mechanism.md), [`025`](025-v03-routing-strategies.md)) needs to be validated against representative traffic before production rollout. Without replay, the validation is "deploy and hope."

Three forces frame the choice:

- **Operational tools must be debuggable from a shell.** Persona P1 (Pia, platform engineer) and Persona P2 (Rohan, inference SRE) live in terminals. A replay tool that requires a programming environment to operate fails the "deployable on a Tuesday afternoon" test.
- **Replay must not be coupled to a live production binary.** Replaying recorded traffic against a config to test it is a *workshop* activity, not a *production* activity. It should not require the gateway to be running, should not contend for the live binary's resources, and should not produce telemetry that contaminates the live observability surface unless explicitly asked to.
- **The eval surface should compose with the WAL, not replace it.** If we invent a new corpus format for evals, we have two source-of-truth shapes for "recorded requests" — the WAL and the eval format — and operators correctly complain. The WAL is the corpus; the eval is a configured run over it.

Requirements this is load-bearing for:

- **`FR-205`** — replay recorded traffic against an alternative configuration; assert behavioural deltas.
- **`FR-X05`** — the eval surface; structured pass/fail outcomes per request, with summary statistics.
- **`NFR-OPS03`** — operational tools ship as single binaries; no Python, no runtime dependency, no container required.
- **`NFR-OBS06`** — replay events are tagged `riftgate.run.kind = "replay"` so they are filterable from live traffic in any downstream observability backend.

## 3. Candidates

### 3.1. None (library API only, status quo)

**What it is.** Keep `riftgate-replay` as a library. Operators wanting to replay or evaluate write Rust programs that depend on the library.

**Why it's interesting.**
- **Zero new code.** Library already exists.
- **Maximum flexibility.** Operators can do anything the library allows.
- **No CLI versioning burden.** No deprecation cycle for command-line interfaces.

**Where it falls short.**
- **Fails NFR-OPS03.** Pia and Rohan are not Rust developers in their day-to-day work; expecting them to compile a Rust program to read a WAL is operator-hostile.
- **Defeats the v0.3 narrative.** "Riftgate has a replay tool" becomes "Riftgate has a Rust library you can build a replay tool out of." The community has heard this before and rejected it.
- **Loses the eval composition.** Without a CLI, the eval workflow ("here is my eval-set, here is my target config, give me a pass/fail report") has no entry point.

**Real-world systems that use it.** Kafka's `kafka-tools` historically had this shape before the CLI matured; Postgres's WAL is library-only and famously hard to inspect from a shell.

### 3.2. Embedded HTTP endpoint (`/admin/replay`)

**What it is.** The live gateway exposes an admin endpoint that accepts a replay request: "replay segments S..E against config C, return summary." The operator POSTs a JSON request, the gateway runs the replay, the response carries the report.

**Why it's interesting.**
- **No new binary.** Reuses the existing gateway as the replay host.
- **Natural authentication.** The same auth that protects the gateway protects replay.
- **Browser-accessible.** A Web UI can call it directly.

**Where it falls short.**
- **Couples replay to a live production process.** A replay run that allocates heavily, or that exercises a buggy new filter, can take the live gateway with it. This violates the *replay-is-a-workshop-activity* principle.
- **Resource contention.** Replay against a multi-hour WAL is CPU and IO heavy; running it inside the live binary either starves live traffic or requires bulkhead infrastructure we have not yet built.
- **Authentication isn't free.** The gateway has no admin-auth surface today; adding one is a separate v0.3+ decision (`Options 017` multitenancy) that we should not block on.
- **Operator workflow is wrong.** "SSH to a host, curl an admin endpoint, parse JSON" is worse UX than "run a command."

**Real-world systems that use it.** Some service-meshes (Istio Pilot's `/debug/*` endpoints). Useful for inspection; rarely the home of long-running compute.

### 3.3. External CLI binary (`riftgate-replay`)

**What it is.** A new binary target in the existing `crates/riftgate-replay` crate (using the `[[bin]]` Cargo manifest section). The library remains; the binary calls into it. Subcommands:

- `riftgate-replay dump --segments seg-0001-*.wal` — decode and print WAL entries to stdout as structured JSON.
- `riftgate-replay replay --segments ... --config new-config.toml` — re-run recorded requests against a fresh in-process Riftgate driver loaded with `new-config.toml`; emit a comparison report.
- `riftgate-replay eval --segments ... --eval-set my-evals.toml` — for each recorded request, evaluate a set of assertions (status code, response schema, content predicates) against the recorded *or* replayed response.

**Why it's interesting.**
- **Single binary, single artifact.** Builds with the rest of the workspace; ships alongside `riftgate` from `cargo build --release`.
- **Operator-friendly.** Standard CLI conventions (`--help`, `--version`, exit codes, structured stdout output).
- **Composes with shell pipelines.** `riftgate-replay dump ... | jq ...` is a natural workflow.
- **Decoupled from the live gateway.** Runs against a separate config; emits to a separate observability sink (default: stdout JSON, opt-in OTel export with `riftgate.run.kind = "replay"` attribute).
- **Eval surface comes free.** `eval` subcommand is the natural shape for the v0.3 deliverable.

**Where it falls short.**
- **CLI versioning has to be designed.** We commit to backward-compatible argument-shape for one minor cycle. Manageable.
- **Two binaries in the workspace.** The build pipeline now produces `riftgate` and `riftgate-replay`. Trivial; only a docs concern.
- **An operator with a UI preference still wants HTTP.** We can ship the CLI in v0.3 and the embedded HTTP endpoint later (it composes — the HTTP endpoint can call the same library functions). v0.3 picks the CLI first because it is the more-load-bearing artifact.

**Real-world systems that use it.** Kafka's `kafka-console-consumer`, `kafka-replay`. Etcd's `etcdctl`. Tempo's `tempo-cli`. Mature shape across the operational-tooling space.

### 3.4. External CLI + embedded HTTP endpoint (both)

**What it is.** Ship the CLI as in 3.3, *and* expose the same library through an admin HTTP endpoint.

**Why it's interesting.**
- Both interfaces available.
- HTTP endpoint enables future Web UI.

**Where it falls short.**
- **Doubles the surface to test.** Two parsing paths, two auth paths, two telemetry-tagging paths.
- **The HTTP path inherits all the v3.2 concerns** (resource contention with live traffic, admin-auth dependency).
- **Strictly more scope than v0.3 needs.** The CLI fulfils the FR-205 and FR-X05 requirements alone; the HTTP path is a v0.4+ enhancement at earliest.

**Real-world systems that use it.** Mature systems eventually grow to this shape (Kubernetes API server + `kubectl`). v0.3 is too early.

### 3.5. Embedded library + Jupyter notebook examples

**What it is.** Keep the library; ship a Python-binding (via `pyo3`) and a set of example Jupyter notebooks demonstrating replay and eval workflows.

**Why it's interesting.**
- Data-science-team-friendly.
- Notebook-first workflow matches some eval tooling (LangSmith, Inspect AI).

**Where it falls short.**
- **Violates the "no Python in the data path, ever" stance** (AGENTS.md Conventions). Replay is not the data path, but adding `pyo3` to the workspace is the camel's nose.
- **Adds a non-Rust dependency.** Operators who do not run Python are stuck.
- **The Jupyter dependency** is heavy for `NFR-OPS03`.
- **Eval is not notebook-first for our personas.** Pia and Rohan want exit codes from CI; notebooks are P3 (Maya) territory.

**Real-world systems that use it.** Eval frameworks aimed at data scientists (Inspect AI). Not the shape for a gateway tool.

## 4. Tradeoff matrix

| Property | 3.1 None | 3.2 HTTP | 3.3 CLI | 3.4 Both | 3.5 Python | Why it matters |
|---|---|---|---|---|---|---|
| Fits `NFR-OPS03` (single binary, no runtime) | n/a | yes | **yes** | yes | no | Operator deployability. |
| Decoupled from live gateway | n/a | no | **yes** | partial | yes | Replay is a workshop activity. |
| Operator-debuggable from a shell | no | weak (curl) | **yes** | yes | weak | Pia / Rohan persona fit. |
| Composes with shell pipelines | no | no | **yes** | yes | no | `jq`, etc. |
| Eval composition (FR-X05) | no | yes | **yes** | yes | yes | v0.3 deliverable. |
| Build artifacts to ship | 0 new | 0 new | 1 new binary | 1 new + 1 endpoint | binary + py-wheel | Build pipeline cost. |
| Authentication burden | none | requires admin auth | local file access | both | local | v0.3 we can do without admin auth. |
| Cross-platform | n/a | n/a | yes | yes | Python-on-platform | macOS dev story. |
| Resource contention with live | n/a | high | none | mixed | none | NFR-P05 reliability. |
| Migration cost if we add the other later | n/a | trivial | **trivial** (library shared) | n/a | medium | Optionality. |
| Telemetry tagging (`riftgate.run.kind`) | weak | mixed | clean | clean | mixed | NFR-OBS06. |
| First-party vs ecosystem-aligned | n/a | unusual | **standard** | standard | ecosystem | Conventions matter. |

## 5. Foundational principles

**Deterministic replay against a WAL (ARIES lineage; CockroachDB; Datomic).** A WAL replay is well-trodden territory: read entries in order, reapply them against a system, observe deltas. The Riftgate twist is that the "system" is a freshly-constructed Riftgate driver loaded with a possibly-different configuration. The WAL entry carries enough state — request bytes, response bytes, timestamps, tenant — to drive the replay; the rest comes from the new config and the routing decisions that emerge from it. This is closer to Datomic's "the database is a value" philosophy than to a traditional database recovery replay.

**Eval-set evaluation patterns from LLM-eval tooling (Anthropic evals, OpenAI evals, LangSmith, Inspect AI).** The shape that has converged: a *task* (input + grader), an *eval set* (collection of tasks), a *runner* (executes the eval set against a target), a *report* (per-task pass/fail + aggregate statistics). The Riftgate `eval` subcommand follows this shape, with the WAL as the eval-set source (or an explicit TOML file) and the gateway as the runner.

**Streaming sketches for replay-time aggregation (HLL, CMS, reservoir sampling).** A replay over hours of recorded traffic produces millions of observations. We approximate cardinalities (`riftgate.replay.unique_tenants`) with HyperLogLog, heavy-hitter sets (top-K models, top-K routes) with Count-Min Sketch, and bounded random traces (`riftgate.replay.sample_traces`) with reservoir sampling. These are the same primitives the v0.4 token-level metrics LLD ([`docs/04-design/lld-observability.md`](../04-design/lld-observability.md)) calls out; the v0.3 replay tool gets to dogfood them.

**Tools should be operator-debuggable from a shell.** The principle behind `pg_dump`, `kafka-console-consumer`, `etcdctl`, `redis-cli`. Riftgate's operational surface — config validation, replay, eval — should compose with `jq`, `grep`, and shell pipelines. The CLI binary is the artifact that makes this possible; the library alone does not.

**Compose, do not replace, the WAL as the corpus.** The WAL format ([ADR `0013`](../06-adrs/0013-append-only-file-wal.md)) is already the source of truth for recorded traffic. An eval-set is a *filter and grader* over WAL segments — not a new corpus format. This avoids the trap of dual-source-of-truth that complicates many eval ecosystems.

## 6. Recommendation

**`v0.3` ships `riftgate-replay` as a new binary target in `crates/riftgate-replay`, with three subcommands (`dump`, `replay`, `eval`), each emitting structured JSON to stdout by default and optionally exporting to OTel with `riftgate.run.kind = "replay"`. The library API remains. An embedded HTTP endpoint is deferred to v0.4+ as an optional addition; Python bindings are explicitly rejected for this milestone.**

Concretely:

1. **Cargo manifest in `crates/riftgate-replay`:**

   ```toml
   [[bin]]
   name = "riftgate-replay"
   path = "src/bin/riftgate-replay.rs"

   [dependencies]
   clap = { workspace = true, features = ["derive"] }
   serde = { workspace = true, features = ["derive"] }
   serde_json = { workspace = true }
   riftgate-config = { workspace = true }
   riftgate-core = { workspace = true }
   riftgate-router = { workspace = true }
   riftgate-obs = { workspace = true }
   ```

   The binary depends on the same workspace crates the gateway uses; replay is a thin in-process driver.

2. **Subcommand contract:**

   ```text
   riftgate-replay dump
       --segments <PATH> ...        # one or more WAL segments
       [--from <TIMESTAMP>]
       [--to   <TIMESTAMP>]
       [--filter <FIELD>=<VALUE>]   # repeatable; AND-composed
       [--format json|jsonl|csv]    # default jsonl

   riftgate-replay replay
       --segments <PATH> ...
       --config <CONFIG_PATH>       # new config to test
       [--compare-against-recorded] # diff replayed responses vs recorded
       [--rate-multiplier <FLOAT>]  # 1.0 = real-time; 0.0 = as fast as possible
       [--otel-export]              # opt-in OTel emission

   riftgate-replay eval
       --segments <PATH> ...
       --eval-set <EVAL_PATH>       # TOML file with task graders
       [--config <CONFIG_PATH>]     # if absent, evaluate recorded responses directly
       [--fail-fast]
       [--exit-code-on-fail <INT>]  # default 1
   ```

3. **Eval-set TOML schema:**

   ```toml
   # evals/openai-chat-completions.toml
   [[task]]
   name        = "responses_are_valid_json"
   selector    = { route = "/v1/chat/completions" }
   grader.kind = "schema"
   grader.schema_path = "schemas/openai-chat.json"
   pass_threshold = 0.99

   [[task]]
   name        = "no_pii_leaks"
   selector    = { route = "/v1/chat/completions" }
   grader.kind = "regex_absence"
   grader.patterns = ["\\b\\d{3}-\\d{2}-\\d{4}\\b"]
   pass_threshold = 1.0

   [[task]]
   name        = "p99_latency_within_budget"
   selector    = { route = "*" }
   grader.kind = "aggregate"
   grader.metric = "ttfb_ms_p99"
   grader.threshold_max = 1500
   ```

4. **Report shape (stdout, default JSON):**

   ```json
   {
     "run_id": "01J5XKZF8M9JKWZ3Y0V7H2QA4P",
     "kind": "eval",
     "started_at": "2026-05-25T14:33:21Z",
     "duration_ms": 8421,
     "segments_scanned": 12,
     "requests_observed": 187432,
     "tasks": [
       { "name": "responses_are_valid_json", "passes": 187001, "fails": 431, "pass_rate": 0.99770, "result": "pass" },
       { "name": "no_pii_leaks",             "passes": 187432, "fails":   0, "pass_rate": 1.00000, "result": "pass" },
       { "name": "p99_latency_within_budget","aggregate": 1340, "threshold": 1500, "result": "pass" }
     ],
     "exit_code": 0
   }
   ```

5. **No live-gateway coupling.** The binary opens WAL segments by file path; it does not connect to a running Riftgate. The `replay` subcommand constructs a Riftgate driver in-process — same `crates/riftgate-router`, `crates/riftgate-config`, `crates/riftgate-filter` (when filters are exercised) — but it does *not* open a network socket. Upstream backends called during `replay` are configurable: real backends (`upstream = "real"`) for true replay against a new config, or recorded-response playback (`upstream = "recorded"`) for evaluating the gateway logic in isolation. Default: `upstream = "recorded"`.

6. **Telemetry hygiene.** All replay-emitted events carry `riftgate.run.kind = "replay" | "eval" | "dump"` and `riftgate.run.id = <ULID>`. The default sink is `JsonStdoutSink` (since the binary is a CLI). Opt-in OTel export via `--otel-export` is filtered to a separate OTLP destination from the live binary's, preventing observability-stream contamination.

7. **Exit codes.** Zero on success. Non-zero on any task fail (`eval`), any decoding error (`dump`), any divergence beyond a configurable threshold (`replay`). CI can wrap `riftgate-replay eval` and gate deployments on the exit code.

8. **No admin auth in v0.3.** The CLI reads local files; the auth surface is filesystem permissions. When the embedded HTTP endpoint lands in a later milestone, it inherits whatever admin-auth surface arrives with [Options `017` multitenancy](README.md).

### Conditions under which we'd revisit

- If operator demand for a programmatic-but-non-Rust API materialises, we revisit Python bindings — but only via a separate crate (`crates/riftgate-replay-py`) gated behind a `py` feature, so the default workspace builds unchanged.
- If operators ask for a Web UI on replay reports, we land the embedded HTTP endpoint as the v0.4+ companion to the CLI. The library API makes this trivial; the CLI continues to be the load-bearing surface.
- If the eval-set TOML schema runs out of expressiveness (e.g. needs custom graders not expressible declaratively), we add a WASM grader hook that reuses the [Options `016`](016-extension-mechanism.md) extension surface — same WIT ABI shape, different host functions.

## 7. What we explicitly reject

- **Library-only (status quo).** Fails NFR-OPS03; defeats the v0.3 narrative; loses the eval composition.
- **HTTP endpoint as the only surface.** Couples replay to the live gateway, contends for resources, requires admin auth we have not built.
- **Python bindings + Jupyter notebooks.** Violates the "no Python in the data path" disposition at the workspace-membership level; eval is not the data path but the dependency cost is wrong for v0.3.
- **A new eval-corpus format.** Would create a dual source of truth with the WAL. The WAL is the corpus.
- **Custom imperative grader in v0.3.** Schema, regex-absence, and aggregate-metric graders cover ≥ 90% of the v0.3 use cases. A WASM-grader hook is the future, not the present.
- **Cluster-distributed replay.** A multi-node replay engine is interesting but is v1.0+ scope at earliest.
- **Replay against a different *protocol* in v0.3.** We replay HTTP/1.1 SSE traffic as recorded; replay against HTTP/2 or gRPC upstream is gated on Options `008` evolution.

## 8. References

1. C. Mohan, Don Haderle, Bruce Lindsay, Hamid Pirahesh, Peter Schwarz, *ARIES: A Transaction Recovery Method Supporting Fine-Granularity Locking and Partial Rollbacks Using Write-Ahead Logging* (ACM TODS, 1992).
2. CockroachDB, *Change data capture and replay* documentation — <https://www.cockroachlabs.com/docs/stable/change-data-capture-overview.html>
3. Datomic, *The Database as a Value* (Rich Hickey) — <https://www.infoq.com/presentations/Datomic-Database-Value/>
4. Anthropic evals — <https://github.com/anthropics/evals>
5. OpenAI evals — <https://github.com/openai/evals>
6. LangSmith — <https://www.langchain.com/langsmith>
7. UK AI Safety Institute, *Inspect* (eval framework) — <https://inspect.ai-safety-institute.org.uk/>
8. Apache Kafka, *kafka-console-consumer / kafka-console-producer* — <https://kafka.apache.org/quickstart>
9. etcd, *etcdctl* — <https://etcd.io/docs/v3.5/dev-guide/interacting_v3/>
10. Philippe Flajolet et al., *HyperLogLog: the analysis of a near-optimal cardinality estimation algorithm* (AOFA 2007).
11. Graham Cormode, S. Muthukrishnan, *An Improved Data Stream Summary: The Count-Min Sketch and its Applications* (J. Algorithms, 2005).
12. Jeffrey S. Vitter, *Random Sampling with a Reservoir* (ACM TOMS, 1985).
13. `clap` (Rust CLI parser) — <https://docs.rs/clap/>
