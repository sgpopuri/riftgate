# NNN. <Decision name in plain English>

> **Status:** `proposed` | `recommended` | `accepted` (see ADR-NNNN) | `superseded by NNN`
> **Source-systems chapter(s):** `chXX_*`, `chYY_*`
> **Related options:** NNN, NNN
> **Related ADR:** [ADR NNNN](../06-adrs/NNNN-<slug>.md)

## 1. The decision in one sentence

> _The single decision this Options doc explores. Resist the temptation to make it two._

## 2. Context — what forces this decision

What in the system requires us to choose one of these candidates? Be specific. Reference the requirement IDs from [`../01-requirements/`](../01-requirements/) where applicable. Reference the LLD section from [`../04-design/`](../04-design/) that depends on this decision.

If this decision is being revisited (i.e. this Options doc supersedes a prior one), say what changed in the world to force the revisit.

## 3. Candidates

For each candidate, ≥3, ≤5:

### 3.1. <Candidate A>

**What it is.** A short paragraph someone unfamiliar with the candidate can read.

**Why it's interesting.** What problem it solves elegantly.

**Where it falls short.** Honest. No hand-waving.

**Real-world systems that use it.** With references where useful.

**Code or config sketch (optional).** Short — a real Options doc is a decision document, not a tutorial. Save the deep code for the implementation.

### 3.2. <Candidate B>
... (same structure)

### 3.3. <Candidate C>
... (same structure)

## 4. Tradeoff matrix

| Property | <A> | <B> | <C> | Why it matters |
|----------|-----|-----|-----|----------------|
| <criterion 1> | … | … | … | … |
| <criterion 2> | … | … | … | … |
| <criterion 3> | … | … | … | … |

Criteria should be specific and Riftgate-aware (e.g. "compatibility with our trait surface" rather than "ease of use"). 5-10 criteria is the typical range.

## 5. What the source-systems chapters say

Direct citations, by plain-text chapter title and number, to the source-systems references that informed this decision. Quote distinctive passages where useful. The source-systems curriculum is not a public sibling repo, so do not link to it — cite by title (e.g. `Ch3 (io_uring)`, `Ch11 (WAL and recovery)`) and let the prose carry the argument.

## 6. Recommendation

The Options doc *recommends* a choice. The ADR *makes* the choice. They can be the same person, but the documents have different jobs.

The recommendation includes:
- The chosen candidate(s).
- The conditions under which we'd revisit.
- The non-default candidates we keep available behind feature flags or as alternative impls.

## 7. What we explicitly reject

For each rejected candidate, one sentence: why we're saying no, and what it would take for us to reconsider. This protects future contributors from re-litigating settled questions and gives them a clear path to reopen if the world changes.

## 8. References

Books, papers, blog posts, source code, kernel commits — anything cited or worth following up. Use a numbered list; the prose can reference `[1]`, `[2]`, etc.

---

**A note on style.** Options docs are written for [Persona P3 (Maya, the systems-engineering learner)](../01-requirements/personas.md). She is smart but does not have the Riftgate context yet. Write so that someone landing on this Options doc cold can follow the decision tree without prior knowledge of the project.
