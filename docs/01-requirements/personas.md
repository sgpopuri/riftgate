# 01.c Personas

> Who Riftgate is for, who it is not for, and what each user expects to find here. We design and document for these specific people; we do not design for "everyone."

---

## Primary personas

### P1 — Pia, the platform engineer

**Context.** Pia works on the platform team at a 200-engineer SaaS company that ships an AI-powered product. The product talks to OpenAI in production today. Leadership wants the option to swap to a self-hosted model on internal infrastructure for cost or compliance reasons. Pia's job is to build the abstraction layer that makes that swap possible without breaking her product engineers.

**What she needs from Riftgate:**
- A pluggable Rust binary she can deploy as a sidecar, run in Kubernetes, or embed in her own container.
- The ability to add custom routing logic (e.g. "route all PII-tagged prompts to the on-prem model") without forking the upstream project.
- Clear documentation of what each subsystem does, so she can reason about failure modes when she is paged at 3am.
- Honest performance numbers so her capacity planning is real.

**What she does not need:**
- A SaaS dashboard. She has her own stack.
- A long list of supported model providers — she only uses two.
- Anyone telling her how to build her product.

**Pia's first 30 minutes in this repo:** reads [`README.md`](../../README.md), then [`00-vision.md`](../00-vision.md), then [`02-mvp-roadmap.md`](02-mvp-roadmap.md). Decides whether to bookmark or close the tab.

---

### P2 — Rohan, the inference SRE

**Context.** Rohan runs the GPU fleet at a model-serving company. He is on call. His current pain is that "P99 is high" tickets land in his queue with no signal about whether the GPU is saturated, the network is congested, the backend is OOM-thrashing, or the gateway is the problem. He wants kernel-level visibility into the gateway and its dependencies in one tool.

**What he needs from Riftgate:**
- Gateway-internal eBPF observability that surfaces CPU on/off-time, syscall stalls, GPU pressure correlation.
- Token-level SLO metrics (TTFT, inter-token latency) — not just request latency.
- A replay log so he can reproduce a bad request after the fact, locally, against a known-good backend.
- Adaptive backpressure that does the right thing under overload without his intervention.

**What he does not need:**
- More dashboards. He already has Grafana.
- Documentation that hand-waves "for high performance, use io_uring." He needs the failure modes.

**Rohan's first 30 minutes in this repo:** scans [`docs/05-options/014-ebpf-integration.md`](../05-options/014-ebpf-integration.md) (when it exists), then [`docs/04-design/lld-observability.md`](../04-design/lld-observability.md). Decides whether the project understands his problem.

---

### P3 — Maya, the systems-engineering learner

**Context.** Maya is a senior backend engineer who knows web frameworks well but has never written a network server from scratch. She has read the buzzwords (epoll, io_uring, work-stealing) and wants to *understand* them. She has tried and bounced off the Linux kernel source. She is looking for a real codebase whose every design decision is documented in plain English.

**What she needs from Riftgate:**
- The Options docs in [`docs/05-options/`](../05-options/), each one explaining a design space, the candidates, and the tradeoffs in language she can read.
- The corresponding ADRs in [`docs/06-adrs/`](../06-adrs/) that show how a decision is actually made.
- A codebase small enough to read end-to-end on a weekend.

**What she does not need:**
- A 10,000-line monolith with no design rationale.
- Marketing claims.
- An entry barrier higher than "knows Rust well enough to read."

**Maya's first 30 minutes in this repo:** reads [`README.md`](../../README.md), then jumps to [`docs/05-options/001-io-model.md`](../05-options/001-io-model.md). If she finds it absorbing, she stays.

---

### P4 — Devansh, the contributor

**Context.** Devansh has built two of his own LLM proxies as side projects. They both calcified. He sees Riftgate, reads the trait surface, and realizes he can plug in his prefix-routing strategy without rewriting the gateway. He wants to upstream a custom `Router` impl.

**What he needs from Riftgate:**
- A clean trait surface in `riftgate-core` he can implement against.
- A [`CONTRIBUTING.md`](../../CONTRIBUTING.md) that makes the contribution path obvious.
- An [`AGENTS.md`](../../AGENTS.md) that tells his agent assistant what to read before writing code.
- A test harness that lets him verify his strategy against a real backend without setting up a whole production cluster.

**What he does not need:**
- A bureaucratic review process.
- A tightly-coupled monolith that resists extension.
- Vague hints about the "right way" to do things — he needs the conventions in writing.

**Devansh's first contribution attempt:** opens an issue describing his routing strategy, gets a response within a week, opens a PR with a new file in `crates/riftgate-router/strategies/`, gets it reviewed, lands within two weeks. If this loop is slower than that, he stops contributing.

---

## Secondary personas (we serve them but do not optimize for them)

### S1 — Ada, the application developer
She wants a SaaS multi-provider router so her app code can call "an LLM" without caring which one. **Not our user.** Ada should use [LiteLLM](https://github.com/BerriAI/litellm), [TensorZero](https://www.tensorzero.com/), or [Helicone](https://www.helicone.ai/). We will not be a better fit for her than they are.

### S2 — Karim, the researcher
He wants to plug a novel routing algorithm into a real gateway and benchmark it against vanilla. **Welcome.** Riftgate's plugin model serves him well, but his needs are not load-bearing for the project's roadmap.

### S3 — Sora, the conference-talk audience member
She watches the Riftgate talk at QCon and wants to know whether to mention it to her team. **Welcome.** The narrative docs in this repo are for her too — but if she has a deployment question, she is in P1's territory and we serve her there.

---

## Anti-personas (explicitly NOT our users)

- **Anyone who needs production support.** Riftgate is OSS. There is no SLA, no support contract, no dedicated team. If you cannot run it without a vendor relationship, you should not run it yet.
- **Anyone who wants a black-box product.** Riftgate is a teaching artifact and a framework. If you do not want to read its code, it is the wrong tool.
- **Anyone whose only criterion is raw P99 leadership.** TensorZero is your project, not ours.

---

## How we use these personas

- Every PR description should be able to answer: *which persona does this serve and how?*
- Every Options doc should be readable by **P3 (Maya)** without a glossary lookup on every paragraph.
- Every narrative doc derived from this work should make sense to **S3 (Sora)** while remaining accurate enough for **P1 (Pia)** to cite in an internal RFC.
- Every feature request that does not serve any of P1-P4 is a candidate for "won't fix" with a respectful explanation.
